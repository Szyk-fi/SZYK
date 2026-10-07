//! Strings: ensembles and solo bowed voices, from analog string machines.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.05)),
    ("Bow", (MD_ATTACK, 0.5), (MD_A2, 0.3)),
    ("Ensemble", (MD_CHORUS, 0.4), (MD_DETUNE, 0.35)),
    ("Hall", (MD_REVERB, 0.4), (MD_RDECAY, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Strings",
        &MACROS,
        vec![
            pr!("String Ensemble";
                A_P1 => SAW, Unison => 3, UniDetune => 0.3, UniSpread => 0.7, B_Level => 0.5, B_P1 => SAW, B_Fine => -5.0,
                F1_Type => 0, F1_Cut => 3500.0, AmpA => 0.35, AmpS => 1.0, AmpR => 0.6, Cho_Mix => 0.6, Cho_Depth => 0.7, Ph_Mix => 0.2),
            pr!("Solina";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Coarse => 12, B_Fine => 4.0, Unison => 3, UniDetune => 0.2, F1_Type => 0, F1_Cut => 2800.0, AmpA => 0.25, AmpS => 1.0, AmpR => 0.5,
                Cho_Mix => 0.7, Cho_Rate => 0.5, Cho_Depth => 0.8, Ph_Mix => 0.25, Ph_Rate => 0.4),
            pr!("Violins";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 7.0, Unison => 4, UniDetune => 0.22, UniSpread => 0.8, F1_Type => 0, F1_Cut => 5500.0, F1_Env => 0.15, E2A => 0.15,
                AmpA => 0.2, AmpS => 1.0, AmpR => 0.45, L1_Rate => 5.2, L1_Fade => 0.5, Cho_Mix => 0.3, Rev_Mix => 0.35, Rev_Decay => 3.0)
                .m(1, SRC_LFO1, T_PITCH, 0.003),
            pr!("Cellos";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 5.0, A_Coarse => -12, B_Coarse => -12, Unison => 3, UniDetune => 0.2, F1_Type => 0, F1_Cut => 1800.0, F1_Env => 0.2, E2A => 0.12,
                AmpA => 0.2, AmpS => 1.0, AmpR => 0.5, L1_Rate => 4.8, L1_Fade => 0.6, Cho_Mix => 0.2, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.0025),
            pr!("Chamber Strings";
                A_P1 => SAW, B_Level => 0.6, B_P1 => TRI, B_Coarse => 12, Unison => 3, UniDetune => 0.18, F1_Type => 0, F1_Cut => 3200.0, AmpA => 0.3, AmpS => 1.0, AmpR => 0.6,
                Cho_Mix => 0.3, Rev_Mix => 0.4, Rev_Decay => 3.5),
            pr!("Tremolo Strings";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 6.0, Unison => 3, UniDetune => 0.2, F1_Type => 0, F1_Cut => 4000.0, AmpA => 0.12, AmpS => 1.0, AmpR => 0.3,
                L1_Shape => 1, L1_Rate => 9.0, Rev_Mix => 0.35)
                .m(1, SRC_LFO1, T_AMP, -0.4),
            pr!("Slow Strings";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 7.0, Unison => 3, UniDetune => 0.25, F1_Type => 0, F1_Cut => 2400.0, AmpA => 1.4, AmpS => 1.0, AmpR => 1.8,
                Cho_Mix => 0.5, Rev_Mix => 0.45, Rev_Decay => 5.0),
            pr!("Synth Strings";
                A_P1 => PULSE, A_P2 => 0.45, B_Level => 0.6, B_P1 => SAW, B_Fine => 6.0, Unison => 3, UniDetune => 0.3, UniSpread => 0.8, F1_Type => 0, F1_Cut => 4500.0, AmpA => 0.3, AmpS => 1.0, AmpR => 0.7,
                Cho_Mix => 0.55, Ph_Mix => 0.3, Rev_Mix => 0.3)
                .m(1, SRC_LFO2, T_A_P1 + 1, 0.2),
            pr!("Mellow Strings";
                A_P1 => TRI, B_Level => 0.7, B_P1 => SAW, B_Fine => 5.0, Unison => 3, UniDetune => 0.2, F1_Type => 1, F1_Cut => 1600.0, AmpA => 0.45, AmpS => 1.0, AmpR => 0.8,
                Cho_Mix => 0.4, Rev_Mix => 0.4),
            pr!("Warm Ensemble";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Coarse => -12, B_Fine => 5.0, C_Level => 0.4, C_P1 => PULSE, C_P2 => 0.4, Unison => 3, UniDetune => 0.25, F1_Type => 0, F1_Cut => 2400.0,
                AmpA => 0.4, AmpS => 1.0, AmpR => 0.8, Cho_Mix => 0.55, Cho_Depth => 0.6, Rev_Mix => 0.35, Drift => 0.3),
            pr!("Bright Ensemble";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Coarse => 12, B_Fine => 6.0, Unison => 5, UniDetune => 0.3, UniSpread => 0.9, F1_Type => 0, F1_Cut => 9000.0, AmpA => 0.2, AmpS => 1.0, AmpR => 0.5,
                Cho_Mix => 0.5, Rev_Mix => 0.3),
            pr!("Octave Strings";
                A_P1 => SAW, A_Coarse => -12, B_Level => 0.7, B_P1 => SAW, B_Coarse => 12, C_Level => 0.5, C_P1 => SAW, C_Fine => 6.0, Unison => 3, UniDetune => 0.2, F1_Type => 0, F1_Cut => 4000.0,
                AmpA => 0.3, AmpS => 1.0, AmpR => 0.6, Cho_Mix => 0.45, Rev_Mix => 0.35),
            pr!("Solo Violin";
                A_P1 => SAW, B_Level => 0.4, B_P1 => TRI, B_Fine => 3.0, Mode => 2, Glide => 0.06, F1_Type => 0, F1_Cut => 4500.0, F1_Env => 0.2, E2A => 0.1,
                AmpA => 0.08, AmpS => 1.0, AmpR => 0.3, L1_Rate => 5.5, L1_Fade => 0.7, Rev_Mix => 0.35, Rev_Decay => 3.0)
                .m(1, SRC_LFO1, T_PITCH, 0.004),
            pr!("Staccato Strings";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 6.0, Unison => 3, UniDetune => 0.2, F1_Type => 0, F1_Cut => 3500.0, F1_Env => 0.3, E2D => 0.15, E2S => 0.2,
                AmpA => 0.005, AmpD => 0.2, AmpS => 0.0, AmpR => 0.1, Cho_Mix => 0.3, Rev_Mix => 0.3),
            pr!("Swell Strings";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 7.0, Unison => 3, UniDetune => 0.25, F1_Type => 1, F1_Cut => 500.0, F1_Env => 0.65, E2A => 2.0, E2D => 1.0, E2S => 0.8,
                AmpA => 1.8, AmpS => 1.0, AmpR => 1.5, Cho_Mix => 0.4, Rev_Mix => 0.4),
            pr!("Orchestra Hit";
                A_P1 => SAW, B_Level => 0.8, B_P1 => PULSE, B_P2 => 0.5, B_Coarse => 7, C_Level => 0.6, C_P1 => SAW, C_Coarse => 12, Unison => 3, UniDetune => 0.3, Chord => 2,
                F1_Type => 0, F1_Cut => 5000.0, AmpA => 0.001, AmpD => 0.5, AmpS => 0.0, AmpR => 0.4, Drv_Type => 1, Drv_Amt => 0.3, Rev_Mix => 0.3, Voices => 12),
        ],
    )
}
