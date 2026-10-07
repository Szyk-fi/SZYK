//! Blaster's live parameter values, shared between the UI thread and the
//! audio thread without a lock. The conversions (clamping, normalised
//! position, display text) are Hydra's, which work on any table's `Def`.

use super::params::{Def, Kind, COUNT, DEFS, P};
pub use crate::apps::hydra::store::{clamp_value, format_value, from_norm, to_norm};
use crate::util::AtomicF32;

/// Every value at one instant: what the audio thread renders a block from.
pub type Snapshot = [f32; COUNT];

pub struct Params {
    v: Vec<AtomicF32>,
}

impl Default for Params {
    fn default() -> Self {
        Self::new()
    }
}

impl Params {
    pub fn new() -> Self {
        Self { v: DEFS.iter().map(|d| AtomicF32::new(d.default)).collect() }
    }
    pub fn get(&self, p: P) -> f32 {
        self.v[p as usize].get()
    }
    pub fn at(&self, i: usize) -> f32 {
        self.v[i].get()
    }
    pub fn set_at(&self, i: usize, value: f32) {
        self.v[i].set(clamp_value(&DEFS[i], value));
    }
    pub fn set(&self, p: P, value: f32) {
        self.set_at(p as usize, value);
    }
    pub fn snapshot(&self) -> Snapshot {
        let mut s = [0.0; COUNT];
        for (o, a) in s.iter_mut().zip(&self.v) {
            *o = a.get();
        }
        s
    }
    pub fn load(&self, values: &[f32]) {
        for (i, v) in values.iter().enumerate().take(COUNT) {
            self.set_at(i, *v);
        }
    }
    pub fn reset_all(&self) {
        for (i, d) in DEFS.iter().enumerate() {
            self.v[i].set(d.default);
        }
    }
    pub fn reset_at(&self, i: usize) {
        self.v[i].set(DEFS[i].default);
    }
    pub fn norm_at(&self, i: usize) -> f32 {
        to_norm(&DEFS[i], self.at(i))
    }
    pub fn set_norm_at(&self, i: usize, x: f32) {
        self.set_at(i, from_norm(&DEFS[i], x.clamp(0.0, 1.0)));
    }
    /// Whether parameter `i` is a stepped choice rather than a continuous value.
    pub fn stepped(i: usize) -> bool {
        !matches!(DEFS[i].kind, Kind::Float { .. })
    }
    /// Moves parameter `i` by `clicks` encoder clicks: a continuous value by
    /// `sens` percent of its range per click, a choice by whole steps.
    pub fn step_at(&self, i: usize, clicks: i32, sens: f32) {
        if clicks == 0 {
            return;
        }
        let d: &Def = &DEFS[i];
        match d.kind {
            Kind::Float { .. } => {
                let x = to_norm(d, self.at(i)) + clicks as f32 * 0.01 * sens;
                self.set_at(i, from_norm(d, x.clamp(0.0, 1.0)));
            }
            Kind::Choice(names) => {
                let n = names.len() as i32;
                self.set_at(i, (self.at(i) as i32 + clicks.signum()).rem_euclid(n) as f32);
            }
            Kind::Toggle => self.set_at(i, if clicks > 0 { 1.0 } else { 0.0 }),
            Kind::Int { min, max } => self.set_at(i, (self.at(i) as i32 + clicks.signum()).clamp(min, max) as f32),
        }
    }
    pub fn text_at(&self, i: usize) -> String {
        format_value(&DEFS[i], self.at(i))
    }
}
