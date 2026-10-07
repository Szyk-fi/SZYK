//! Brass: analog brass sections, stabs and solo horns.

use super::*;

const MACROS: MacroSet = [
    ("Blare", (MD_CUTOFF, 0.45), (MD_FENV, 0.3)),
    ("Bite", (MD_DRIVE, 0.5), (MD_RES, 0.1)),
    ("Section", (MD_DETUNE, 0.4), (MD_SPREAD, 0.3)),
    ("Hall", (MD_REVERB, 0.3), (MD_DELAY, 0.12)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Brass",
        &MACROS,
        vec![
            pr!("Brass Stab";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 8.0, Unison => 2, UniDetune => 0.2, F1_Cut => 700.0, F1_Res => 0.2, F1_Env => 0.55,
                E2A => 0.06, E2D => 0.35, E2S => 0.5, AmpA => 0.02, AmpD => 0.4, AmpS => 0.7, AmpR => 0.2, Drv_Type => 1, Drv_Amt => 0.25),
            pr!("Synth Brass";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 9.0, C_Level => 0.4, C_P1 => PULSE, C_P2 => 0.4, C_Coarse => -12, Unison => 3, UniDetune => 0.25,
                F1_Type => 1, F1_Cut => 600.0, F1_Res => 0.15, F1_Env => 0.6, E2A => 0.08, E2D => 0.4, E2S => 0.55, AmpA => 0.03, AmpD => 0.3, AmpS => 0.85, AmpR => 0.25,
                Cho_Mix => 0.3, Rev_Mix => 0.2),
            pr!("Brass Section";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 6.0, Unison => 5, UniDetune => 0.15, UniSpread => 0.7, F1_Type => 1, F1_Cut => 800.0, F1_Env => 0.5, E2A => 0.07, E2D => 0.5, E2S => 0.6,
                AmpA => 0.04, AmpS => 0.9, AmpR => 0.3, Drv_Type => 1, Drv_Amt => 0.2, Rev_Mix => 0.3, Rev_Decay => 2.5, Voices => 14),
            pr!("Soft Horn";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Fine => 4.0, Mode => 2, Glide => 0.04, F1_Type => 1, F1_Cut => 500.0, F1_Env => 0.4, E2A => 0.12, E2D => 0.5, E2S => 0.6,
                AmpA => 0.08, AmpS => 1.0, AmpR => 0.3, Rev_Mix => 0.3, L1_Rate => 5.0, L1_Fade => 0.8)
                .m(1, SRC_LFO1, T_PITCH, 0.003),
            pr!("Trumpet";
                A_P1 => SAW, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.3, Mode => 2, Glide => 0.03, F1_Type => 1, F1_Cut => 900.0, F1_Res => 0.15, F1_Env => 0.55, E2A => 0.04, E2D => 0.2, E2S => 0.6,
                AmpA => 0.025, AmpS => 1.0, AmpR => 0.2, Drv_Type => 1, Drv_Amt => 0.15, Rev_Mix => 0.25, L1_Rate => 5.5, L1_Fade => 1.0)
                .m(1, SRC_LFO1, T_PITCH, 0.003),
            pr!("Trombone";
                A_P1 => SAW, A_Coarse => -12, B_Level => 0.6, B_P1 => SAW, B_Coarse => -12, B_Fine => 5.0, Mode => 2, Glide => 0.1, F1_Type => 1, F1_Cut => 450.0, F1_Env => 0.5, E2A => 0.1, E2D => 0.4, E2S => 0.6,
                AmpA => 0.07, AmpS => 1.0, AmpR => 0.3, Rev_Mix => 0.3),
            pr!("Fanfare";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 7.0, Unison => 5, UniDetune => 0.2, UniSpread => 0.8, Chord => 2, F1_Type => 1, F1_Cut => 700.0, F1_Env => 0.6, E2A => 0.05, E2D => 0.5, E2S => 0.6,
                AmpA => 0.03, AmpS => 0.9, AmpR => 0.4, Rev_Mix => 0.4, Rev_Decay => 3.0, Voices => 12),
            pr!("Jump Brass";
                A_P1 => SAW, B_Level => 0.8, B_P1 => PULSE, B_P2 => 0.5, B_Fine => 8.0, Unison => 3, UniDetune => 0.2, F1_Type => 1, F1_Cut => 500.0, F1_Res => 0.2, F1_Env => 0.7, E2A => 0.02, E2D => 0.25, E2S => 0.3,
                AmpA => 0.01, AmpD => 0.3, AmpS => 0.7, AmpR => 0.15, Drv_Type => 1, Drv_Amt => 0.3),
            pr!("Bass Brass";
                A_P1 => SAW, A_Coarse => -12, B_Level => 0.8, B_P1 => SAW, B_Coarse => -24, SubLevel => 0.3, Mode => 1, F1_Type => 1, F1_Cut => 350.0, F1_Env => 0.5, E2A => 0.06, E2D => 0.3, E2S => 0.5,
                AmpA => 0.02, AmpS => 0.9, AmpR => 0.2, Drv_Type => 1, Drv_Amt => 0.25),
            pr!("Bright Brass";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 10.0, Unison => 3, UniDetune => 0.25, F1_Type => 1, F1_Cut => 1500.0, F1_Env => 0.5, E2A => 0.04, E2D => 0.3, E2S => 0.7,
                AmpA => 0.015, AmpS => 0.9, AmpR => 0.25, Drv_Type => 2, Drv_Amt => 0.2, Drv_Mix => 0.5, Cho_Mix => 0.2),
            pr!("Mellow Brass";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Fine => 5.0, Unison => 2, UniDetune => 0.15, F1_Type => 1, F1_Cut => 450.0, F1_Env => 0.35, E2A => 0.15, E2D => 0.6, E2S => 0.7,
                AmpA => 0.1, AmpS => 1.0, AmpR => 0.4, Cho_Mix => 0.25, Rev_Mix => 0.35),
            pr!("Sforzando";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 8.0, Unison => 3, UniDetune => 0.25, F1_Type => 1, F1_Cut => 1200.0, F1_Env => 0.4, E2A => 0.01, E2D => 0.6, E2S => 0.2,
                AmpA => 0.005, AmpD => 0.8, AmpS => 0.5, AmpR => 0.3, Drv_Type => 1, Drv_Amt => 0.3, Rev_Mix => 0.35),
            pr!("Pad Brass";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 7.0, Unison => 3, UniDetune => 0.3, F1_Type => 1, F1_Cut => 500.0, F1_Env => 0.5, E2A => 0.5, E2D => 1.0, E2S => 0.7,
                AmpA => 0.4, AmpS => 1.0, AmpR => 1.2, Cho_Mix => 0.4, Rev_Mix => 0.45),
            pr!("Tuba";
                A_P1 => SAW, A_Coarse => -12, B_Level => 0.7, B_P1 => TRI, B_Coarse => -24, Mode => 1, F1_Type => 1, F1_Cut => 300.0, F1_Env => 0.4, E2A => 0.08, E2D => 0.3, E2S => 0.6,
                AmpA => 0.05, AmpS => 1.0, AmpR => 0.2, Rev_Mix => 0.25),
            pr!("Solo Sax";
                A_P1 => SAW, A_P3 => 0.2, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.35, B_Fine => 3.0, Mode => 2, Glide => 0.05, F1_Type => 1, F1_Cut => 1400.0, F1_Res => 0.2, F1_Env => 0.3, E2A => 0.04, E2D => 0.3, E2S => 0.6,
                AmpA => 0.03, AmpS => 1.0, AmpR => 0.2, Drv_Type => 1, Drv_Amt => 0.2, L1_Rate => 5.0, L1_Fade => 1.0, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.003).m(2, SRC_MOD_WHEEL, T_F1_CUT, 0.2),
            pr!("Octave Brass";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Coarse => 12, C_Level => 0.6, C_P1 => SAW, C_Coarse => -12, Unison => 2, UniDetune => 0.15, F1_Type => 1, F1_Cut => 700.0, F1_Env => 0.5, E2A => 0.05, E2D => 0.4, E2S => 0.6,
                AmpA => 0.03, AmpS => 0.9, AmpR => 0.25, Drv_Type => 1, Drv_Amt => 0.2, Rev_Mix => 0.25),
        ],
    )
}
