//! One admission policy for borrowed synchronous and source-owned yielding jobs.
use super::MAX_STRUCTURE_BYTES;
use std::sync::Arc;
const MAX_SYNTAX_NEWLINES: usize = 50_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxAdmissionStatus {
    Pending,
    Admitted,
    TooLarge,
}

struct AdmissionScan {
    next: usize,
    breaks: usize,
    status: SyntaxAdmissionStatus,
}
impl AdmissionScan {
    fn new(bytes: usize) -> Self {
        Self {
            next: 0,
            breaks: 0,
            status: if bytes > MAX_STRUCTURE_BYTES {
                SyntaxAdmissionStatus::TooLarge
            } else if bytes == 0 {
                SyntaxAdmissionStatus::Admitted
            } else {
                SyntaxAdmissionStatus::Pending
            },
        }
    }
    fn advance(&mut self, source: &str, budget: usize) {
        if self.status != SyntaxAdmissionStatus::Pending || budget == 0 {
            return;
        }
        let end = source.len().min(self.next.saturating_add(budget));
        for byte in &source.as_bytes()[self.next..end] {
            self.next += 1;
            if *byte == b'\n' {
                self.breaks += 1;
                if self.breaks >= MAX_SYNTAX_NEWLINES {
                    self.status = SyntaxAdmissionStatus::TooLarge;
                    return;
                }
            }
        }
        if self.next == source.len() {
            self.status = SyntaxAdmissionStatus::Admitted;
        }
    }
}

pub struct SyntaxAdmission {
    source: Arc<String>,
    scan: AdmissionScan,
}
impl SyntaxAdmission {
    pub fn new(source: Arc<String>) -> Self {
        Self {
            scan: AdmissionScan::new(source.len()),
            source,
        }
    }
    pub const fn status(&self) -> SyntaxAdmissionStatus {
        self.scan.status
    }
    pub fn source_snapshot(&self) -> &Arc<String> {
        &self.source
    }
    pub fn advance(&mut self, budget: usize) -> SyntaxAdmissionStatus {
        self.scan.advance(&self.source, budget);
        self.status()
    }
}

pub fn preparation_exceeds_limits(text: &str) -> bool {
    let mut scan = AdmissionScan::new(text.len());
    scan.advance(text, usize::MAX);
    scan.status == SyntaxAdmissionStatus::TooLarge
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_admission_matches_byte_row_and_unicode_boundaries_without_copying() {
        for (source, expected) in [
            (String::new(), SyntaxAdmissionStatus::Admitted),
            ("文😀\r\n".repeat(40_000), SyntaxAdmissionStatus::Admitted),
            ("\n".repeat(49_999), SyntaxAdmissionStatus::Admitted),
            ("\n".repeat(50_000), SyntaxAdmissionStatus::TooLarge),
            (
                "x".repeat(MAX_STRUCTURE_BYTES),
                SyntaxAdmissionStatus::Admitted,
            ),
            (
                "x".repeat(MAX_STRUCTURE_BYTES + 1),
                SyntaxAdmissionStatus::TooLarge,
            ),
        ] {
            assert_eq!(
                preparation_exceeds_limits(&source),
                expected == SyntaxAdmissionStatus::TooLarge
            );
            let source = Arc::new(source);
            for budget in [7, 64 * 1024] {
                let mut job = SyntaxAdmission::new(source.clone());
                let initial = job.status();
                assert_eq!(job.advance(0), initial);
                while job.status() == SyntaxAdmissionStatus::Pending {
                    let previous = job.scan.next;
                    job.advance(budget);
                    assert!(job.scan.next - previous <= budget);
                }
                assert_eq!(job.status(), expected);
                assert!(Arc::ptr_eq(job.source_snapshot(), &source));
            }
        }
    }
}
