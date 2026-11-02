//! Motion (ui skill §3, math.md → Motion): critically damped springs that never
//! overshoot, and a cubic-bezier tween for drawers. Both advance by real `dt` with a
//! closed form, so they are exact at 75 or 165 Hz.

/// Values closer than this to the target count as arrived (px or 0-1 units).
const PRECISION: f32 = 0.01;

/// ζ = 1 spring, ω = 2π / duration (boltsnap's convention, ≈ 98.6 % there at T).
#[derive(Clone, Copy, Debug)]
pub struct Spring {
    pub value: f32,
    pub target: f32,
    vel: f32,
    omega: f32,
}

impl Spring {
    pub fn new(value: f32, duration: f32) -> Self {
        Spring {
            value,
            target: value,
            vel: 0.0,
            omega: std::f32::consts::TAU / duration.max(1e-3),
        }
    }

    /// Head for `target`, keeping the current value and velocity (no jump).
    pub fn set(&mut self, target: f32) {
        self.target = target;
    }

    /// Be at the target now (reduced motion, first layout).
    pub fn finish(&mut self) {
        (self.value, self.vel) = (self.target, 0.0);
    }

    pub fn moving(&self) -> bool {
        self.value != self.target || self.vel != 0.0
    }

    /// Advance `dt` seconds. Returns whether it is still moving.
    pub fn step(&mut self, dt: f32) -> bool {
        if !self.moving() {
            return false;
        }
        let (w, x0, v0) = (self.omega, self.value - self.target, self.vel);
        let e = (-w * dt).exp();
        let x = (x0 + (v0 + w * x0) * dt) * e;
        let v = (v0 - w * (v0 + w * x0) * dt) * e;
        // ζ = 1 still overshoots once if launched toward the target fast: clamp.
        let crossed = x != 0.0 && x.signum() != x0.signum() && x0 != 0.0;
        if crossed || (x.abs() <= PRECISION && (v * dt).abs() < PRECISION) {
            self.finish();
            return false;
        }
        (self.value, self.vel) = (self.target + x, v);
        true
    }
}

/// From `from` to `to` over `duration` s on a CSS-style cubic bezier.
#[derive(Clone, Copy, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    t: f32,
    duration: f32,
    curve: (f32, f32, f32, f32),
}

impl Tween {
    pub fn new(value: f32, duration: f32, curve: (f32, f32, f32, f32)) -> Self {
        Tween {
            from: value,
            to: value,
            t: 1.0,
            duration: duration.max(1e-3),
            curve,
        }
    }

    pub fn value(&self) -> f32 {
        let (x1, y1, x2, y2) = self.curve;
        self.from + (self.to - self.from) * bezier_y(self.t, x1, y1, x2, y2)
    }

    /// Start over from where it is now toward `to`.
    pub fn set(&mut self, to: f32) {
        if to != self.to {
            (self.from, self.to, self.t) = (self.value(), to, 0.0);
        }
    }

    pub fn finish(&mut self) {
        self.t = 1.0;
    }

    pub fn moving(&self) -> bool {
        self.t < 1.0
    }

    pub fn step(&mut self, dt: f32) -> bool {
        self.t = (self.t + dt / self.duration).min(1.0);
        self.moving()
    }
}

/// y of the bezier (0,0) (x1,y1) (x2,y2) (1,1) at x = `x`: solve x(s) = x by
/// bisection (monotone in s for 0 ≤ x1, x2 ≤ 1), then evaluate y(s).
fn bezier_y(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let at = |s: f32, a: f32, b: f32| {
        let i = 1.0 - s;
        3.0 * i * i * s * a + 3.0 * i * s * s * b + s * s * s
    };
    if x <= 0.0 || x >= 1.0 {
        return x.clamp(0.0, 1.0); // exact ends: a finished drawer sits on whole px
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if at(mid, x1, x2) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at((lo + hi) / 2.0, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_settles_without_overshoot() {
        let mut s = Spring::new(0.0, 0.22);
        s.set(100.0);
        let mut t = 0.0;
        while s.step(1.0 / 165.0) {
            assert!(s.value <= 100.0, "overshoot {}", s.value);
            t += 1.0 / 165.0;
        }
        assert_eq!(s.value, 100.0);
        assert!(t > 0.15 && t < 0.6, "settled after {t}");
    }

    #[test]
    fn spring_is_frame_rate_independent() {
        let (mut a, mut b) = (Spring::new(0.0, 0.22), Spring::new(0.0, 0.22));
        a.set(1.0);
        b.set(1.0);
        for _ in 0..33 {
            a.step(1.0 / 165.0); // 0.2 s at 165 Hz
        }
        for _ in 0..15 {
            b.step(1.0 / 75.0); // 0.2 s at 75 Hz
        }
        assert!(
            (a.value - b.value).abs() < 1e-3,
            "{} vs {}",
            a.value,
            b.value
        );
    }

    #[test]
    fn retarget_keeps_value() {
        let mut s = Spring::new(0.0, 0.15);
        s.set(10.0);
        s.step(0.05);
        let here = s.value;
        s.set(-10.0);
        assert_eq!(s.value, here);
        while s.step(1.0 / 165.0) {}
        assert_eq!(s.value, -10.0);
    }

    #[test]
    fn tween_ends_and_curve() {
        let mut t = Tween::new(0.0, 0.5, (0.76, 0.0, 0.12, 1.0));
        t.set(1.0);
        assert_eq!(t.value(), 0.0);
        t.step(0.25);
        let mid = t.value();
        assert!(mid > 0.3 && mid < 0.7, "{mid}"); // slow start, fast middle
        t.step(0.25);
        assert!(!t.moving());
        assert!((t.value() - 1.0).abs() < 1e-4);
    }
}
