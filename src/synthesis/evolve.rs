//! Local (no-AI) patch exploration on normalized parameter vectors:
//! randomize, mutate and breed. Locked params are never touched.

use super::expr::white;

pub struct Rng(pub u32);

impl Rng {
    pub fn seeded() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1);
        Rng(t | 1)
    }
    /// -1..1
    pub fn bi(&mut self) -> f32 {
        white(&mut self.0)
    }
    /// 0..1
    pub fn uni(&mut self) -> f32 {
        self.bi() * 0.5 + 0.5
    }
    /// Roughly Gaussian (sum of three uniforms), about -1..1.
    pub fn gauss(&mut self) -> f32 {
        (self.bi() + self.bi() + self.bi()) / 3.0 * 1.7
    }
}

/// Move every unlocked param toward a random value by `strength` (0..1).
pub fn randomize(values: &mut [f32], locks: &[bool], strength: f32, rng: &mut Rng) {
    let s = strength.clamp(0.0, 1.0);
    for (i, v) in values.iter_mut().enumerate() {
        if locks.get(i).copied().unwrap_or(false) {
            continue;
        }
        let target = rng.uni();
        *v = (*v + (target - *v) * s).clamp(0.0, 1.0);
    }
}

/// Small Gaussian nudges; `amount` 0..1 scales the spread.
pub fn mutate(values: &[f32], locks: &[bool], amount: f32, rng: &mut Rng) -> Vec<f32> {
    let spread = 0.02 + amount.clamp(0.0, 1.0) * 0.3;
    values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if locks.get(i).copied().unwrap_or(false) {
                *v
            } else {
                (*v + rng.gauss() * spread).clamp(0.0, 1.0)
            }
        })
        .collect()
}

/// Child of two parents: per-param blend with a random bias, plus mutation.
pub fn breed(a: &[f32], b: &[f32], locks: &[bool], amount: f32, rng: &mut Rng) -> Vec<f32> {
    let n = a.len().max(b.len());
    let child: Vec<f32> = (0..n)
        .map(|i| {
            let (x, y) = (a.get(i).copied().unwrap_or(0.5), b.get(i).copied().unwrap_or(0.5));
            if locks.get(i).copied().unwrap_or(false) {
                return x;
            }
            // mostly pick one parent, sometimes blend
            let r = rng.uni();
            if r < 0.4 {
                x
            } else if r < 0.8 {
                y
            } else {
                x + (y - x) * rng.uni()
            }
        })
        .collect();
    mutate(&child, locks, amount * 0.5, rng)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_are_respected_everywhere() {
        let mut rng = Rng(42);
        let base = vec![0.5; 8];
        let mut locks = vec![false; 8];
        locks[3] = true;
        let mut r = base.clone();
        randomize(&mut r, &locks, 1.0, &mut rng);
        assert_eq!(r[3], 0.5);
        assert!(r.iter().enumerate().any(|(i, v)| i != 3 && (*v - 0.5).abs() > 0.01));
        assert_eq!(mutate(&base, &locks, 1.0, &mut rng)[3], 0.5);
        assert_eq!(breed(&base, &vec![0.9; 8], &locks, 1.0, &mut rng)[3], 0.5);
    }

    #[test]
    fn everything_stays_normalized() {
        let mut rng = Rng(7);
        let v = vec![0.0, 1.0, 0.99, 0.01];
        for _ in 0..200 {
            let m = mutate(&v, &[], 1.0, &mut rng);
            assert!(m.iter().all(|x| (0.0..=1.0).contains(x)));
        }
    }
}
