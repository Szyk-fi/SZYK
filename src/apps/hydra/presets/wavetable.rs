//! Wavetable: sounds that walk through Hydra's six table banks.

use super::*;

const MACROS: MacroSet = [
    ("Position", (MD_A1, 0.6), (MD_B1, 0.4)),
    ("Warp", (MD_A2, 0.4), (MD_DRIVE, 0.25)),
    ("Motion", (MD_LFO1, 0.45), (MD_LFO2, 0.4)),
    ("Space", (MD_REVERB, 0.3), (MD_DELAY, 0.25)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Wavetable",
        &MACROS,
        vec![
            pr!("Wavetable Sweep";
                A_Type => 1, A_P1 => 0.5, A_P2 => 0.8, Unison => 3, UniDetune => 0.25, UniSpread => 0.7, F1_Cut => 9000.0,
                L2_Rate => 0.3, AmpA => 0.02, AmpR => 0.6, Dly_Mix => 0.2, Rev_Mix => 0.3)
                .m(1, SRC_LFO2, T_A_P1, 0.5),
            pr!("Digital Flute";
                A_Type => 1, A_P1 => 0.3, A_P2 => 0.6, AmpA => 0.08, AmpS => 0.9, AmpR => 0.25, F1_Cut => 9000.0, L1_Rate => 5.0, Rev_Mix => 0.25)
                .m(1, SRC_LFO1, T_PITCH, 0.006),
            pr!("Basic Morph";
                A_Type => 1, A_P1 => 0.0, A_P2 => BASIC, Unison => 2, UniDetune => 0.15, F1_Cut => 8000.0, AmpA => 0.01, AmpS => 0.9, AmpR => 0.4, L1_Rate => 0.4, Rev_Mix => 0.25, Cho_Mix => 0.2)
                .m(1, SRC_LFO1, T_A_P1, 0.5),
            pr!("Organ Table";
                A_Type => 1, A_P1 => 0.3, A_P2 => ORGAN, B_Type => 1, B_Level => 0.4, B_P1 => 0.6, B_P2 => ORGAN, B_Coarse => 12, AmpA => 0.01, AmpS => 1.0, AmpR => 0.2, L1_Rate => 0.3,
                Cho_Mix => 0.3, Rev_Mix => 0.2)
                .m(1, SRC_LFO1, T_A_P1, 0.4),
            pr!("Vowel Table";
                A_Type => 1, A_P1 => 0.0, A_P2 => VOWEL, Unison => 2, UniDetune => 0.15, AmpA => 0.04, AmpS => 1.0, AmpR => 0.3, F1_Cut => 7000.0, L1_Rate => 0.25, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_A_P1, 0.8),
            pr!("Digital Cloud";
                A_Type => 1, A_P1 => 0.5, A_P2 => DIGITAL, B_Type => 1, B_Level => 0.6, B_P1 => 0.4, B_P2 => DIGITAL, B_Fine => 8.0, Unison => 3, UniDetune => 0.3, UniSpread => 0.9,
                AmpA => 0.4, AmpS => 1.0, AmpR => 1.2, F1_Cut => 10000.0, L1_Rate => 0.12, L2_Rate => 0.19, Cho_Mix => 0.3, Rev_Mix => 0.45, Rev_Decay => 5.0)
                .m(1, SRC_LFO1, T_A_P1, 0.6).m(2, SRC_LFO2, T_A_P1 + 3, 0.5),
            pr!("Sweep Table";
                A_Type => 1, A_P1 => 0.0, A_P2 => SWEEP, Unison => 3, UniDetune => 0.25, UniSpread => 0.8, AmpA => 0.02, AmpS => 0.9, AmpR => 0.5, F1_Cut => 11000.0, E3A => 1.5, E3D => 1.0, E3S => 1.0,
                Dly_Mix => 0.2, Rev_Mix => 0.3)
                .m(1, SRC_ENV3, T_A_P1, 0.9),
            pr!("Glass Harmonics";
                A_Type => 1, A_P1 => 0.6, A_P2 => GLASS, B_Type => 1, B_Level => 0.5, B_P1 => 0.3, B_P2 => GLASS, B_Coarse => 12, AmpA => 0.01, AmpD => 1.2, AmpS => 0.3, AmpR => 1.2, F1_Cut => 14000.0,
                Rev_Mix => 0.45, Rev_Decay => 5.0, Dly_Mix => 0.2)
                .m(1, SRC_AMP_ENV, T_A_P1, -0.3),
            pr!("Warped Table";
                A_Type => 1, A_P1 => 0.4, A_P2 => DIGITAL, A_P3 => 0.5, Unison => 2, UniDetune => 0.2, AmpA => 0.01, AmpS => 0.9, AmpR => 0.3, F1_Cut => 8000.0, L1_Rate => 0.5, Drv_Type => 3, Drv_Amt => 0.3)
                .m(1, SRC_LFO1, T_A_P1 + 2, 0.6),
            pr!("Metallic Table";
                A_Type => 1, A_P1 => 0.7, A_P2 => SWEEP, A_P3 => 0.7, B_Type => 2, B_Level => 0.3, B_P1 => FM_3_5, B_P2 => 0.4, Ring => 0.2, AmpA => 0.003, AmpD => 0.6, AmpS => 0.3, AmpR => 0.4,
                F1_Cut => 7000.0, Rev_Mix => 0.3),
            pr!("Growl Table";
                A_Type => 1, A_P1 => 0.3, A_P2 => VOWEL, A_P3 => 0.4, SubLevel => 0.3, Mode => 1, F1_Type => 1, F1_Cut => 2000.0, F1_Res => 0.3, L1_Rate => 4.0, AmpS => 1.0, AmpR => 0.1,
                Drv_Type => 2, Drv_Amt => 0.4, Drv_Mix => 0.7)
                .m(1, SRC_LFO1, T_A_P1, 0.6).m(2, SRC_LFO1, T_F1_CUT, 0.3),
            pr!("Evolving Texture";
                A_Type => 1, A_P1 => 0.2, A_P2 => SWEEP, B_Type => 1, B_Level => 0.6, B_P1 => 0.8, B_P2 => GLASS, B_Fine => 5.0, C_Type => 1, C_Level => 0.4, C_P1 => 0.5, C_P2 => VOWEL, C_Coarse => -12,
                AmpA => 2.0, AmpS => 1.0, AmpR => 3.0, F1_Cut => 6000.0, L1_Rate => 0.05, L2_Rate => 0.08, Rev_Mix => 0.55, Rev_Decay => 8.0, Cho_Mix => 0.3)
                .m(1, SRC_LFO1, T_A_P1, 0.7).m(2, SRC_LFO2, T_A_P1 + 3, 0.7).m(3, SRC_LFO1, T_A_P1 + 6, 0.5),
            pr!("Bell Table";
                A_Type => 1, A_P1 => 0.6, A_P2 => GLASS, B_Type => 2, B_Level => 0.3, B_P1 => FM_3_5, B_P2 => 0.3, AmpA => 0.001, AmpD => 2.0, AmpS => 0.0, AmpR => 1.5, Rev_Mix => 0.4, Rev_Decay => 5.0)
                .m(1, SRC_AMP_ENV, T_A_P1, -0.4),
            pr!("Vocal Table";
                A_Type => 1, A_P1 => 0.2, A_P2 => VOWEL, B_Type => 1, B_Level => 0.5, B_P1 => 0.5, B_P2 => VOWEL, B_Fine => 6.0, Mode => 2, Glide => 0.05, AmpA => 0.03, AmpS => 1.0, AmpR => 0.3,
                F1_Cut => 8000.0, Dly_Mix => 0.2, Rev_Mix => 0.3)
                .m(1, SRC_MOD_WHEEL, T_A_P1, 0.8).m(2, SRC_LFO2, T_A_P1 + 3, 0.2),
            pr!("Pluck Table";
                A_Type => 1, A_P1 => 0.8, A_P2 => DIGITAL, F1_Type => 1, F1_Cut => 2000.0, F1_Env => 0.5, E2D => 0.25, E2S => 0.0, E3D => 0.2, E3S => 0.0, AmpA => 0.001, AmpD => 0.4, AmpS => 0.0, AmpR => 0.3,
                Dly_Mix => 0.25, Rev_Mix => 0.25)
                .m(1, SRC_ENV3, T_A_P1, -0.6),
            pr!("Detuned Stack";
                A_Type => 1, A_P1 => 0.3, A_P2 => BASIC, B_Type => 1, B_Level => 0.8, B_P1 => 0.35, B_P2 => BASIC, B_Fine => 9.0, C_Type => 1, C_Level => 0.6, C_P1 => 0.25, C_P2 => ORGAN, C_Fine => -8.0,
                Unison => 3, UniDetune => 0.3, UniSpread => 0.9, AmpA => 0.01, AmpS => 0.9, AmpR => 0.5, F1_Cut => 9000.0, Cho_Mix => 0.3, Rev_Mix => 0.3),
        ],
    )
}
