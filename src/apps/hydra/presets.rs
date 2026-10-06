//! Hydra's factory sounds, plus Randomize and Mutate.
//!
//! A preset is a list of (parameter index, value) overrides applied on top
//! of the initial patch, so each one only says what makes it that sound.

use super::dsp::Rng;
use super::params::*;
use super::store::{from_norm, to_norm, Params};

pub struct Preset {
    pub name: &'static str,
    pub values: Vec<(usize, f32)>,
}

impl Preset {
    /// Adds a mod-matrix slot (1..=12): `source` moves `target` by `amount`.
    fn m(mut self, slot: usize, source: usize, target: usize, amount: f32) -> Self {
        let base = P::M1_Src as usize + (slot - 1) * 3;
        self.values.extend([(base, source as f32), (base + 1, target as f32), (base + 2, amount)]);
        self
    }
}

macro_rules! pr {
    ($name:expr; $( $p:expr => $v:expr ),* $(,)?) => {
        Preset { name: $name, values: vec![ $( ($p as usize, $v as f32) ),* ] }
    };
}

const SINE: f32 = 0.0;
const TRI: f32 = 0.333;
const SAW: f32 = 0.667;
const PULSE: f32 = 1.0;

pub fn factory() -> Vec<Preset> {
    use P::*;
    vec![
        pr!("Init"; ),
        pr!("Super Saw";
            A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 7.0, Unison => 7, UniDetune => 0.5, UniSpread => 0.9,
            F1_Cut => 6500.0, F1_Res => 0.15, AmpA => 0.006, AmpD => 0.6, AmpS => 0.8, AmpR => 0.45,
            Cho_Mix => 0.25, Rev_Mix => 0.22, Rev_Size => 0.7, Rev_Decay => 2.5),
        pr!("Sub Bass";
            A_P1 => SINE, A_Level => 0.9, SubLevel => 0.6, F1_Cut => 900.0, F1_Res => 0.1, F1_Env => 0.25,
            E2A => 0.001, E2D => 0.2, E2S => 0.0, AmpA => 0.002, AmpD => 0.3, AmpS => 0.9, AmpR => 0.12,
            Mode => 1, Glide => 0.05, Drv_Type => 1, Drv_Amt => 0.25, Drv_Mix => 0.6),
        pr!("Acid Ladder";
            A_P1 => SAW, Mode => 1, Glide => 0.04, F1_Type => 0, F1_Cut => 14000.0, F2_Cut => 450.0, F2_Res => 0.8, F2_Env => 0.65, F2_Key => 0.4, F2_Drive => 0.4,
            E2A => 0.001, E2D => 0.25, E2S => 0.0, AmpD => 0.3, AmpS => 0.6, AmpR => 0.1, Drv_Type => 1, Drv_Amt => 0.3,
            Dly_Mix => 0.18, Dly_Time => 375.0, Dly_Fb => 0.4),
        pr!("Pluck Harp";
            A_Type => 3, A_P1 => 0.4, A_P2 => 0.8, A_P3 => 0.4, B_Type => 3, B_Level => 0.3, B_Coarse => 12, B_P1 => 0.5, B_P2 => 0.8, B_P3 => 0.35,
            AmpS => 1.0, AmpR => 1.2, F1_Cut => 12000.0, Cho_Mix => 0.2, Rev_Mix => 0.3, Rev_Decay => 3.5),
        pr!("FM Electric Piano";
            A_Type => 2, A_P1 => 0.2, A_P2 => 0.5, A_P3 => 0.1, A_Level => 0.7, B_Type => 2, B_Level => 0.3, B_P1 => 0.867, B_P2 => 0.35,
            AmpA => 0.002, AmpD => 2.2, AmpS => 0.0, AmpR => 0.5, VelSens => 0.8, Cho_Mix => 0.3, Cho_Depth => 0.4, Rev_Mix => 0.15)
            .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.3).m(2, SRC_VELOCITY, T_A_P1 + 1, 0.25),
        pr!("FM Bell";
            A_Type => 2, A_P1 => 0.533, A_P2 => 0.62, A_P3 => 0.05, A_Level => 0.6, B_Type => 2, B_Level => 0.35, B_P1 => 0.2, B_P2 => 0.3, B_Coarse => 12,
            AmpA => 0.001, AmpD => 4.0, AmpS => 0.0, AmpR => 2.0, Rev_Mix => 0.4, Rev_Decay => 5.0, Rev_Size => 0.8)
            .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.35),
        pr!("Warm Pad";
            A_P1 => 0.55, Unison => 3, UniDetune => 0.3, UniSpread => 0.8, B_Level => 0.5, B_P1 => SAW, B_Coarse => -12,
            F1_Type => 0, F1_Cut => 2500.0, F1_Res => 0.05, AmpA => 0.8, AmpD => 1.0, AmpS => 0.8, AmpR => 1.5,
            L1_Rate => 0.2, Cho_Mix => 0.4, Ph_Mix => 0.15, Rev_Mix => 0.5, Rev_Decay => 6.0)
            .m(1, SRC_LFO1, T_F1_CUT, 0.08),
        pr!("Wavetable Sweep";
            A_Type => 1, A_P1 => 0.5, A_P2 => 0.8, Unison => 3, UniDetune => 0.25, UniSpread => 0.7, F1_Cut => 9000.0,
            L2_Rate => 0.3, AmpA => 0.02, AmpR => 0.6, Dly_Mix => 0.2, Rev_Mix => 0.3)
            .m(1, SRC_LFO2, T_A_P1, 0.5),
        pr!("Vowel Choir";
            A_Type => 1, A_P1 => 0.5, A_P2 => 0.4, B_Type => 1, B_Level => 0.6, B_Fine => 6, B_P1 => 0.5, B_P2 => 0.4,
            L1_Rate => 0.15, AmpA => 0.6, AmpS => 0.9, AmpR => 1.2, F1_Cut => 7000.0, Cho_Mix => 0.3, Rev_Mix => 0.5, Rev_Decay => 5.0)
            .m(1, SRC_LFO1, T_A_P1, 0.5).m(2, SRC_LFO1, T_A_P1 + 3, 0.45),
        pr!("Drawbar Organ";
            A_Type => 1, A_P1 => 0.5, A_P2 => 0.2, B_Type => 1, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.6, B_P2 => 0.2,
            AmpA => 0.003, AmpD => 0.1, AmpS => 1.0, AmpR => 0.08, Cho_Mix => 0.3, Cho_Rate => 0.8, Ph_Mix => 0.2, Ph_Rate => 0.7, Drv_Type => 1, Drv_Amt => 0.2),
        pr!("Brass Stab";
            A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 8, Unison => 2, UniDetune => 0.2, F1_Cut => 700.0, F1_Res => 0.2, F1_Env => 0.55,
            E2A => 0.06, E2D => 0.35, E2S => 0.5, AmpA => 0.02, AmpD => 0.4, AmpS => 0.7, AmpR => 0.2, Drv_Type => 1, Drv_Amt => 0.25),
        pr!("Reese Bass";
            A_P1 => SAW, Unison => 3, UniDetune => 0.4, UniSpread => 0.2, SubLevel => 0.4, Mode => 1, F1_Cut => 500.0, F1_Res => 0.2,
            F2_Cut => 1400.0, F2_Res => 0.2, Route => 0, AmpD => 0.4, AmpS => 0.9, AmpR => 0.2, Drv_Type => 3, Drv_Amt => 0.25, Drv_Mix => 0.5)
            .m(1, SRC_LFO1, T_F1_CUT, 0.12),
        pr!("Wobble";
            A_P1 => SAW, SubLevel => 0.5, Mode => 1, F1_Cut => 400.0, F1_Res => 0.45, L1_Rate => 3.0, AmpS => 1.0, AmpR => 0.1, Drv_Type => 1, Drv_Amt => 0.35)
            .m(1, SRC_LFO1, T_F1_CUT, 0.6),
        pr!("Sync Lead";
            A_P1 => PULSE, A_P2 => 0.35, B_Level => 0.6, B_P1 => SAW, B_Coarse => 7, B_Sync => 1, Mode => 2, Glide => 0.06,
            F1_Cut => 5200.0, F1_Res => 0.25, F1_Env => 0.15, E2D => 0.4, AmpD => 0.3, AmpS => 0.8, AmpR => 0.2, Dly_Mix => 0.2, Dly_Time => 330.0, Rev_Mix => 0.2)
            .m(1, SRC_ENV3, T_PITCH_A + 1, 0.2).m(2, SRC_MOD_WHEEL, T_PITCH, 0.01),
        pr!("Glass Arp";
            A_Type => 1, A_P1 => 0.5, A_P2 => 1.0, AmpA => 0.002, AmpD => 0.25, AmpS => 0.2, AmpR => 0.3, Arp_On => 1, Arp_Mode => 2, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.4,
            Dly_Mix => 0.3, Dly_Time => 300.0, Rev_Mix => 0.35, Rev_Decay => 3.0),
        pr!("Noise Sweep";
            A_Type => 4, A_P1 => 0.3, B_Level => 0.0, F1_Type => 4, F1_Cut => 200.0, F1_Res => 0.55, F1_Env => 0.9, E2A => 3.0, E2D => 0.5, E2S => 1.0,
            AmpA => 2.5, AmpS => 1.0, AmpR => 1.5, Rev_Mix => 0.5, Rev_Decay => 5.0),
        pr!("Karplus Bass";
            A_Type => 3, A_Coarse => -12, A_P1 => 0.8, A_P2 => 0.7, A_P3 => 0.3, SubLevel => 0.3, AmpS => 1.0, AmpR => 0.3, F1_Cut => 6000.0, Mode => 1),
        pr!("808 Sub";
            A_P1 => SINE, AmpA => 0.001, AmpD => 0.7, AmpS => 0.0, AmpR => 0.2, E3A => 0.0005, E3D => 0.1, E3S => 0.0, Mode => 1, Drv_Type => 1, Drv_Amt => 0.2)
            .m(1, SRC_ENV3, T_PITCH, 0.35),
        pr!("Detuned Square";
            A_P1 => PULSE, A_P2 => 0.5, Unison => 5, UniDetune => 0.35, UniSpread => 0.8, F1_Type => 0, F1_Cut => 4000.0, AmpA => 0.01, AmpS => 0.9, Cho_Mix => 0.3),
        pr!("Ring Bell";
            A_P1 => SINE, B_Level => 0.7, B_P1 => SINE, B_Coarse => 7, B_Fine => 12, Ring => 0.8, A_Level => 0.4,
            AmpA => 0.001, AmpD => 3.0, AmpS => 0.0, AmpR => 1.5, Rev_Mix => 0.4, Rev_Decay => 4.5),
        pr!("Ambient Drone";
            A_P1 => TRI, B_Type => 1, B_Level => 0.5, B_P1 => 0.5, B_P2 => 1.0, C_Type => 4, C_Level => 0.1, C_P1 => 0.8,
            AmpA => 4.0, AmpS => 1.0, AmpR => 6.0, L2_Rate => 0.1, F1_Cut => 3500.0, Rev_Mix => 0.7, Rev_Decay => 12.0, Rev_Size => 0.9, Dly_Mix => 0.3, Dly_Fb => 0.6, Dly_Time => 600.0)
            .m(1, SRC_LFO2, T_A_P1 + 3, 0.5),
        pr!("Rave Stab";
            A_P1 => SAW, Unison => 5, UniDetune => 0.5, UniSpread => 0.8, F1_Cut => 1800.0, F1_Res => 0.3, F1_Env => 0.5,
            E2A => 0.002, E2D => 0.2, E2S => 0.1, AmpD => 0.3, AmpS => 0.0, AmpR => 0.2, Rev_Mix => 0.2, Dly_Mix => 0.25, Dly_Time => 250.0),
        pr!("Chiptune";
            A_P1 => PULSE, A_P2 => 0.25, AmpS => 1.0, AmpR => 0.05, Drv_Type => 4, Drv_Amt => 0.3, Drv_Mix => 1.0,
            Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Gate => 0.6, F1_Cut => 14000.0),
        pr!("String Ensemble";
            A_P1 => SAW, Unison => 3, UniDetune => 0.3, UniSpread => 0.7, B_Level => 0.5, B_P1 => SAW, B_Fine => -5,
            F1_Type => 0, F1_Cut => 3500.0, AmpA => 0.35, AmpS => 1.0, AmpR => 0.6, Cho_Mix => 0.6, Cho_Depth => 0.7, Ph_Mix => 0.2),
        pr!("Dub Chord";
            A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Coarse => 7, F1_Type => 0, F1_Cut => 1200.0, F1_Res => 0.3, AmpA => 0.01, AmpD => 0.4, AmpS => 0.5, AmpR => 0.3,
            Dly_Mix => 0.5, Dly_Time => 500.0, Dly_Fb => 0.6, Rev_Mix => 0.3),
        pr!("Digital Flute";
            A_Type => 1, A_P1 => 0.3, A_P2 => 0.6, AmpA => 0.08, AmpS => 0.9, AmpR => 0.25, F1_Cut => 9000.0, L1_Rate => 5.0, Rev_Mix => 0.25)
            .m(1, SRC_LFO1, T_PITCH, 0.006),
        pr!("Metal Hit";
            A_Type => 2, A_P1 => 0.8, A_P2 => 0.9, A_P3 => 0.3, B_Type => 4, B_Level => 0.3, B_P1 => 0.2, B_P3 => 0.5, Ring => 0.2,
            AmpD => 0.4, AmpS => 0.0, AmpR => 0.3, Rev_Mix => 0.3),
    ]
}

