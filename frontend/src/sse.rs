use openwebide_core::RunEvent;

/// Parse the tagged run event in an SSE frame's data, ignoring the event line.
pub fn parse_frame(frame: &str) -> Option<RunEvent> {
    let mut data = String::new();
    for line in frame.lines() {
        if let Some(value) = line.strip_prefix("data: ") {
            data.push_str(value.trim());
        }
    }
    serde_json::from_str(&data).ok()
}

pub struct FrameBuffer {
    pending: String,
}

impl FrameBuffer {
    pub fn new() -> Self {
        Self {
            pending: String::new(),
        }
    }

    pub fn push(&mut self, data: &str) -> Vec<String> {
        self.pending.push_str(data);
        let mut frames = Vec::new();
        while let Some(pos) = self.pending.find("\n\n") {
            let frame = self.pending[..pos].to_string();
            self.pending.drain(..pos + 2);
            frames.push(frame);
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
        let wire = "event: delta\ndata: {\"kind\":\"delta\",\"content\":\"café 🦀\"}\n\nevent: cancelled\ndata: {\"kind\":\"cancelled\"}\n\n";
        let expected = vec![
            RunEvent::Delta {
                content: "café 🦀".into(),
            },
            RunEvent::Cancelled,
        ];
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
