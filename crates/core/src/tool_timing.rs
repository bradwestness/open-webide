//! Durable tool execution timing, excluding time waiting for approval.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTiming {
    pub started_at_ms: u64,
    pub elapsed_ms: u64,
    pub finished: bool,
}
impl ToolTiming {
    pub fn start(now_ms: u64) -> Self {
        Self {
            started_at_ms: now_ms,
            elapsed_ms: 0,
            finished: false,
        }
    }
    pub fn sample(self, now_ms: u64, finished: bool) -> Self {
        if self.finished {
            return self;
        }
        Self {
            elapsed_ms: self
                .elapsed_ms
                .max(now_ms.saturating_sub(self.started_at_ms)),
            finished,
            ..self
        }
    }
    /// Delayed/replayed progress cannot replace a completed duration or change
    /// the identity of the observed execution.
    pub fn merge(self, update: Self) -> Result<Self, &'static str> {
        if self.started_at_ms != update.started_at_ms {
            return Err("Tool timing start does not match its execution");
        }
        if self.finished || (!update.finished && update.elapsed_ms < self.elapsed_ms) {
            return Ok(self);
        }
        Ok(update)
    }
}

/// Client-side interpolation of sampled host timing. Server and browser epoch
/// clocks may differ: advance from the received duration using the client clock.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LiveToolTiming {
    timing: Option<ToolTiming>,
    received_at_ms: f64,
    elapsed_ms: f64,
}
impl LiveToolTiming {
    pub fn observe(&mut self, timing: Option<ToolTiming>, client_now_ms: f64) {
        if !client_now_ms.is_finite() {
            return;
        }
        let timing = match (self.timing, timing) {
            (Some(previous), Some(update)) if previous.started_at_ms == update.started_at_ms => {
                Some(previous.merge(update).unwrap_or(previous))
            }
            (_, timing) => timing,
        };
        let elapsed = self.elapsed_ms(client_now_ms);
        #[allow(
            clippy::cast_precision_loss,
            reason = "Human-scale durations intentionally use display precision"
        )]
        {
            self.elapsed_ms = timing.map_or(0.0, |timing| {
                if timing.finished
                    || self
                        .timing
                        .is_none_or(|previous| previous.started_at_ms != timing.started_at_ms)
                {
                    timing.elapsed_ms as f64
                } else {
                    elapsed.unwrap_or_default().max(timing.elapsed_ms as f64)
                }
            });
        }
        self.timing = timing;
        self.received_at_ms = client_now_ms;
    }
    pub fn elapsed_ms(self, client_now_ms: f64) -> Option<f64> {
        self.timing.map(|timing| {
            self.elapsed_ms
                + if !timing.finished && client_now_ms.is_finite() {
                    (client_now_ms - self.received_at_ms).max(0.0)
                } else {
                    0.0
                }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_timing_uses_received_duration_and_survives_updates_and_clock_skew() {
        let mut live = LiveToolTiming::default();
        let timing = ToolTiming::start(100).sample(3100, false);
        live.observe(Some(timing), 1_000_000.0);
        assert_eq!(live.elapsed_ms(1_000_500.0), Some(3500.0));
        live.observe(Some(ToolTiming::start(100)), 1_000_500.0);
        assert_eq!(live.elapsed_ms(1_000_700.0), Some(3700.0));
        let finished = timing.sample(4100, true);
        live.observe(Some(finished), 1_000_700.0);
        assert_eq!(live.elapsed_ms(2_000_000.0), Some(4000.0));
        live.observe(Some(timing), 2_000_000.0);
        assert_eq!(live.elapsed_ms(3_000_000.0), Some(4000.0));
        live.observe(None, 3_000_000.0);
        assert_eq!(live.elapsed_ms(4_000_000.0), None);
    }

    #[test]
    fn timing_is_monotonic_and_finished_duration_is_immutable() {
        let timing = ToolTiming::start(1000).sample(1500, false);
        assert_eq!(timing.elapsed_ms, 500);
        assert_eq!(timing.sample(900, false).elapsed_ms, 500);
        let finished = timing.sample(1900, true);
        assert_eq!(finished.elapsed_ms, 900);
        assert_eq!(finished.sample(9999, false), finished);
        assert_eq!(finished.merge(timing).unwrap(), finished);
        assert_eq!(timing.merge(ToolTiming::start(1000)).unwrap(), timing);
        assert!(timing.merge(ToolTiming::start(2000)).is_err());
    }
}
