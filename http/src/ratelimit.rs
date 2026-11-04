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
        self.check_at(bucket, cfg, Instant::now())
    }

    /// Clock-injected variant of [`Self::check`] so the 60s window rollover is testable.
    fn check_at(&self, bucket: &str, cfg: &RateLimitCfg, now: Instant) -> Result<(), RouterError> {
        if !cfg.enabled || cfg.requests_per_minute == 0 {
            return Ok(());
        }
        let mut g = self.inner.lock().unwrap();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_isolation_and_disabled() {
        let lim = RateLimiter::default();
        let cfg = RateLimitCfg {
            enabled: true,
            requests_per_minute: 2,
        };
        assert!(lim.check("a", &cfg).is_ok());
        assert!(lim.check("a", &cfg).is_ok());
        assert!(matches!(
            lim.check("a", &cfg),
            Err(RouterError::RateLimited(_))
        ));
        assert!(lim.check("b", &cfg).is_ok());

        let off = RateLimitCfg {
            enabled: false,
            requests_per_minute: 1,
        };
        assert!(lim.check("a", &off).is_ok());

        let zero = RateLimitCfg {
            enabled: true,
            requests_per_minute: 0,
        };
        assert!(lim.check("a", &zero).is_ok());
    }

    #[test]
    fn window_rolls_over_after_sixty_seconds() {
        let lim = RateLimiter::default();
        let cfg = RateLimitCfg {
            enabled: true,
            requests_per_minute: 1,
        };
        let t0 = Instant::now();
        assert!(lim.check_at("a", &cfg, t0).is_ok());
        assert!(matches!(
            lim.check_at("a", &cfg, t0 + Duration::from_secs(1)),
            Err(RouterError::RateLimited(_))
        ));
        // A full window later the bucket resets and admits the next request.
        assert!(lim
            .check_at("a", &cfg, t0 + Duration::from_secs(61))
            .is_ok());
    }
}
