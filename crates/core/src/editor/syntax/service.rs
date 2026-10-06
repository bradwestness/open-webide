//! One preparation service used behind synchronous and worker transport adapters.
use super::{MAX_ANALYSIS_MESSAGE_BYTES, SyntaxAnalysisData, SyntaxPreparations, SyntaxStatus};
use crate::{editor::MAX_STRUCTURE_BYTES, highlight::Language};

pub const SYNTAX_PROTOCOL_VERSION: u32 = 1;
pub const MAX_SYNTAX_REQUEST_BYTES: usize = MAX_STRUCTURE_BYTES * 6 + 8192;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxRequest {
    pub version: u32,
    pub ticket: u32,
    pub document: String,
    pub language: Language,
    pub source: String,
    pub tab_width: usize,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxReply {
    pub version: u32,
    pub ticket: u32,
    pub status: SyntaxStatus,
    pub analysis: Option<SyntaxAnalysisData>,
}

impl SyntaxPreparations<String> {
    /// Malformed envelopes are transport failures; valid oversized sources receive
    /// an explicit fallback status. Limits and shaping are shared above adapters.
    pub fn handle_message(
        &mut self,
        message: &str,
        should_continue: impl FnMut() -> bool,
    ) -> Option<String> {
        if message.len() > MAX_SYNTAX_REQUEST_BYTES {
            return None;
        }
        let request: SyntaxRequest = serde_json::from_str(message).ok()?;
        if request.version != SYNTAX_PROTOCOL_VERSION
            || request.document.len() > 4096
            || request.document.is_empty()
            || !(1..=16).contains(&request.tab_width)
        {
            return None;
        }
        let (mut status, analysis) = self.prepare(
            request.document,
            request.language,
            &request.source,
            request.tab_width,
            should_continue,
        );
        let analysis = analysis.and_then(|value| value.transfer_data());
        if matches!(status, SyntaxStatus::Ready { .. }) && analysis.is_none() {
            status = SyntaxStatus::TooLarge;
        }
        let reply = SyntaxReply {
            version: SYNTAX_PROTOCOL_VERSION,
            ticket: request.ticket,
            status,
            analysis,
        };
        let message = serde_json::to_string(&reply).ok()?;
        (message.len() <= MAX_ANALYSIS_MESSAGE_BYTES).then_some(message)
    }
}

impl SyntaxReply {
    pub fn receive(
        message: &str,
        ticket: u32,
        source: &str,
    ) -> Option<(SyntaxStatus, Option<std::sync::Arc<super::SyntaxAnalysis>>)> {
        if message.len() > MAX_ANALYSIS_MESSAGE_BYTES {
            return None;
        }
        let reply: Self = serde_json::from_str(message).ok()?;
        if reply.version != SYNTAX_PROTOCOL_VERSION || reply.ticket != ticket {
            return None;
        }
        let analysis = match (reply.status, reply.analysis) {
            (SyntaxStatus::Ready { .. }, Some(data)) => Some(data.validate(source)?),
            (SyntaxStatus::Cancelled | SyntaxStatus::TooLarge, None) => None,
            _ => return None,
        };
        Some((reply.status, analysis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(source: &str) -> SyntaxRequest {
        SyntaxRequest {
            version: SYNTAX_PROTOCOL_VERSION,
            ticket: 42,
            document: "1:main.rs".into(),
            language: Language::Rust,
            source: source.into(),
            tab_width: 4,
        }
    }
    #[test]
    fn message_contract_matches_direct_preparation_and_rejects_wrong_tickets_and_sources() {
        let source = "fn main() {\r\n call(\"文😀\");\r\n}";
        let mut direct = SyntaxPreparations::default();
        let (_, expected) =
            direct.prepare("1:main.rs".to_string(), Language::Rust, source, 4, || true);
        let mut worker = SyntaxPreparations::default();
        let input = serde_json::to_string(&request(source)).unwrap();
        let output = worker.handle_message(&input, || true).unwrap();
        let (status, result) = SyntaxReply::receive(&output, 42, source).unwrap();
        assert_eq!(status, SyntaxStatus::Ready { incremental: false });
        assert_eq!(expected.unwrap().highlights(), result.unwrap().highlights());
        assert!(SyntaxReply::receive(&output, 43, source).is_none());
        assert!(SyntaxReply::receive(&output, 42, &source.replace("main", "xxxx")).is_none());
        let output = worker.handle_message(&input, || false).unwrap();
        assert!(matches!(
            SyntaxReply::receive(&output, 42, source),
            Some((SyntaxStatus::Cancelled, None))
        ));
        let output = worker
            .handle_message(
                &serde_json::to_string(&request(&"x".repeat(MAX_STRUCTURE_BYTES + 1))).unwrap(),
                || true,
            )
            .unwrap();
        assert!(matches!(
            SyntaxReply::receive(&output, 42, ""),
            Some((SyntaxStatus::TooLarge, None))
        ));
        let mut invalid = request(source);
        invalid.version += 1;
        assert!(
            worker
                .handle_message(&serde_json::to_string(&invalid).unwrap(), || true)
                .is_none()
        );
        assert!(worker.handle_message("{}", || true).is_none());
    }
}
