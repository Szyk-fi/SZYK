//! Pad: slow, wide, sustained sounds to hold chords under.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.08)),
    ("Motion", (MD_LFO1, 0.4), (MD_A1, 0.35)),
    ("Swell", (MD_ATTACK, 0.55), (MD_RELEASE, 0.35)),
    ("Space", (MD_REVERB, 0.35), (MD_CHORUS, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Pad",
        &MACROS,
        vec![
            pr!("Warm Pad";
                A_P1 => 0.55, Unison => 3, UniDetune => 0.3, UniSpread => 0.8, B_Level => 0.5, B_P1 => SAW, B_Coarse => -12,
                F1_Type => 0, F1_Cut => 2500.0, F1_Res => 0.05, AmpA => 0.8, AmpD => 1.0, AmpS => 0.8, AmpR => 1.5,
                L1_Rate => 0.2, Cho_Mix => 0.4, Ph_Mix => 0.15, Rev_Mix => 0.5, Rev_Decay => 6.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.08),
            pr!("PWM Pad";
                A_P1 => PULSE, A_P2 => 0.5, B_Level => 0.6, B_P1 => PULSE, B_Fine => 6.0, Unison => 3, UniDetune => 0.25, UniSpread => 0.8,
                F1_Type => 0, F1_Cut => 3200.0, AmpA => 0.5, AmpS => 0.9, AmpR => 1.2, L1_Rate => 0.4, L2_Rate => 0.27, Cho_Mix => 0.35, Rev_Mix => 0.35, Rev_Decay => 4.0)
                .m(1, SRC_LFO1, T_A_P1 + 1, 0.35).m(2, SRC_LFO2, T_A_P1 + 4, 0.3),
            pr!("Soft Pad";
                A_P1 => TRI, B_Level => 0.5, B_P1 => SINE, B_Coarse => 12, Unison => 2, UniDetune => 0.15, F1_Type => 0, F1_Cut => 1800.0,
                AmpA => 1.2, AmpS => 1.0, AmpR => 2.0, Cho_Mix => 0.3, Rev_Mix => 0.45, Rev_Decay => 5.0),
            pr!("Dark Pad";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Coarse => -12, B_Fine => 5.0, Unison => 3, UniDetune => 0.3, F1_Type => 1, F1_Cut => 900.0, F1_Res => 0.15,
                AmpA => 1.0, AmpS => 1.0, AmpR => 2.5, L1_Rate => 0.1, Rev_Mix => 0.45, Rev_Decay => 7.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.15),
            pr!("Bright Pad";
                A_P1 => SAW, B_Level => 0.6, B_P1 => PULSE, B_P2 => 0.4, B_Coarse => 12, Unison => 5, UniDetune => 0.35, UniSpread => 0.9, F1_Type => 0, F1_Cut => 9000.0,
                AmpA => 0.35, AmpS => 0.9, AmpR => 1.0, Cho_Mix => 0.35, Rev_Mix => 0.3, Rev_Decay => 3.5),
            pr!("Glass Pad";
                A_Type => 1, A_P1 => 0.4, A_P2 => GLASS, B_Type => 1, B_Level => 0.5, B_P1 => 0.6, B_P2 => GLASS, B_Coarse => 12, Unison => 3, UniDetune => 0.2, UniSpread => 0.8,
                AmpA => 0.7, AmpS => 0.9, AmpR => 2.0, F1_Cut => 11000.0, L2_Rate => 0.15, Rev_Mix => 0.5, Rev_Decay => 6.0, Cho_Mix => 0.25)
                .m(1, SRC_LFO2, T_A_P1, 0.4),
            pr!("Evolving Pad";
                A_Type => 1, A_P1 => 0.2, A_P2 => SWEEP, B_Type => 1, B_Level => 0.6, B_P1 => 0.7, B_P2 => DIGITAL, B_Fine => 7.0, Unison => 3, UniDetune => 0.25,
                AmpA => 1.5, AmpS => 1.0, AmpR => 3.0, F1_Cut => 5000.0, L1_Rate => 0.07, L2_Rate => 0.11, Rev_Mix => 0.5, Rev_Decay => 8.0, Cho_Mix => 0.3)
                .m(1, SRC_LFO1, T_A_P1, 0.6).m(2, SRC_LFO2, T_A_P1 + 3, 0.6),
            pr!("Dream Pad";
                A_P1 => SINE, B_Level => 0.6, B_P1 => TRI, B_Coarse => 12, B_Fine => 8.0, Unison => 3, UniDetune => 0.2, F1_Cut => 4000.0, AmpA => 1.5, AmpS => 1.0, AmpR => 3.0,
                Cho_Mix => 0.6, Cho_Depth => 0.7, Ph_Mix => 0.3, Rev_Mix => 0.6, Rev_Decay => 9.0, Rev_Size => 0.9),
            pr!("Swell Pad";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 8.0, Unison => 3, UniDetune => 0.3, F1_Type => 1, F1_Cut => 400.0, F1_Env => 0.7, E2A => 3.5, E2D => 1.0, E2S => 0.8,
                AmpA => 2.8, AmpS => 1.0, AmpR => 2.0, Rev_Mix => 0.4, Rev_Decay => 5.0, Cho_Mix => 0.3),
            pr!("Tape Pad";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Fine => -6.0, Drift => 0.6, Unison => 2, UniDetune => 0.2, F1_Type => 0, F1_Cut => 1500.0, AmpA => 0.9, AmpS => 0.9, AmpR => 1.5,
                L1_Rate => 0.5, Cho_Mix => 0.5, Cho_Rate => 0.25, Drv_Type => 1, Drv_Amt => 0.2, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.0015),
            pr!("Pulsing Pad";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 7.0, Unison => 3, UniDetune => 0.3, F1_Type => 0, F1_Cut => 2500.0, AmpA => 0.4, AmpS => 0.8, AmpR => 1.0,
                L1_Shape => 1, L1_Rate => 2.0, Rev_Mix => 0.35, Dly_Mix => 0.25, Dly_Time => 375.0)
                .m(1, SRC_LFO1, T_AMP, -0.35).m(2, SRC_LFO1, T_F1_CUT, 0.15),
            pr!("Stacked Fifths";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Fine => 5.0, Unison => 3, UniDetune => 0.25, Chord => 2, F1_Type => 0, F1_Cut => 3000.0, AmpA => 0.5, AmpS => 0.9, AmpR => 1.2,
                Cho_Mix => 0.35, Rev_Mix => 0.4, Rev_Decay => 4.5, Voices => 12),
            pr!("Minor Seventh Pad";
                A_P1 => TRI, B_Level => 0.6, B_P1 => SAW, B_Fine => 6.0, Unison => 2, UniDetune => 0.2, Chord => 8, F1_Type => 0, F1_Cut => 2200.0, AmpA => 0.6, AmpS => 0.9, AmpR => 1.5,
                Cho_Mix => 0.4, Rev_Mix => 0.45, Rev_Decay => 5.0, Voices => 14),
            pr!("Scale Chord Pad";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Fine => 5.0, Unison => 2, UniDetune => 0.2, Scale => 1, Chord => 16, F1_Type => 0, F1_Cut => 2800.0, AmpA => 0.35, AmpS => 0.9, AmpR => 1.0,
                Cho_Mix => 0.35, Rev_Mix => 0.4, Rev_Decay => 4.0, Voices => 14),
            pr!("Ice Pad";
                A_Type => 2, A_P1 => FM_3_5, A_P2 => 0.25, A_P3 => 0.1, B_Type => 2, B_Level => 0.4, B_P1 => FM_2, B_P2 => 0.2, B_Fine => 6.0, AmpA => 0.8, AmpS => 0.9, AmpR => 2.5,
                F1_Cut => 9000.0, Cho_Mix => 0.35, Rev_Mix => 0.55, Rev_Decay => 7.0, Dly_Mix => 0.2, Dly_Time => 450.0)
                .m(1, SRC_LFO2, T_A_P1 + 1, 0.25),
            pr!("Air Pad";
                A_P1 => SAW, A_Level => 0.5, B_Type => 4, B_Level => 0.25, B_P1 => 0.35, Unison => 3, UniDetune => 0.25, F1_Type => 0, F1_Cut => 5000.0, AmpA => 1.2, AmpS => 1.0, AmpR => 2.5,
                L1_Rate => 0.15, Cho_Mix => 0.4, Rev_Mix => 0.5, Rev_Decay => 6.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.15),
            pr!("Hybrid Pad";
                A_Type => 1, A_P1 => 0.3, A_P2 => ORGAN, B_P1 => SAW, B_Level => 0.6, B_Coarse => -12, C_Type => 2, C_Level => 0.25, C_P1 => FM_3, C_P2 => 0.2,
                Unison => 3, UniDetune => 0.25, F1_Type => 0, F1_Cut => 3500.0, AmpA => 0.9, AmpS => 1.0, AmpR => 2.0, L1_Rate => 0.12, Rev_Mix => 0.45, Cho_Mix => 0.3)
                .m(1, SRC_LFO1, T_F1_CUT, 0.12),
        ],
    )
}
