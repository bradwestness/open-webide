//! Provider SSE field parsing, shared by the llama.cpp and Ollama stream
//! parsers.
//!
//! One `data:` line is one payload; multi-line `data` concatenation is not
//! supported (no local provider uses it).

/// What one line of a provider stream is.
pub(crate) enum SseField<'a> {
    /// A payload line: a `data: …` value or a bare JSON line.
    Data(&'a str),
    /// A provider error line: the value of an `error: …` field.
    Error(&'a str),
    /// A comment, an `event:`/`id:`/`retry:`/unknown field, or a line with
    /// no colon: ignore it.
    Ignore,
}

/// Classify one line of a provider stream.
///
/// A `:`-prefixed line is an SSE comment. A line whose trimmed form starts
/// with `{` is a bare JSON payload (lenient fallback). Otherwise the line is
/// split at the first `:` and one leading space is stripped from the value:
/// `data` is a payload, `error` a provider error, and `event`/`id`/`retry`/
/// unknown fields (or a line with no colon) are ignored.
pub(crate) fn sse_field(line: &str) -> SseField<'_> {
    if line.starts_with(':') {
        return SseField::Ignore;
    }
    if line.trim_start().starts_with('{') {
        return SseField::Data(line);
    }
    let Some((name, value)) = line.split_once(':') else {
        return SseField::Ignore;
    };
    let value = value.strip_prefix(' ').unwrap_or(value);
    match name {
        "data" => SseField::Data(value),
        "error" => SseField::Error(value),
        _ => SseField::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comment_line_is_ignored() {
        assert!(matches!(sse_field(": keep-alive"), SseField::Ignore));
    }

    #[test]
    fn data_field_with_space() {
        assert!(matches!(
            sse_field("data: {\"a\":1}"),
            SseField::Data(v) if v == "{\"a\":1}"
        ));
    }

    #[test]
    fn data_field_without_space() {
        assert!(matches!(
            sse_field("data:{\"a\":1}"),
            SseField::Data(v) if v == "{\"a\":1}"
        ));
    }

    #[test]
    fn bare_json_line_is_data() {
        assert!(matches!(
            sse_field("{\"a\":1}"),
            SseField::Data(v) if v == "{\"a\":1}"
        ));
    }

    #[test]
    fn error_field() {
        assert!(matches!(
            sse_field("error: {\"message\":\"boom\"}"),
            SseField::Error(v) if v == "{\"message\":\"boom\"}"
        ));
    }

    #[test]
    fn event_id_retry_and_unknown_fields_are_ignored() {
        assert!(matches!(sse_field("event: message"), SseField::Ignore));
        assert!(matches!(sse_field("id: 42"), SseField::Ignore));
        assert!(matches!(sse_field("retry: 3000"), SseField::Ignore));
        assert!(matches!(sse_field("foo: bar"), SseField::Ignore));
    }

    #[test]
    fn no_colon_is_ignored() {
        assert!(matches!(sse_field("plain line"), SseField::Ignore));
    }
}