/// Loads a preset: the initial patch, then its overrides. Every value is
/// clamped by the table, so a preset can never leave a parameter out of range.
pub fn apply(params: &Params, preset: &Preset) {
    params.reset_all();
    for &(i, v) in &preset.values {
        params.set_at(i, v);
    }
}

/// A fresh random patch that is meant to be playable: a sounding oscillator,
/// a filter that isn't closed, a sustaining envelope and a few mod routings.
pub fn randomize(params: &Params, rng: &mut Rng) {
    params.reset_all();
    let r = |rng: &mut Rng, lo: f32, hi: f32| lo + rng.unit() * (hi - lo);
    let pick = |rng: &mut Rng, n: usize| (rng.unit() * n as f32) as usize % n;
    for (base, level) in [(P::A_Type as usize, 0.8f32), (P::B_Type as usize, 0.0), (P::C_Type as usize, 0.0)] {
        let on = base == P::A_Type as usize || rng.unit() < 0.6;
        params.set_at(base, pick(rng, 3) as f32);
        params.set_at(base + 1, if on { r(rng, 0.4, 0.9) * (level.max(0.5) / 0.8f32).min(1.0) } else { 0.0 });
        params.set_at(base + 2, [0.0, 0.0, -12.0, 7.0, 12.0][pick(rng, 5)]);
        params.set_at(base + 3, r(rng, -12.0, 12.0));
        params.set_at(base + 4, r(rng, 0.0, 1.0));
        params.set_at(base + 5, r(rng, 0.0, 1.0));
        params.set_at(base + 6, r(rng, 0.0, 0.5));
    }
    params.set(P::Unison, [1, 1, 3, 5][pick(rng, 4)] as f32);
    params.set(P::UniDetune, r(rng, 0.1, 0.5));
    params.set(P::SubLevel, if rng.unit() < 0.3 { r(rng, 0.2, 0.6) } else { 0.0 });
    params.set(P::F1_Type, pick(rng, 4) as f32);
    params.set(P::F1_Cut, 10.0f32.powf(r(rng, 2.6, 4.2)));
    params.set(P::F1_Res, r(rng, 0.05, 0.6));
    params.set(P::F1_Env, r(rng, -0.3, 0.7));
    params.set(P::E2D, r(rng, 0.1, 1.5));
    params.set(P::E2S, r(rng, 0.0, 0.6));
    params.set(P::AmpA, 10.0f32.powf(r(rng, -2.6, -0.3)));
    params.set(P::AmpD, r(rng, 0.2, 1.5));
    params.set(P::AmpS, r(rng, 0.4, 1.0));
    params.set(P::AmpR, r(rng, 0.1, 1.5));
    params.set(P::L1_Rate, 10.0f32.powf(r(rng, -1.0, 0.9)));
    params.set(P::L1_Shape, pick(rng, 6) as f32);
    for slot in 0..3 {
        let base = P::M1_Src as usize + slot * 3;
        params.set_at(base, (2 + pick(rng, 4)) as f32);
        params.set_at(base + 1, [T_A_P1, T_A_P1 + 1, T_F1_CUT, T_PITCH_A, T_A_P1 + 3, T_F1_RES][pick(rng, 6)] as f32);
        params.set_at(base + 2, r(rng, -0.5, 0.5));
    }
    params.set(P::Cho_Mix, if rng.unit() < 0.5 { r(rng, 0.1, 0.4) } else { 0.0 });
    params.set(P::Dly_Mix, if rng.unit() < 0.4 { r(rng, 0.1, 0.35) } else { 0.0 });
    params.set(P::Rev_Mix, r(rng, 0.0, 0.4));
}

