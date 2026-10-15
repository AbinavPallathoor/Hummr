//! Smoothing for things that move every frame: meters, the playhead and the
//! live pitch needle. The audio thread reports values in coarse, jittery
//! steps; these turn them into steady motion.

use vme_core::dsp::amp_to_db;

/// Level meter: instant attack, 30 dB/s release, 1 s peak hold then 20 dB/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub level_db: f32,
    pub peak_db: f32,
    hold: f32,
}

impl Default for Meter {
    fn default() -> Self {
        Self {
            level_db: Self::FLOOR,
            peak_db: Self::FLOOR,
            hold: 0.0,
        }
    }
}

impl Meter {
    pub const FLOOR: f32 = -60.0;

    pub fn update(&mut self, peak_linear: f32, dt: f32) {
        let db = amp_to_db(peak_linear).max(Self::FLOOR);
        self.level_db = if db >= self.level_db {
            db
        } else {
            (self.level_db - 30.0 * dt).max(db)
        };
        if db >= self.peak_db {
            self.peak_db = db;
            self.hold = 1.0;
        } else if self.hold > 0.0 {
            self.hold -= dt;
        } else {
            self.peak_db = (self.peak_db - 20.0 * dt).max(self.level_db);
        }
    }

    fn frac(db: f32) -> f32 {
        ((db - Self::FLOOR) / -Self::FLOOR).clamp(0.0, 1.0)
    }

    pub fn fraction(&self) -> f32 {
        Self::frac(self.level_db)
    }

    pub fn peak_fraction(&self) -> f32 {
        Self::frac(self.peak_db)
    }

    pub fn is_idle(&self) -> bool {
        self.level_db <= Self::FLOOR && self.peak_db <= Self::FLOOR
    }
}

/// Follows a position that advances at a known rate but is only reported
/// in buffer-sized jumps. Predicts forward, then eases toward reports.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Follower {
    pub value: f64,
}

impl Follower {
    /// `rate` is units per second while running.
    pub fn update(&mut self, target: f64, rate: f64, dt: f64, running: bool) {
        if !running {
            self.value = target;
            return;
        }
        let predicted = self.value + rate * dt;
        if (target - predicted).abs() > 0.25 * rate.abs().max(1e-9) {
            // Seek or restart: jump rather than glide.
            self.value = target;
        } else {
            let k = 1.0 - (-dt / 0.08).exp();
            self.value = predicted + (target - predicted) * k;
        }
    }
}

/// Exponential moving average with time constant `tau` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ema {
    pub value: Option<f32>,
    pub tau: f32,
}

impl Ema {
    pub fn new(tau: f32) -> Self {
        Self { value: None, tau }
    }

    pub fn update(&mut self, target: Option<f32>, dt: f32) {
        self.value = match (self.value, target) {
            (Some(v), Some(t)) if (v - t).abs() < 1.0 => {
                Some(v + (t - v) * (1.0 - (-dt / self.tau).exp()))
            }
            (_, t) => t,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_attacks_instantly_and_releases_slowly() {
        let mut m = Meter::default();
        m.update(1.0, 0.016);
        assert_eq!(m.level_db, 0.0);
        m.update(0.0, 0.1);
        assert!((m.level_db + 3.0).abs() < 1e-3);
        assert_eq!(m.peak_db, 0.0, "peak holds");
        for _ in 0..320 {
            m.update(0.0, 0.016);
        }
        assert!(m.is_idle());
    }

    #[test]
    fn follower_glides_and_jumps() {
        let mut f = Follower::default();
        f.update(0.0, 1.0, 0.016, true);
        for _ in 0..10 {
            let target = f.value + 0.016 + 0.003;
            f.update(target, 1.0, 0.016, true);
        }
        assert!(f.value > 0.15 && f.value < 0.2);
        f.update(5.0, 1.0, 0.016, true);
        assert_eq!(f.value, 5.0);
    }

    #[test]
    fn ema_snaps_on_large_changes() {
        let mut e = Ema::new(0.1);
        e.update(Some(60.0), 0.016);
        e.update(Some(60.4), 0.016);
        let v = e.value.unwrap();
        assert!(v > 60.0 && v < 60.4);
        e.update(Some(67.0), 0.016);
        assert_eq!(e.value, Some(67.0));
        e.update(None, 0.016);
        assert_eq!(e.value, None);
    }
}
