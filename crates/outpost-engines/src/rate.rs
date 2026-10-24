//! Download speed and time left, smoothed so the numbers don't jump about.

use std::time::{Duration, Instant};

/// Follows a running byte count. Feed it the bytes done so far, as often as you like.
#[derive(Debug, Clone)]
pub struct Rate {
    last: Option<(Instant, u64)>,
    bps: Option<f64>,
}

/// Samples closer together than this are folded into the next one.
const MIN_GAP: Duration = Duration::from_millis(500);
/// How much each new sample counts (higher reacts faster, lower is steadier).
const WEIGHT: f64 = 0.3;

impl Default for Rate {
    fn default() -> Self {
        Self::new()
    }
}

impl Rate {
    pub fn new() -> Self {
        Rate { last: None, bps: None }
    }

    pub fn update(&mut self, done: u64) {
        self.update_at(Instant::now(), done);
    }

    pub fn update_at(&mut self, now: Instant, done: u64) {
        let Some((t, before)) = self.last else {
            self.last = Some((now, done));
            return;
        };
        if done < before {
            // A new file or layer started from zero: keep the speed, restart the count.
            self.last = Some((now, done));
            return;
        }
        let gap = now.saturating_duration_since(t);
        if gap < MIN_GAP {
            return;
        }
        let sample = (done - before) as f64 / gap.as_secs_f64();
        self.bps = Some(match self.bps {
            Some(old) => old + WEIGHT * (sample - old),
            None => sample,
        });
        self.last = Some((now, done));
    }

    /// Bytes a second, once there is enough to say.
    pub fn bytes_per_sec(&self) -> Option<u64> {
        self.bps.map(|b| b.max(0.0) as u64)
    }

    /// Seconds until `remaining` more bytes are done at the current speed.
    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        match self.bps {
            Some(b) if b >= 1.0 => Some((remaining as f64 / b).ceil() as u64),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_and_time_left() {
        let start = Instant::now();
        let mut r = Rate::new();
        assert_eq!(r.bytes_per_sec(), None);
        r.update_at(start, 0);
        r.update_at(start + Duration::from_secs(1), 10_000_000);
        assert_eq!(r.bytes_per_sec(), Some(10_000_000));
        assert_eq!(r.eta_secs(50_000_000), Some(5));
        // Quick repeats are folded in.
        r.update_at(start + Duration::from_millis(1100), 10_500_000);
        assert_eq!(r.bytes_per_sec(), Some(10_000_000));
        // Smoothed: a slow second only pulls it down part way.
        r.update_at(start + Duration::from_secs(2), 12_000_000);
        let s = r.bytes_per_sec().unwrap();
        assert!(s < 10_000_000 && s > 2_000_000, "{s}");
        // A new file restarts from zero without a silly speed.
        r.update_at(start + Duration::from_secs(3), 0);
        assert_eq!(r.bytes_per_sec(), Some(s));
        // Stalled: no time left to promise.
        let mut stalled = Rate::new();
        stalled.update_at(start, 5);
        stalled.update_at(start + Duration::from_secs(1), 5);
        assert_eq!(stalled.eta_secs(100), None);
    }
}
