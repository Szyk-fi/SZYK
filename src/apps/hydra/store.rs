//! The live parameter values, shared between the UI thread and the audio
//! thread without a lock, and the conversions the table's kinds need:
//! normalised position, display text, stepping and defaults.

use super::params::{Def, Kind, Unit, COUNT, DEFS, P};
use crate::util::AtomicF32;

/// A copy of every value at one instant, which is what the audio thread
/// renders a block from (so a block never sees half an edit).
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

    /// Sets parameter `i`, clamped and (for steps) rounded to what it allows.
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

    /// 0..1 position of parameter `i`.
    pub fn norm_at(&self, i: usize) -> f32 {
        to_norm(&DEFS[i], self.at(i))
    }

    pub fn set_norm_at(&self, i: usize, x: f32) {
        self.set_at(i, from_norm(&DEFS[i], x.clamp(0.0, 1.0)));
    }

    /// Whether parameter `i` is a stepped choice (a mode, a switch) rather
    /// than a continuous value.
    pub fn stepped(i: usize) -> bool {
        !matches!(DEFS[i].kind, Kind::Float { .. })
    }

    /// Moves parameter `i` by `clicks` encoder clicks: a continuous value by
    /// `sens` percent of its range per click, a choice by whole steps (wrapping
    /// for choices, stopping at the ends for numbers).
    pub fn step_at(&self, i: usize, clicks: i32, sens: f32) {
        if clicks == 0 {
            return;
        }
        let d = &DEFS[i];
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

pub fn clamp_value(d: &Def, v: f32) -> f32 {
    let v = if v.is_finite() { v } else { d.default };
    match d.kind {
        Kind::Float { min, max, .. } => v.clamp(min, max),
        Kind::Choice(names) => v.round().clamp(0.0, names.len() as f32 - 1.0),
        Kind::Toggle => (v >= 0.5) as u8 as f32,
        Kind::Int { min, max } => v.round().clamp(min as f32, max as f32),
    }
}

pub fn to_norm(d: &Def, v: f32) -> f32 {
    let span = |lo: f32, hi: f32| if hi > lo { ((v - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
    match d.kind {
        Kind::Float { min, max, exp: false, .. } => span(min, max),
        Kind::Float { min, max, exp: true, .. } => ((v.max(min) / min).ln() / (max / min).ln()).clamp(0.0, 1.0),
        Kind::Choice(names) => span(0.0, names.len() as f32 - 1.0),
        Kind::Toggle => v.clamp(0.0, 1.0),
        Kind::Int { min, max } => span(min as f32, max as f32),
    }
}

pub fn from_norm(d: &Def, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    match d.kind {
        Kind::Float { min, max, exp: false, .. } => min + (max - min) * x,
        Kind::Float { min, max, exp: true, .. } => min * (max / min).powf(x),
        Kind::Choice(names) => (x * (names.len() as f32 - 1.0)).round(),
        Kind::Toggle => (x >= 0.5) as u8 as f32,
        Kind::Int { min, max } => (min as f32 + x * (max - min) as f32).round(),
    }
}

pub fn format_value(d: &Def, v: f32) -> String {
    match d.kind {
        Kind::Choice(names) => names[(v.round().max(0.0) as usize).min(names.len() - 1)].to_string(),
        Kind::Toggle => if v >= 0.5 { "on" } else { "off" }.to_string(),
        Kind::Int { .. } => format!("{:+}", v.round() as i32).trim_start_matches('+').to_string(),
        Kind::Float { unit, .. } => match unit {
            Unit::None => format!("{v:.2}"),
            Unit::Percent => format!("{:.0}%", v * 100.0),
            Unit::Signed => format!("{:+.0}%", v * 100.0),
            Unit::Hz => {
                if v >= 1000.0 {
                    format!("{:.2} kHz", v / 1000.0)
                } else if v >= 100.0 {
                    format!("{v:.0} Hz")
                } else {
                    format!("{v:.2} Hz")
                }
            }
            Unit::Seconds => {
                if v < 1.0 {
                    format!("{:.0} ms", v * 1000.0)
                } else {
                    format!("{v:.2} s")
                }
            }
            Unit::Ms => format!("{v:.0} ms"),
            Unit::Cents => format!("{v:+.0} ct"),
            Unit::Bpm => format!("{v:.0} BPM"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_inside_its_own_range() {
        for (i, d) in DEFS.iter().enumerate() {
            assert_eq!(clamp_value(d, d.default), d.default, "{} ({i}) default {} is out of range", d.name, d.default);
        }
    }

    #[test]
    fn position_round_trips_for_every_parameter() {
        for (i, d) in DEFS.iter().enumerate() {
            for x in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let v = from_norm(d, x);
                let back = to_norm(d, v);
                let again = from_norm(d, back);
                assert!((again - v).abs() <= 1e-3 * v.abs().max(1.0), "{} ({i}): {x} -> {v} -> {back} -> {again}", d.name);
            }
        }
    }

    #[test]
    fn stepping_a_choice_wraps_and_a_number_stops() {
        let p = Params::new();
        let mode = P::Mode as usize;
        p.step_at(mode, -1, 1.0);
        assert_eq!(p.at(mode), 2.0, "choices wrap");
        let voices = P::Voices as usize;
        for _ in 0..40 {
            p.step_at(voices, 1, 1.0);
        }
        assert_eq!(p.at(voices), 16.0, "numbers stop at the end");
    }

    #[test]
    fn a_continuous_value_moves_by_the_sensitivity_percent_of_its_range() {
        let p = Params::new();
        let cut = P::F1_Cut as usize;
        let before = p.norm_at(cut);
        p.step_at(cut, 5, 1.0);
        assert!((p.norm_at(cut) - before - 0.05).abs() < 1e-4);
    }

    #[test]
    fn text_is_readable() {
        let p = Params::new();
        assert_eq!(p.text_at(P::F1_Cut as usize), "8.00 kHz");
        assert_eq!(p.text_at(P::Level as usize), "70%");
        assert_eq!(p.text_at(P::Mode as usize), "Poly");
        assert_eq!(p.text_at(P::AmpR as usize), "250 ms");
    }
}
