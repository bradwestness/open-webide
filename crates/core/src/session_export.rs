//! Portable Markdown export of persisted conversation, without filesystem reads.
use crate::{
    ChatSession, ConversationEntry, PromptContent, Role, ToolStep,
    tui::{extract_editor_context_prelude, parse_thinking},
    vfs::format_utc_timestamp,
};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionExport {
    pub filename: String,
    pub markdown: String,
}

pub fn session_markdown(session: &ChatSession, entries: &[ConversationEntry]) -> String {
    let mut output = format!(
        "# {}\n\nCreated: {}\n\n",
        inline(&session.name),
        format_utc_timestamp(session.created_at)
    );
    for entry in entries {
        match entry {
            ConversationEntry::Message(message) => {
                let _ = writeln!(
                    output,
                    "## {}\n\n{}\n",
                    message.role.as_str(),
                    format_utc_timestamp(message.created_at)
                );
                if message.role == Role::User {
                    let (context, text) = extract_editor_context_prelude(&message.content);
                    let prompt = PromptContent::decode(text);
                    output.push_str(&prompt.text);
                    output.push_str("\n\n");
                    if let Some(context) = context {
                        output.push_str("### Editor context\n\n");
                        fenced(&mut output, context, "text");
                    }
                    for reference in prompt.references {
                        let _ = writeln!(
                            output,
                            "### Reference: {}\n",
                            inline(&reference.mention.path)
                        );
                        fenced(&mut output, &reference.content, "text");
                    }
                    for image in prompt.images {
                        let _ = writeln!(output, "![{}]({})\n", inline(&image.name), image.url());
                    }
                } else if message.role == Role::Assistant {
                    let parsed = parse_thinking(&message.content);
                    if let Some(thinking) = parsed.thinking {
                        output.push_str("### Reasoning\n\n");
                        fenced(&mut output, &thinking, "text");
                    }
                    output.push_str(&parsed.answer);
                    output.push_str("\n\n");
                } else {
                    fenced(&mut output, &message.content, "text");
                }
                if let Some(calls) = &message.tool_calls {
                    output.push_str("### Tool calls\n\n");
                    fenced(
                        &mut output,
                        &serde_json::to_string_pretty(calls)
                            .expect("String-only tool calls serialize"),
                        "json",
                    );
                }
            }
            ConversationEntry::ToolStep(step) => tool_step(&mut output, step),
        }
    }
    output
}
pub fn session_markdown_filename(session: &ChatSession) -> String {
    let name: String = session
        .name
        .chars()
        .take(80)
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        format!("session-{}.md", session.id)
    } else {
        format!("{name}.md")
    }
}
fn tool_step(output: &mut String, step: &ToolStep) {
    let _ = writeln!(
        output,
        "## Tool: {}\n\nStatus: {}\n",
        inline(&step.name),
        match step.ok {
            Some(true) => "Succeeded",
            Some(false) => "Failed",
            None if step.timing.is_some_and(|timing| timing.finished) => "Stopped",
            None => "Incomplete",
        }
    );
    if let Some(timing) = step.timing.filter(|timing| timing.finished) {
        let _ = writeln!(
            output,
            "Duration: {}.{}s\n",
            timing.elapsed_ms / 1000,
            (timing.elapsed_ms % 1000) / 100
        );
    }
    output.push_str("### Request\n\n");
    fenced(output, &step.summary, "text");
    if let Some(result) = &step.result_summary {
        output.push_str("### Result\n\n");
        fenced(output, result, "text");
    }
    if let Some(diff) = &step.diff {
        let _ = writeln!(output, "### File: {}\n", inline(&diff.path));
        output.push_str("#### Before\n\n");
        if diff.old_unavailable {
            output.push_str("Previous contents unavailable.\n\n");
        } else if let Some(old) = &diff.old {
            fenced(output, old, "text");
        } else {
            output.push_str("File did not exist.\n\n");
        }
        output.push_str("#### After\n\n");
        fenced(output, &diff.new, "text");
    }
}
fn inline(text: &str) -> String {
    let mut output = String::new();
    for ch in text.chars() {
        if ch.is_control() {
            output.push(' ');
        } else {
            if "\\`*_[]<>#!|()".contains(ch) {
                output.push('\\');
            }
            output.push(ch);
        }
    }
    output
}
fn fenced(output: &mut String, text: &str, language: &str) {
    let mut longest = 0;
    let mut count = 0;
    for ch in text.chars() {
        if ch == '`' {
            count += 1;
            longest = longest.max(count);
        } else {
            count = 0;
        }
    }
    let fence = "`".repeat(3.max(longest + 1));
    let _ = writeln!(output, "{fence}{language}");
    output.push_str(text);
    if !text.ends_with('\n') {
        output.push('\n');
    }
    let _ = writeln!(output, "{fence}\n");
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ChatMessage, FileDiff, ToolCall, ToolTiming,
        prompt::{Mention, MentionKind, PromptImage, PromptReference},
    };
    #[test]
    fn export_preserves_messages_attachments_reasoning_and_arbitrary_tool_fences() {
        let session:ChatSession=serde_json::from_value(serde_json::json!({"id":1,"name":"../../[session]\n# forged","connection_id":null,"created_at":0})).unwrap();
        let prompt = PromptContent {
            text: "Update packages".into(),
            references: vec![PromptReference {
                mention: Mention {
                    kind: MentionKind::File,
                    path: "sample.csproj".into(),
                },
                content: "```xml\n<Project/>\n```".into(),
            }],
            images: vec![
                PromptImage::from_bytes("[image].png".into(), b"\x89PNG\r\n\x1a\n").unwrap(),
            ],
        };
        let message = |id, role, content| {
            ConversationEntry::Message(ChatMessage {
                id,
                session_id: 1,
                role,
                content,
                created_at: 0,
                tool_calls: None,
                tool_call_id: None,
                usage: None,
            })
        };
        let tool = ToolStep {
            timing: Some(ToolTiming::start(1000).sample(2400, true)),
            tool_call_id: "t".into(),
            name: "run_command".into(),
            summary: "dotnet restore".into(),
            ok: Some(false),
            result_summary: Some("`````\nraw <output>\n`````".into()),
            diff: Some(FileDiff {
                path: "sample.csproj".into(),
                old: Some("old".into()),
                new: "new".into(),
                old_unavailable: false,
                backup_path: None,
            }),
            anchor_message_id: 1,
            checkpoint: None,
        };
        let call = ConversationEntry::Message(ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: "<think>reasoning</think>\nAnswer".into(),
            created_at: 0,
            tool_calls: Some(vec![ToolCall {
                id: "t".into(),
                name: "run_command".into(),
                arguments: "{}".into(),
            }]),
            tool_call_id: None,
            usage: None,
        });
        let text = session_markdown(
            &session,
            &[
                message(1, Role::User, prompt.encode().unwrap()),
                call,
                ConversationEntry::ToolStep(tool),
                message(3, Role::System, "stored system info".into()),
            ],
        );
        assert!(text.contains("## user\n"));
        assert!(text.contains("Update packages"));
        assert!(text.contains("### Reference: sample.csproj\n\n````text\n```xml"));
        assert!(text.contains("data:image/png;base64,"));
        assert!(text.contains("### Reasoning\n\n```text\nreasoning\n```\n\nAnswer"));
        assert!(text.contains("### Tool calls"));
        assert!(text.contains("Status: Failed"));
        assert!(text.contains("Duration: 1.4s"));
        assert!(text.contains("``````text\n`````\nraw <output>\n`````\n``````"));
        assert!(text.contains("#### Before\n\n```text\nold"));
        assert!(text.contains("#### After\n\n```text\nnew"));
        assert!(text.contains("## system\n"));
        assert!(!session_markdown_filename(&session).contains('/'));
    }
}
