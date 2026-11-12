//! Small numeric helpers shared by the animation code: frame-rate independent smoothing, a damped
//! spring for follow-through and overshoot, and the easing curves.

use std::f32::consts::{PI, TAU};

/// Moves `current` toward `target`, covering `1 - 1/e` of the gap every `1 / rate` seconds
/// whatever the frame rate.
pub fn approach(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-rate * dt).exp())
}

/// The blend factor [`approach`] uses, for values that are not plain `f32`s.
pub fn approach_factor(rate: f32, dt: f32) -> f32 {
    1.0 - (-rate * dt).exp()
}

/// Hermite smoothstep of `x` between `edge0` and `edge1`.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ease-out with a little overshoot past 1, the snap anime poses settle with.
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0) - 1.0;
    const S: f32 = 1.35;
    1.0 + t * t * ((S + 1.0) * t + S)
}

/// A single bump: 0 at both ends of `0..1`, 1 in the middle.
pub fn bump(t: f32) -> f32 {
    if (0.0..=1.0).contains(&t) {
        (t * PI).sin()
    } else {
        0.0
    }
}

/// Wraps an angle into `-π..π`.
pub fn wrap_angle(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// A damped spring chasing a target: the source of every overshoot, bounce and settle.
#[derive(Clone, Copy, Debug, Default)]
pub struct Spring {
    pub value: f32,
    pub velocity: f32,
}

impl Spring {
    /// Advances by `dt`. `omega` is the natural frequency in rad/s; `zeta` the damping ratio
    /// (1 = no overshoot, below 1 bounces). Sub-stepped so a slow frame cannot blow it up.
    pub fn step(&mut self, target: f32, omega: f32, zeta: f32, dt: f32) {
        let steps = ((dt * omega / 0.4).ceil() as usize).clamp(1, 12);
        let h = dt / steps as f32;
        for _ in 0..steps {
            let accel = omega * omega * (target - self.value) - 2.0 * zeta * omega * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }
    }

    /// An instantaneous change of velocity, e.g. the jolt of landing.
    pub fn kick(&mut self, impulse: f32) {
        self.velocity += impulse;
    }
}

/// Deterministic pseudo-random numbers for blink timing, fidgets and gesture choice.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Default for Rng {
    fn default() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        Rng(nanos | 1)
    }
}

impl Rng {
    #[cfg(test)]
    pub fn seeded(seed: u32) -> Self {
        Rng(seed | 1)
    }

    /// Uniform in `0..1` (xorshift32).
    pub fn unit(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    pub fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approach_is_frame_rate_independent() {
        let mut coarse = 0.0;
        coarse = approach(coarse, 1.0, 6.0, 0.1);
        let mut fine = 0.0;
        for _ in 0..10 {
            fine = approach(fine, 1.0, 6.0, 0.01);
        }
        assert!((coarse - fine).abs() < 1e-4);
    }

    #[test]
    fn an_underdamped_spring_overshoots_then_settles() {
        let mut spring = Spring::default();
        let mut peak = 0.0f32;
        for _ in 0..600 {
            spring.step(1.0, 14.0, 0.4, 1.0 / 60.0);
            peak = peak.max(spring.value);
        }
        assert!(peak > 1.05, "expected overshoot, peak {peak}");
        assert!((spring.value - 1.0).abs() < 1e-3);
        // A long frame must stay stable.
        let mut slow = Spring::default();
        for _ in 0..40 {
            slow.step(1.0, 30.0, 0.3, 0.1);
        }
        assert!(slow.value.is_finite() && (slow.value - 1.0).abs() < 0.05);
    }

    #[test]
    fn wrap_angle_takes_the_short_way() {
        assert!((wrap_angle(6.0) - (6.0 - TAU)).abs() < 1e-5);
        assert!((wrap_angle(-4.0) - (TAU - 4.0)).abs() < 1e-5);
        assert_eq!(wrap_angle(0.5), 0.5);
    }
}
