use openwebide_core::tui::{EditorContext, SelectionContext};

pub fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Convert a UTF-16 code-unit offset into a byte offset into `s`.
///
/// Returns the byte index of the first char whose UTF-16 end exceeds `u16_off`; if the offset
/// falls inside a surrogate pair (only possible for offsets past the low surrogate of a
/// multi-unit char), it snaps back to the start of that char. Offsets past the end return
/// `s.len()`.
pub fn utf16_to_byte(s: &str, u16_off: usize) -> usize {
    let mut u16_count = 0usize;
    for (byte_idx, ch) in s.char_indices() {
        let next_count = u16_count + ch.len_utf16();
        if next_count > u16_off {
            return byte_idx;
        }
        u16_count = next_count;
    }
    s.len()
}

/// Build an `EditorContext` from raw editor content and UTF-16 selection offsets, snapping
/// offsets to byte indices and truncating any selection text to a safe UTF-8 boundary.
pub fn editor_context(
    file_path: String,
    content: &str,
    sel_start_u16: usize,
    sel_end_u16: usize,
) -> EditorContext {
    let sel_start = utf16_to_byte(content, sel_start_u16);
    let sel_end = utf16_to_byte(content, sel_end_u16);

    let safe_start = sel_start.min(content.len());
    let text_before_start = &content[..safe_start];
    let start_line = text_before_start.matches('\n').count() + 1;
    let last_newline = text_before_start.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let cursor_col = text_before_start[last_newline..].chars().count() + 1;

    let selection = if sel_end > sel_start {
        let safe_end = sel_end.min(content.len());
        let text_before_end = &content[..safe_end];
        let end_line = text_before_end.matches('\n').count() + 1;
        let raw_selection = &content[safe_start..safe_end];
        let line_count = raw_selection.lines().count();

        let final_text = if line_count > 100 || raw_selection.len() > 8192 {
            let mut truncated = raw_selection
                .lines()
                .take(100)
                .collect::<Vec<_>>()
                .join("\n");
            let cut = truncated.floor_char_boundary(8000);
            truncated.truncate(cut);
            truncated.push_str("\n... [truncated: selection exceeds 100 lines / 8KB; use read_file with line offsets]");
            truncated
        } else {
            raw_selection.to_string()
        };

        Some(SelectionContext {
            start_line,
            end_line,
            text: final_text,
        })
    } else {
        None
    };

    EditorContext {
        file_path,
        cursor_line: start_line,
        cursor_col,
        selection,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_editor_context_basic() {
        let content = "// café\nfn main() {}\n";
        let ctx = editor_context("f.rs".to_string(), content, 7, 7);
        assert_eq!(ctx.cursor_line, 1);
        assert_eq!(ctx.cursor_col, 8);
        assert!(ctx.selection.is_none());
    }

    #[test]
    fn test_editor_context_emoji_before_cursor() {
        let content = "a😀b\nfn main() {}\n";
        // "a😀b" is 1 + 2 (surrogate pair) + 1 = 4 UTF-16 units; cursor after 'b'
        let ctx = editor_context("f.rs".to_string(), content, 4, 4);
        assert_eq!(ctx.cursor_line, 1);
        assert_eq!(ctx.cursor_col, 4);
    }

    #[test]
    fn test_utf16_to_byte_inside_surrogate_pair_snaps_back() {
        let content = "a😀b";
        // 'a' = 1 u16, '😀' = 2 u16 (offsets 1..3), 'b' = 1 u16 (offset 3)
        let emoji_start_byte = utf16_to_byte(content, 1);
        // offset 2 lands inside the surrogate pair; should snap back to emoji start
        let inside_surrogate = utf16_to_byte(content, 2);
        assert_eq!(inside_surrogate, emoji_start_byte);
        assert!(content.is_char_boundary(inside_surrogate));
    }

    #[test]
    fn test_offsets_past_end_clamp() {
        let content = "hello";
        let byte = utf16_to_byte(content, 1000);
        assert_eq!(byte, content.len());

        let ctx = editor_context("f.rs".to_string(), content, 0, 1000);
        assert_eq!(ctx.selection.unwrap().text, "hello".to_string());
    }

    #[test]
    fn test_multibyte_selection_returns_exact_substring() {
        let content = "café latte";
        let start = content.find('l').unwrap();
        let start_u16 = content[..start].encode_utf16().count();
        let end_u16 = content.encode_utf16().count();
        let ctx = editor_context("f.rs".to_string(), content, start_u16, end_u16);
        assert_eq!(ctx.selection.unwrap().text, "latte".to_string());
    }

    #[test]
    fn test_8000_byte_cut_inside_multibyte_char_stays_valid_utf8() {
        // Build a selection > 8192 bytes where byte offset 8000 lands mid-character:
        // 3999 * 2 bytes = 7998 bytes, plus a 1-byte 'x' = 7999 bytes, then a 2-byte
        // 'é' spanning bytes 7999..8001, so byte 8000 falls inside that 'é'.
        let mut content = "é".repeat(3999);
        content.push('x');
        content.push('é');
        content.push_str(&"y".repeat(200));
        let end_u16 = content.encode_utf16().count();
        let ctx = editor_context("f.rs".to_string(), &content, 0, end_u16);
        let text = ctx.selection.unwrap().text;
        assert!(std::str::from_utf8(text.as_bytes()).is_ok());
        let truncated_body = text.split("\n... [truncated").next().unwrap();
        assert!(truncated_body.len() < 8000);
        assert!(truncated_body.ends_with('x'));
    }

    #[test]
    fn test_utf16_to_byte_round_trips_at_char_boundaries() {
        let s = "a😀café/b€c";
        let mut u16_count = 0usize;
        for (byte_idx, ch) in s.char_indices() {
            assert_eq!(utf16_to_byte(s, u16_count), byte_idx);
            u16_count += ch.len_utf16();
        }
        assert_eq!(utf16_to_byte(s, u16_count), s.len());

        for (byte_idx, _) in s.char_indices() {
            let u16_off = s[..byte_idx].encode_utf16().count();
            let mapped_byte = utf16_to_byte(s, u16_off);
            assert_eq!(s[..mapped_byte].encode_utf16().count(), u16_off);
        }
    }
}
