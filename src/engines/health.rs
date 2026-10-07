//! Engine health tracking, EWMA latency monitoring, and circuit-breaker quarantine.
//!
//! Engines encountering bot challenges, 403 Forbidden, 429 Too Many Requests,
//! or repeated connection failures are quarantined for 10 minutes (`QUARANTINE_DURATION`).
//! Quarantined engines are skipped pre-fanout, reclaiming 500-1200ms per query.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::Engine;

/// Duration an engine remains quarantined after triggering a circuit breaker.
pub const QUARANTINE_DURATION: Duration = Duration::from_secs(600); // 10 minutes

/// Exponential weighted moving average (EWMA) smoothing factor.
const EWMA_ALPHA: f64 = 0.2;

/// Failure threshold before soft errors trigger quarantine.
const CONSECUTIVE_FAILURE_THRESHOLD: u32 = 3;

#[derive(Clone, Debug)]
pub struct EngineHealthRecord {
    pub engine: Engine,
    pub ewma_latency_ms: f64,
    pub consecutive_failures: u32,
    pub total_successes: u64,
    pub total_failures: u64,
    pub quarantined_until: Option<Instant>,
}

impl EngineHealthRecord {
    pub fn new(engine: Engine) -> Self {
        Self {
            engine,
            ewma_latency_ms: 500.0,
            consecutive_failures: 0,
            total_successes: 0,
            total_failures: 0,
            quarantined_until: None,
        }
    }

    pub fn is_quarantined(&self, now: Instant) -> bool {
        if let Some(until) = self.quarantined_until {
            now < until
        } else {
            false
        }
    }

    pub fn record_success(&mut self, latency_ms: u64) {
        self.consecutive_failures = 0;
        self.total_successes += 1;
        self.quarantined_until = None;
        self.ewma_latency_ms =
            (1.0 - EWMA_ALPHA) * self.ewma_latency_ms + EWMA_ALPHA * (latency_ms as f64);
    }

    pub fn record_failure(&mut self, is_hard_block: bool, now: Instant) {
        self.total_failures += 1;
        self.consecutive_failures += 1;
        if is_hard_block || self.consecutive_failures >= CONSECUTIVE_FAILURE_THRESHOLD {
            log::warn!(
                "[Nexus::Health] Quarantining search engine {:?} for {:?} (consecutive_failures={}, hard_block={})",
                self.engine,
                QUARANTINE_DURATION,
                self.consecutive_failures,
                is_hard_block
            );
            self.quarantined_until = Some(now + QUARANTINE_DURATION);
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EngineHealthTracker {
    records: HashMap<Engine, EngineHealthRecord>,
}

impl EngineHealthTracker {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
        }
    }

    pub fn is_quarantined(&self, engine: Engine) -> bool {
        let now = Instant::now();
        self.records
            .get(&engine)
            .map(|r| r.is_quarantined(now))
            .unwrap_or(false)
    }

    pub fn record_success(&mut self, engine: Engine, latency_ms: u64) {
        self.records
            .entry(engine)
            .or_insert_with(|| EngineHealthRecord::new(engine))
            .record_success(latency_ms);
    }

    pub fn record_failure(&mut self, engine: Engine, is_hard_block: bool) {
        let now = Instant::now();
        self.records
            .entry(engine)
            .or_insert_with(|| EngineHealthRecord::new(engine))
            .record_failure(is_hard_block, now);
    }

    pub fn get_ewma_latency(&self, engine: Engine) -> f64 {
        self.records
            .get(&engine)
            .map(|r| r.ewma_latency_ms)
            .unwrap_or(500.0)
    }

    pub fn consecutive_failures(&self, engine: Engine) -> u32 {
        self.records
            .get(&engine)
            .map(|r| r.consecutive_failures)
            .unwrap_or(0)
    }
}
