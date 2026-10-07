//! One preparation service used behind synchronous and worker transport adapters.
use super::{
    MAX_ANALYSIS_MESSAGE_BYTES, SyntaxAnalysisData, SyntaxPreparations, SyntaxSource, SyntaxStatus,
};
use crate::{editor::MAX_STRUCTURE_BYTES, highlight::Language};

pub const SYNTAX_PROTOCOL_VERSION: u32 = 5;
pub const MAX_SYNTAX_REQUEST_BYTES: usize = MAX_STRUCTURE_BYTES * 6 + 8192;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxRequest {
    pub version: u32,
    pub ticket: u32,
    pub document: String,
    pub language: Language,
    pub source: SyntaxSource,
    pub tab_width: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_ticket: Option<u32>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyntaxReply {
    pub version: u32,
    pub ticket: u32,
    pub status: SyntaxStatus,
    pub analysis: Option<SyntaxAnalysisData>,
}

impl SyntaxRequest {
    pub fn new(
        ticket: u32,
        document: String,
        language: Language,
        source: &str,
        tab_width: usize,
        previous: Option<(u32, &super::SyntaxAnalysis)>,
    ) -> Self {
        Self {
            version: SYNTAX_PROTOCOL_VERSION,
            ticket,
            document,
            language,
            tab_width,
            source: SyntaxSource::publication(source, previous.map(|(_, value)| value.source())),
            base_ticket: previous.map(|(ticket, _)| ticket),
        }
    }
}

impl SyntaxPreparations<String> {
    /// Malformed envelopes are transport failures; valid oversized sources receive
    /// an explicit fallback status. Limits and shaping are shared above adapters.
    pub fn handle_message(
        &mut self,
        message: &str,
        mut should_continue: impl FnMut() -> bool,
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
        let previous = self
            .previous_publication(&request.document)
            .filter(|(ticket, _)| request.base_ticket == Some(*ticket));
        let control_reply = |status| {
            serde_json::to_string(&SyntaxReply {
                version: SYNTAX_PROTOCOL_VERSION,
                ticket: request.ticket,
                status,
                analysis: None,
            })
            .ok()
        };
        if matches!(request.source, SyntaxSource::Replace { .. }) {
            // A delta without an advertised base is malformed, rather than a resync.
            request.base_ticket?;
            if previous.is_none() {
                return control_reply(SyntaxStatus::NeedsSource);
            }
        }
        let base_source = previous.as_ref().map(|(_, analysis)| analysis.source());
        let length = request.source.result_length(base_source)?;
        if length > MAX_STRUCTURE_BYTES {
            self.remove(&request.document);
            return control_reply(SyntaxStatus::TooLarge);
        }
        if !should_continue() {
            self.remove(&request.document);
            return control_reply(SyntaxStatus::Cancelled);
        }
        let source = request.source.resolve(base_source)?;
        let (mut status, prepared) = self.prepare(
            request.document.clone(),
            request.language,
            &source,
            request.tab_width,
            should_continue,
        );
        let analysis = prepared.as_ref().and_then(|value| {
            value
                .transfer_data_reusing(
                    previous
                        .as_ref()
                        .map(|(ticket, analysis)| (*ticket, analysis.as_ref())),
                )
                .or_else(|| value.transfer_data())
        });
        if matches!(status, SyntaxStatus::Ready { .. }) && analysis.is_none() {
            status = SyntaxStatus::TooLarge;
        }
        let mut reply = SyntaxReply {
            version: SYNTAX_PROTOCOL_VERSION,
            ticket: request.ticket,
            status,
            analysis,
        };
        let mut message = serde_json::to_string(&reply).ok()?;
        if message.len() > MAX_ANALYSIS_MESSAGE_BYTES {
            reply.analysis = prepared.as_ref()?.transfer_data();
            if reply.analysis.is_none() {
                reply.status = SyntaxStatus::TooLarge;
            }
            message = serde_json::to_string(&reply).ok()?;
            if message.len() > MAX_ANALYSIS_MESSAGE_BYTES {
                return None;
            }
        }
        if reply.analysis.is_some()
            && let Some(prepared) = prepared
        {
            self.remember_publication(&request.document, request.ticket, prepared);
        }
        Some(message)
    }
}

impl SyntaxReply {
    pub fn receive(
        message: &str,
        ticket: u32,
        source: &str,
    ) -> Option<(SyntaxStatus, Option<std::sync::Arc<super::SyntaxAnalysis>>)> {
        Self::receive_reusing(message, ticket, source, None)
    }
    pub fn receive_reusing(
        message: &str,
        ticket: u32,
        source: &str,
        previous: Option<(u32, &super::SyntaxAnalysis)>,
    ) -> Option<(SyntaxStatus, Option<std::sync::Arc<super::SyntaxAnalysis>>)> {
        Self::receive_source(message, ticket, || std::sync::Arc::from(source), previous)
    }

    /// Retain the facade's immutable snapshot after validating the worker's source.
    pub fn receive_shared(
        message: &str,
        ticket: u32,
        source: std::sync::Arc<str>,
        previous: Option<(u32, &super::SyntaxAnalysis)>,
    ) -> Option<(SyntaxStatus, Option<std::sync::Arc<super::SyntaxAnalysis>>)> {
        Self::receive_source(message, ticket, || source, previous)
    }

    fn receive_source(
        message: &str,
        ticket: u32,
        source: impl FnOnce() -> std::sync::Arc<str>,
        previous: Option<(u32, &super::SyntaxAnalysis)>,
    ) -> Option<(SyntaxStatus, Option<std::sync::Arc<super::SyntaxAnalysis>>)> {
        if message.len() > MAX_ANALYSIS_MESSAGE_BYTES {
            return None;
        }
        let reply: Self = serde_json::from_str(message).ok()?;
        if reply.version != SYNTAX_PROTOCOL_VERSION || reply.ticket != ticket {
            return None;
        }
        let analysis = match (reply.status, reply.analysis) {
            (SyntaxStatus::Ready { .. }, Some(data)) => {
                Some(data.validate_shared(source(), previous)?)
            }
            (
                SyntaxStatus::Cancelled | SyntaxStatus::TooLarge | SyntaxStatus::NeedsSource,
                None,
            ) => None,
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
            source: SyntaxSource::Full(source.into()),
            tab_width: 4,
            base_ticket: None,
        }
    }
    #[test]
    fn source_request_spans_match_all_languages_and_resync_stale_or_evicted_bases() {
        for &(path, body, _) in crate::editor::syntax_contracts::LANGUAGE_CASES {
            let source = format!("{body}{}", "\n".repeat(1000));
            let language = crate::highlight::language_from_path(path);
            let mut service = SyntaxPreparations::default();
            let first = SyntaxRequest::new(42, path.into(), language, &source, 4, None);
            let response = service
                .handle_message(&serde_json::to_string(&first).unwrap(), || true)
                .unwrap();
            let old = SyntaxReply::receive(&response, 42, &source)
                .unwrap()
                .1
                .unwrap();
            let revised = source.replace("文😀", "🦀 changed");
            let update =
                SyntaxRequest::new(43, path.into(), language, &revised, 4, Some((42, &old)));
            assert!(
                matches!(update.source, SyntaxSource::Replace { .. }),
                "{path}"
            );
            let delta = serde_json::to_string(&update).unwrap();
            let full = SyntaxRequest::new(43, path.into(), language, &revised, 4, None);
            assert!(
                delta.len() * 2 < serde_json::to_string(&full).unwrap().len(),
                "{path}"
            );
            let response = service.handle_message(&delta, || true).unwrap();
            let prepared = SyntaxReply::receive_reusing(&response, 43, &revised, Some((42, &old)))
                .unwrap()
                .1
                .unwrap();
            let mut direct = SyntaxPreparations::default();
            let expected = direct
                .prepare(path.to_string(), language, &revised, 4, || true)
                .1
                .unwrap();
            assert_eq!(prepared.folds(), expected.folds(), "{path}");
            assert_eq!(prepared.highlights(), expected.highlights(), "{path}");
            assert_eq!(
                prepared
                    .structure()
                    .map(|value| serde_json::to_value(value.transfer_data()).unwrap()),
                expected
                    .structure()
                    .map(|value| serde_json::to_value(value.transfer_data()).unwrap()),
                "{path}"
            );
            // The worker advanced to ticket 43; a discarded UI reply must resync.
            for evicted in [false, true] {
                if evicted {
                    service.remove(&path.to_string());
                }
                let response = service.handle_message(&delta, || true).unwrap();
                assert_eq!(
                    SyntaxReply::receive_reusing(&response, 43, &revised, Some((42, &old)))
                        .unwrap()
                        .0,
                    SyntaxStatus::NeedsSource
                );
                let response = service
                    .handle_message(&serde_json::to_string(&full).unwrap(), || true)
                    .unwrap();
                assert!(
                    SyntaxReply::receive(&response, 43, &revised)
                        .unwrap()
                        .1
                        .is_some()
                );
            }
        }
    }

    #[test]
    fn malformed_source_requests_and_reconstructed_limits_do_not_publish() {
        let source = format!("{}文😀\r\n", "prefix ".repeat(30));
        let mut service = SyntaxPreparations::default();
        let first = request(&source);
        let response = service
            .handle_message(&serde_json::to_string(&first).unwrap(), || true)
            .unwrap();
        let old = SyntaxReply::receive(&response, 42, &source)
            .unwrap()
            .1
            .unwrap();
        let mut update = SyntaxRequest::new(
            43,
            first.document.clone(),
            first.language,
            &source,
            4,
            Some((42, &old)),
        );
        for (start, end) in [
            (source.find('文').unwrap() + 1, source.len()),
            (2, 1),
            (0, usize::MAX),
        ] {
            update.source = SyntaxSource::Replace {
                start,
                end,
                text: "".into(),
            };
            assert!(
                service
                    .handle_message(&serde_json::to_string(&update).unwrap(), || true)
                    .is_none()
            );
        }
        update.source = SyntaxSource::Replace {
            start: 0,
            end: 0,
            text: "".into(),
        };
        update.base_ticket = None;
        assert!(
            service
                .handle_message(&serde_json::to_string(&update).unwrap(), || true)
                .is_none()
        );
        update.base_ticket = Some(42);
        update.source = SyntaxSource::Replace {
            start: 0,
            end: 0,
            text: "x".repeat(MAX_STRUCTURE_BYTES),
        };
        let response = service
            .handle_message(&serde_json::to_string(&update).unwrap(), || true)
            .unwrap();
        assert_eq!(
            SyntaxReply::receive(&response, 43, &source).unwrap().0,
            SyntaxStatus::TooLarge
        );
        assert!(service.is_empty());
    }

    #[test]
    fn large_worker_updates_transfer_changed_spans_and_share_unchanged_rows() {
        let source = "SELECT name, value FROM items;\r\n".repeat(1000);
        let mut service = SyntaxPreparations::default();
        let mut input = request(&source);
        input.language = Language::Sql;
        let first = service
            .handle_message(&serde_json::to_string(&input).unwrap(), || true)
            .unwrap();
        let old = SyntaxReply::receive(&first, 42, &source)
            .unwrap()
            .1
            .unwrap();
        let revised = source.replacen("value", "revised_value", 1);
        input.source = SyntaxSource::Full(revised.clone());
        input.ticket = 43;
        input.base_ticket = Some(42);
        let delta = service
            .handle_message(&serde_json::to_string(&input).unwrap(), || true)
            .unwrap();
        let wire: serde_json::Value = serde_json::from_str(&delta).unwrap();
        assert_eq!(wire["analysis"]["source"]["text"], "revised_");
        assert_eq!(wire["analysis"]["highlights"].as_array().unwrap().len(), 2);
        assert_eq!(wire["analysis"]["highlights"][1]["count"], 1000);
        assert_eq!(
            wire["analysis"]["source"]["start"],
            source.find("value").unwrap()
        );
        let updated = SyntaxReply::receive_reusing(&delta, 43, &revised, Some((42, &old)))
            .unwrap()
            .1
            .unwrap();
        input.ticket = 44;
        input.base_ticket = None;
        let full = service
            .handle_message(&serde_json::to_string(&input).unwrap(), || true)
            .unwrap();
        assert!(
            delta.len() * 2 < full.len(),
            "changed-span publication must materially reduce this workload"
        );
        assert_eq!(
            updated.highlights(),
            SyntaxReply::receive(&full, 44, &revised)
                .unwrap()
                .1
                .unwrap()
                .highlights()
        );
        assert!(
            old.highlights()
                .unwrap()
                .iter()
                .skip(1)
                .zip(updated.highlights().unwrap().iter().skip(1))
                .all(|(old, new)| std::sync::Arc::ptr_eq(old, new))
        );
    }

    #[test]
    fn row_references_preserve_paint_and_reject_missing_stale_or_mismatching_bases() {
        for (language, source) in [
            (
                Language::Rust,
                "fn main() {\r\n let text = \"文😀\";\r\n}\r\n",
            ),
            (Language::Json, "{\r\n \"name\": \"文😀\"\r\n}\r\n"),
            (
                Language::Sql,
                "/* first\r\nstill comment */\r\nSELECT '文😀';\r\n",
            ),
        ] {
            let mut service = SyntaxPreparations::default();
            let mut input = request(source);
            input.language = language;
            let first = service
                .handle_message(&serde_json::to_string(&input).unwrap(), || true)
                .unwrap();
            let old = SyntaxReply::receive(&first, 42, source).unwrap().1.unwrap();
            let revised = source.replace("文😀", "😀 changed");
            input.source = SyntaxSource::Full(revised.clone());
            input.ticket = 43;
            input.base_ticket = Some(42);
            let next = service
                .handle_message(&serde_json::to_string(&input).unwrap(), || true)
                .unwrap();
            let value: serde_json::Value = serde_json::from_str(&next).unwrap();
            assert_eq!(value["analysis"]["base_ticket"], 42);
            assert!(
                value["analysis"]["highlights"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row.get("reuse").is_some())
            );
            let new = SyntaxReply::receive_reusing(&next, 43, &revised, Some((42, &old)))
                .unwrap()
                .1
                .unwrap();
            assert!(std::sync::Arc::ptr_eq(
                &old.highlights().unwrap()[0],
                &new.highlights().unwrap()[0]
            ));
            let mut direct = SyntaxPreparations::default();
            let expected = direct
                .prepare("direct".to_string(), language, &revised, 4, || true)
                .1
                .unwrap();
            assert_eq!(new.highlights(), expected.highlights());
            assert!(old.matches_source(source));
            assert!(SyntaxReply::receive(&next, 43, &revised).is_none());
            assert!(SyntaxReply::receive_reusing(&next, 43, source, Some((42, &old))).is_none());
            assert!(SyntaxReply::receive_reusing(&next, 43, &revised, Some((41, &old))).is_none());
            assert!(SyntaxReply::receive_reusing(&next, 44, &revised, Some((42, &old))).is_none());
            for replacement in [
                serde_json::json!({"reuse":usize::MAX,"count":1}),
                serde_json::json!({"reuse":1,"count":1}),
                serde_json::json!({"reuse":0,"count":1,"extra":true}),
            ] {
                let mut invalid = value.clone();
                invalid["analysis"]["highlights"][0] = replacement;
                assert!(
                    SyntaxReply::receive_reusing(
                        &invalid.to_string(),
                        43,
                        &revised,
                        Some((42, &old))
                    )
                    .is_none()
                );
            }
            let mut invalid = value;
            invalid["analysis"]
                .as_object_mut()
                .unwrap()
                .remove("base_ticket");
            assert!(
                SyntaxReply::receive_reusing(&invalid.to_string(), 43, &revised, Some((42, &old)))
                    .is_none()
            );
            // A stale/discarded base or LRU eviction returns a complete standalone result.
            for evicted in [false, true] {
                if evicted {
                    service.remove(&input.document);
                }
                input.ticket += 1;
                let full = service
                    .handle_message(&serde_json::to_string(&input).unwrap(), || true)
                    .unwrap();
                assert!(
                    SyntaxReply::receive(&full, input.ticket, &revised)
                        .unwrap()
                        .1
                        .is_some()
                );
            }
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
