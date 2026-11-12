//! Head-pat hearts, ported from `spawnHeadpatHearts()` / `renderHeadpatHearts()`.
//!
//! The reference painted emoji hearts on a 2D canvas over the stage. Here they are painted straight
//! into the captured RGBA frame just before it is presented, which keeps them in screen space like
//! the original and needs no text or UI rendering in Bevy.

use bevy::prelude::*;

#[derive(Clone, Copy)]
struct Heart {
    /// Position as a fraction of the frame.
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    /// Size in reference pixels (the reference stage was ~520 px wide).
    size: f32,
    alpha: f32,
    decay: f32,
    scale: f32,
    /// Pink or a lighter rose, so the burst is not one flat colour.
    tint: [f32; 3],
}

#[derive(Resource, Default)]
pub struct Hearts {
    live: Vec<Heart>,
    seed: u32,
}

impl Hearts {
    fn rand(&mut self) -> f32 {
        // xorshift32; hearts do not need a real RNG.
        if self.seed == 0 {
            self.seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(1)
                | 1;
        }
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x % 100_000) as f32 / 100_000.0
    }

    /// Eight hearts above the head.
    pub fn burst(&mut self) {
        for _ in 0..8 {
            let tint = if self.rand() < 0.5 {
                [244.0, 114.0, 182.0]
            } else {
                [251.0, 164.0, 205.0]
            };
            let heart = Heart {
                x: 0.44 + (self.rand() - 0.5) * 0.28,
                y: 0.25 + self.rand() * 0.12,
                vx: (self.rand() - 0.5) * 0.16,
                vy: -0.22 - self.rand() * 0.25,
                size: 18.0 + self.rand() * 16.0,
                alpha: 1.0,
                decay: 0.45 + self.rand() * 0.35,
                scale: 0.4,
                tint,
            };
            self.live.push(heart);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn step(&mut self, dt: f32) {
        for h in &mut self.live {
            h.x += h.vx * dt;
            h.y += h.vy * dt;
            h.scale = (h.scale + 2.2 * dt).min(1.0);
            h.alpha -= h.decay * dt;
        }
        self.live.retain(|h| h.alpha > 0.0 && h.y >= -0.1);
    }

    /// Composites the hearts over a straight-alpha RGBA8 frame.
    pub fn paint(&self, rgba: &mut [u8], width: u32, height: u32) {
        let px_scale = width as f32 / 520.0;
        for h in &self.live {
            let radius = (h.size * h.scale * px_scale * 0.5).max(2.0);
            let cx = h.x * width as f32;
            let cy = h.y * height as f32;
            let x0 = (cx - radius * 1.3).floor().max(0.0) as i32;
            let x1 = (cx + radius * 1.3).ceil().min(width as f32 - 1.0) as i32;
            let y0 = (cy - radius * 1.3).floor().max(0.0) as i32;
            let y1 = (cy + radius * 1.3).ceil().min(height as f32 - 1.0) as i32;
            for py in y0..=y1 {
                for px in x0..=x1 {
                    // Heart curve (x² + y² − 1)³ − x²y³ ≤ 0, y up.
                    let u = (px as f32 + 0.5 - cx) / radius;
                    let v = -(py as f32 + 0.5 - cy) / radius + 0.15;
                    let f = (u * u + v * v - 1.0).powi(3) - u * u * v * v * v;
                    // Soft edge plus a faint glow, like the reference's shadowBlur.
                    let coverage = if f <= 0.0 {
                        1.0
                    } else {
                        (1.0 - f * 6.0).clamp(0.0, 1.0) * 0.35
                    };
                    if coverage <= 0.0 {
                        continue;
                    }
                    let a = coverage * h.alpha.clamp(0.0, 1.0);
                    let i = ((py as u32 * width + px as u32) * 4) as usize;
                    let Some(dst) = rgba.get_mut(i..i + 4) else { continue };
                    let da = dst[3] as f32 / 255.0;
                    let out_a = a + da * (1.0 - a);
                    if out_a <= 0.0 {
                        continue;
                    }
                    for (channel, tint) in dst.iter_mut().zip(h.tint) {
                        let blended = (tint * a + *channel as f32 * da * (1.0 - a)) / out_a;
                        *channel = blended.clamp(0.0, 255.0) as u8;
                    }
                    dst[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hearts_paint_and_fade_out() {
        let mut hearts = Hearts::default();
        hearts.burst();
        assert_eq!(hearts.live.len(), 8);
        let (w, h) = (260u32, 370u32);
        let mut frame = vec![0u8; (w * h * 4) as usize];
        hearts.step(0.3);
        hearts.paint(&mut frame, w, h);
        assert!(frame.as_chunks::<4>().0.iter().any(|p| p[3] > 200), "hearts should be visible");
        for _ in 0..60 {
            hearts.step(0.1);
        }
        assert!(hearts.is_empty(), "hearts must expire");
    }
}