/// Nudges a random handful of parameters a little, to evolve the patch.
pub fn mutate(params: &Params, rng: &mut Rng) {
    for (i, d) in DEFS.iter().enumerate() {
        // leave the performance setup, the sound's routing and the arp alone
        let skip = matches!(d.kind, Kind::Toggle | Kind::Int { .. } | Kind::Choice(_)) || [P::Level as usize, P::Tune as usize, P::ModWheel as usize].contains(&i) || (P::Mac1 as usize..=P::Mac4 as usize).contains(&i);
        if skip || rng.unit() > 0.18 {
            continue;
        }
        let x = to_norm(d, params.at(i)) + rng.bipolar() * 0.09;
        params.set_at(i, from_norm(d, x.clamp(0.0, 1.0)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_a_name_and_only_valid_parameters() {
        let presets = factory();
        assert!(presets.len() >= 24, "a real set of sounds: {}", presets.len());
        let mut names: Vec<_> = presets.iter().map(|p| p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), presets.len(), "names are unique");
        for p in &presets {
            for &(i, v) in &p.values {
                assert!(i < COUNT, "{}: index {i}", p.name);
                assert!(v.is_finite(), "{}: {}", p.name, DEFS[i].name);
            }
        }
    }

    #[test]
    fn applying_a_preset_resets_what_it_does_not_mention() {
        let params = Params::new();
        let presets = factory();
        apply(&params, &presets[1]);
        assert_eq!(params.get(P::Unison), 7.0);
        apply(&params, &presets[0]);
        assert_eq!(params.get(P::Unison), 1.0, "Init puts everything back");
        assert_eq!(params.get(P::F1_Cut), DEFS[P::F1_Cut as usize].default);
    }

    #[test]
    fn randomize_and_mutate_stay_in_range_and_keep_a_sounding_patch() {
        let params = Params::new();
        let mut rng = Rng::new(99);
        for _ in 0..50 {
            randomize(&params, &mut rng);
            assert!(params.get(P::A_Level) >= 0.3, "an oscillator is on");
            assert!(params.get(P::F1_Cut) >= 100.0, "the filter is not closed");
            assert!(params.get(P::AmpS) >= 0.4);
            for i in 0..COUNT {
                let v = params.at(i);
                assert!(v.is_finite());
                assert_eq!(super::super::store::clamp_value(&DEFS[i], v), v, "{}", DEFS[i].name);
            }
            mutate(&params, &mut rng);
        }
    }

    #[test]
    fn mutate_changes_some_continuous_parameters_but_not_the_setup() {
        let params = Params::new();
        let mut rng = Rng::new(3);
        let before = params.snapshot();
        for _ in 0..5 {
            mutate(&params, &mut rng);
        }
        let after = params.snapshot();
        assert!(before.iter().zip(&after).any(|(a, b)| a != b), "something moved");
        for p in [P::Voices, P::Mode, P::Arp_On, P::Unison, P::A_Type, P::Level] {
            assert_eq!(before[p as usize], after[p as usize], "{:?} is left alone", p);
        }
    }
}
