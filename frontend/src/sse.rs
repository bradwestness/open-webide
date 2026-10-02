use openwebide_core::RunEvent;

/// Parse the tagged run event in an SSE frame's data, ignoring the event line.
pub fn parse_frame(frame: &str) -> Option<RunEvent> {
    serde_json::from_str(&frame_data(frame)).ok()
}

fn frame_data(frame: &str) -> String {
    frame
        .split(['\r', '\n'])
        .filter_map(|line| {
            let value = line.strip_prefix("data:")?;
            Some(value.strip_prefix(' ').unwrap_or(value))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct FrameBuffer {
    line: String,
    frame: String,
    after_cr: bool,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            line: String::new(),
            frame: String::new(),
            after_cr: false,
        }
    }

    pub fn push(&mut self, data: &str) -> Vec<String> {
        let mut frames = Vec::new();
        for ch in data.chars() {
            let after_cr = std::mem::replace(&mut self.after_cr, ch == '\r');
            if ch == '\n' && after_cr {
                continue;
            }
            if matches!(ch, '\r' | '\n') {
                if self.line.is_empty() {
                    frames.push(std::mem::take(&mut self.frame));
                } else {
                    self.frame.push_str(&self.line);
                    self.frame.push('\n');
                    self.line.clear();
                }
            } else {
                self.line.push(ch);
            }
        }
        frames
    }
}

impl Default for FrameBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openwebide_core::{ChatMessage, Role, utf8::Utf8Decoder};

    #[test]
    fn backend_frames_parse_as_run_events() {
        let message = ChatMessage {
            id: 2,
            session_id: 1,
            role: Role::Assistant,
            content: "reply".into(),
            created_at: 0,
            tool_calls: None,
            tool_call_id: None,
            usage: None,
        };
        for (frame, expected) in [
            (
                "event: delta\ndata: {\"kind\":\"delta\",\"content\":\"hi\"}\n\n",
                RunEvent::Delta {
                    content: "hi".into(),
                },
            ),
            (
                "event: done\ndata: {\"kind\":\"done\",\"message\":{\"id\":2,\"session_id\":1,\"role\":\"assistant\",\"content\":\"reply\",\"created_at\":0}}\n\n",
                RunEvent::Done { message },
            ),
            (
                "event: error\ndata: {\"kind\":\"error\",\"message\":\"failed\"}\n\n",
                RunEvent::Error {
                    message: "failed".into(),
                },
            ),
        ] {
            assert_eq!(parse_frame(frame), Some(expected));
        }
    }

    #[test]
    fn tagged_data_determines_event_kind() {
        assert_eq!(
            parse_frame("event: ignored\ndata: {\"kind\":\"cancelled\"}\n\n"),
            Some(RunEvent::Cancelled),
        );
        assert_eq!(parse_frame("data: {\"kind\":\"unknown\"}\n\n"), None);
        assert_eq!(parse_frame("data: not json\n\n"), None);
    }

    #[test]
    fn every_byte_split_preserves_utf8_and_frame_boundaries() {
        let lf_wire = "event: delta\ndata:{\"kind\":\"delta\",\ndata: \"content\":\"café 🦀\"}\n\nevent: cancelled\ndata: {\"kind\":\"cancelled\"}\n\n";
        let expected = vec![
            RunEvent::Delta {
                content: "café 🦀".into(),
            },
            RunEvent::Cancelled,
        ];
        for delimiter in ["\n", "\r\n", "\r"] {
            let wire = lf_wire.replace('\n', delimiter);
            for split in 0..=wire.len() {
                let mut decoder = Utf8Decoder::new();
                let mut buffer = FrameBuffer::new();
                let mut events = Vec::new();
                for chunk in [&wire.as_bytes()[..split], &wire.as_bytes()[split..]] {
                    for frame in buffer.push(&decoder.push(chunk)) {
                        events.push(parse_frame(&frame).unwrap());
                    }
                }
                for frame in buffer.push(&decoder.finish()) {
                    events.push(parse_frame(&frame).unwrap());
                }
                assert_eq!(events, expected, "split at byte {split}");
            }
        }
    }
    #[test]
    fn data_fields_preserve_newlines_and_whitespace() {
        assert_eq!(
            frame_data(
                ": comment\ndata:  first  \ndata:second\ndata:\ndata: \nretry: 10\nid: ignored"
            ),
            " first  \nsecond\n\n"
        );
        for frame in [
            "",
            ": comment",
            "data:",
            "data: ",
            "data: not json",
            "data: {\"kind\":\"unknown\"}",
            "data: {\"kind\":\"delta\",\"content\":\"split\ndata: string\"}",
        ] {
            assert_eq!(parse_frame(frame), None);
        }
    }

    #[test]
    fn only_blank_line_terminated_frames_are_dispatched() {
        for delimiter in ["\n", "\r\n", "\r"] {
            let mut buffer = FrameBuffer::new();
            let wire = [
                ": comment",
                "",
                "",
                "data:{\"kind\":\"cancelled\"}",
                "",
                "data:{\"kind\":\"error\",\"message\":\"unfinished\"}",
                "",
            ]
            .join(delimiter);
            let events: Vec<_> = buffer
                .push(&wire)
                .iter()
                .filter_map(|frame| parse_frame(frame))
                .collect();
            assert_eq!(events, [RunEvent::Cancelled]);
            assert!(buffer.push("").is_empty());
        }
    }
}
