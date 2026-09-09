//! Soft per-key / anonymous rate limit (429).

use aria_router_config::RateLimitCfg;
use aria_router_core::RouterError;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct RateLimiter {
    inner: Mutex<HashMap<String, Window>>,
}

#[derive(Debug)]
struct Window {
    start: Instant,
    count: u32,
}

impl RateLimiter {
    pub fn check(&self, bucket: &str, cfg: &RateLimitCfg) -> Result<(), RouterError> {
        if !cfg.enabled || cfg.requests_per_minute == 0 {
            return Ok(());
        }
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        let w = g.entry(bucket.to_string()).or_insert(Window {
            start: now,
            count: 0,
        });
        if now.duration_since(w.start) >= Duration::from_secs(60) {
            w.start = now;
            w.count = 0;
        }
        if w.count >= cfg.requests_per_minute {
            return Err(RouterError::RateLimited(format!(
                "bucket {bucket} exceeded {} rpm",
                cfg.requests_per_minute
            )));
        }
        w.count += 1;
        Ok(())
    }
}
