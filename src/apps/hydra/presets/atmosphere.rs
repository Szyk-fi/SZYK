//! Atmosphere: drones, textures and slow-moving ambience.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.1)),
    ("Motion", (MD_LFO1, 0.45), (MD_LFO2, 0.4)),
    ("Texture", (MD_NOISE, 0.25), (MD_A1, 0.35)),
    ("Space", (MD_REVERB, 0.35), (MD_DELAY, 0.25)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Atmosphere",
        &MACROS,
        vec![
            pr!("Ambient Drone";
                A_P1 => TRI, B_Type => 1, B_Level => 0.5, B_P1 => 0.5, B_P2 => 1.0, C_Type => 4, C_Level => 0.1, C_P1 => 0.8,
                AmpA => 4.0, AmpS => 1.0, AmpR => 6.0, L2_Rate => 0.1, F1_Cut => 3500.0, Rev_Mix => 0.7, Rev_Decay => 12.0, Rev_Size => 0.9, Dly_Mix => 0.3, Dly_Fb => 0.6, Dly_Time => 600.0)
                .m(1, SRC_LFO2, T_A_P1 + 3, 0.5),
            pr!("Deep Space";
                A_P1 => SINE, A_Coarse => -12, B_Level => 0.5, B_P1 => TRI, B_Coarse => 7, B_Fine => 4.0, C_Type => 4, C_Level => 0.08, C_P1 => 0.9,
                AmpA => 5.0, AmpS => 1.0, AmpR => 8.0, F1_Type => 1, F1_Cut => 800.0, L1_Rate => 0.05, Rev_Mix => 0.8, Rev_Decay => 16.0, Rev_Size => 1.0, Dly_Mix => 0.35, Dly_Time => 900.0, Dly_Fb => 0.65)
                .m(1, SRC_LFO1, T_F1_CUT, 0.3),
            pr!("Dark Drone";
                A_P1 => SAW, A_Coarse => -24, B_Level => 0.8, B_P1 => SAW, B_Coarse => -24, B_Fine => 3.0, SubLevel => 0.5, F1_Type => 1, F1_Cut => 300.0, F1_Res => 0.3,
                AmpA => 3.0, AmpS => 1.0, AmpR => 5.0, L1_Rate => 0.06, Drv_Type => 1, Drv_Amt => 0.3, Rev_Mix => 0.5, Rev_Decay => 9.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.4),
            pr!("Shimmer";
                A_Type => 1, A_P1 => 0.5, A_P2 => GLASS, A_Coarse => 12, B_Type => 1, B_Level => 0.6, B_P1 => 0.55, B_P2 => GLASS, B_Coarse => 19, C_P1 => SINE, C_Level => 0.4, C_Coarse => 24,
                Unison => 3, UniDetune => 0.2, AmpA => 2.0, AmpS => 1.0, AmpR => 4.0, F1_Cut => 14000.0, Rev_Mix => 0.7, Rev_Decay => 12.0, Rev_Size => 0.9, Cho_Mix => 0.4, Dly_Mix => 0.3, Dly_Time => 700.0),
            pr!("Cave";
                A_Type => 4, A_P1 => 0.8, A_P2 => 0.1, B_P1 => SINE, B_Level => 0.4, B_Coarse => -12, F1_Type => 4, F1_Cut => 250.0, F1_Res => 0.5, AmpA => 3.0, AmpS => 1.0, AmpR => 5.0,
                L1_Rate => 0.07, L2_Rate => 0.04, Rev_Mix => 0.8, Rev_Decay => 15.0, Rev_Size => 1.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.35).m(2, SRC_LFO2, T_PITCH_A + 1, 0.0),
            pr!("Underwater";
                A_P1 => SINE, B_Type => 4, B_Level => 0.3, B_P1 => 0.9, F1_Type => 1, F1_Cut => 500.0, F1_Res => 0.4, AmpA => 2.0, AmpS => 1.0, AmpR => 4.0, L1_Rate => 0.3, L2_Rate => 0.13,
                Cho_Mix => 0.6, Cho_Rate => 0.3, Cho_Depth => 0.8, Rev_Mix => 0.6, Rev_Decay => 8.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.4).m(2, SRC_LFO2, T_PITCH, 0.0015),
            pr!("Wind";
                A_Type => 4, A_P1 => 0.5, A_P2 => 0.0, F1_Type => 4, F1_Cut => 700.0, F1_Res => 0.45, AmpA => 2.5, AmpS => 1.0, AmpR => 3.0, L1_Rate => 0.15, L2_Rate => 0.09, L1_Shape => 5, L2_Shape => 5,
                Rev_Mix => 0.45, Rev_Decay => 5.0, Ph_Mix => 0.3)
                .m(1, SRC_LFO1, T_F1_CUT, 0.55).m(2, SRC_LFO2, T_PAN, 0.8),
            pr!("Crackling Fire";
                A_Type => 4, A_P1 => 0.3, A_P2 => 0.9, A_P3 => 0.0, F1_Type => 0, F1_Cut => 4500.0, AmpA => 1.5, AmpS => 1.0, AmpR => 2.0, Rev_Mix => 0.3),
            pr!("Cosmic Wash";
                A_Type => 1, A_P1 => 0.3, A_P2 => SWEEP, B_P1 => SAW, B_Level => 0.4, B_Fine => 6.0, Unison => 3, UniDetune => 0.35, UniSpread => 1.0, AmpA => 3.0, AmpS => 1.0, AmpR => 5.0,
                F1_Cut => 4000.0, L1_Rate => 0.06, L2_Rate => 0.1, Ph_Mix => 0.4, Ph_Rate => 0.1, Rev_Mix => 0.7, Rev_Decay => 12.0, Dly_Mix => 0.3, Dly_Time => 800.0, Dly_Fb => 0.6)
                .m(1, SRC_LFO1, T_A_P1, 0.6).m(2, SRC_LFO2, T_F1_CUT, 0.3),
            pr!("Ghost";
                A_Type => 1, A_P1 => 0.5, A_P2 => VOWEL, B_Type => 4, B_Level => 0.2, B_P1 => 0.4, AmpA => 2.5, AmpS => 1.0, AmpR => 4.0, F1_Type => 0, F1_Cut => 3500.0, L1_Rate => 0.12,
                Rev_Mix => 0.7, Rev_Decay => 10.0, Dly_Mix => 0.35, Dly_Fb => 0.6, Dly_Time => 650.0, Drift => 0.7)
                .m(1, SRC_LFO1, T_A_P1, 0.5),
            pr!("Industrial Hum";
                A_P1 => SAW, A_Coarse => -24, B_P1 => PULSE, B_P2 => 0.3, B_Level => 0.6, B_Coarse => -17, B_Fine => 2.0, Ring => 0.3, F1_Type => 1, F1_Cut => 600.0, F1_Res => 0.35,
                AmpA => 1.0, AmpS => 1.0, AmpR => 2.0, Drv_Type => 2, Drv_Amt => 0.5, Rev_Mix => 0.3, L1_Rate => 0.5)
                .m(1, SRC_LFO1, T_F1_CUT, 0.2),
            pr!("Singing Bowl";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.25, B_Type => 2, B_Level => 0.5, B_P1 => FM_2, B_P2 => 0.3, B_Fine => 4.0, AmpA => 0.05, AmpD => 8.0, AmpS => 0.0, AmpR => 6.0,
                L1_Rate => 4.5, Rev_Mix => 0.6, Rev_Decay => 9.0)
                .m(1, SRC_LFO1, T_AMP, -0.25),
            pr!("Aurora";
                A_Type => 1, A_P1 => 0.5, A_P2 => GLASS, B_P1 => SAW, B_Level => 0.4, B_Coarse => 12, Unison => 5, UniDetune => 0.4, UniSpread => 1.0, AmpA => 3.5, AmpS => 1.0, AmpR => 5.0,
                F1_Type => 0, F1_Cut => 6000.0, L1_Rate => 0.08, Cho_Mix => 0.5, Rev_Mix => 0.7, Rev_Decay => 12.0, Dly_Mix => 0.3, Dly_Time => 750.0)
                .m(1, SRC_LFO1, T_A_P1, 0.5).m(2, SRC_LFO1, T_F1_CUT, 0.2),
            pr!("Cathedral Air";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_P1 => SINE, B_Level => 0.5, B_Coarse => 12, C_Type => 4, C_Level => 0.06, C_P1 => 0.6, AmpA => 2.5, AmpS => 1.0, AmpR => 5.0, F1_Cut => 5000.0,
                Rev_Mix => 0.85, Rev_Decay => 18.0, Rev_Size => 1.0),
            pr!("Fog";
                A_P1 => TRI, A_Coarse => -12, B_Type => 4, B_Level => 0.4, B_P1 => 0.85, F1_Type => 1, F1_Cut => 420.0, F1_Res => 0.2, AmpA => 3.0, AmpS => 1.0, AmpR => 5.0, L1_Rate => 0.09,
                Cho_Mix => 0.4, Rev_Mix => 0.6, Rev_Decay => 10.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.3),
            pr!("Tension";
                A_P1 => SAW, A_Coarse => -12, B_Level => 0.7, B_P1 => SAW, B_Coarse => -12, B_Fine => 18.0, C_Level => 0.4, C_P1 => PULSE, C_P2 => 0.3, C_Coarse => -5, F1_Type => 1, F1_Cut => 700.0, F1_Res => 0.4,
                AmpA => 4.0, AmpS => 1.0, AmpR => 4.0, L1_Rate => 0.2, Rev_Mix => 0.5, Rev_Decay => 8.0, Drv_Type => 1, Drv_Amt => 0.25)
                .m(1, SRC_LFO1, T_F1_CUT, 0.4).m(2, SRC_ENV3, T_PITCH_A + 2, 0.015),
        ],
    )
}
