//! Bells: struck metal, wood and glass, from FM and ring modulation.

use super::*;

const MACROS: MacroSet = [
    ("Strike", (MD_A2, 0.35), (MD_B2, 0.3)),
    ("Ring", (MD_DECAY, 0.5), (MD_RELEASE, 0.45)),
    ("Metal", (MD_RING, 0.4), (MD_CLEV, 0.3)),
    ("Space", (MD_REVERB, 0.35), (MD_DELAY, 0.2)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Bells",
        &MACROS,
        vec![
            pr!("FM Bell";
                A_Type => 2, A_P1 => 0.533, A_P2 => 0.62, A_P3 => 0.05, A_Level => 0.6, B_Type => 2, B_Level => 0.35, B_P1 => 0.2, B_P2 => 0.3, B_Coarse => 12,
                AmpA => 0.001, AmpD => 4.0, AmpS => 0.0, AmpR => 2.0, Rev_Mix => 0.4, Rev_Decay => 5.0, Rev_Size => 0.8)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.35),
            pr!("Ring Bell";
                A_P1 => SINE, B_Level => 0.7, B_P1 => SINE, B_Coarse => 7, B_Fine => 12.0, Ring => 0.8, A_Level => 0.4,
                AmpA => 0.001, AmpD => 3.0, AmpS => 0.0, AmpR => 1.5, Rev_Mix => 0.4, Rev_Decay => 4.5),
            pr!("Tubular Bell";
                A_Type => 2, A_P1 => FM_3_5, A_P2 => 0.5, A_P3 => 0.0, B_Type => 2, B_Level => 0.4, B_P1 => FM_5, B_P2 => 0.35, B_Coarse => 12, C_Type => 2, C_Level => 0.3, C_P1 => FM_1, C_P2 => 0.2,
                AmpA => 0.001, AmpD => 5.0, AmpS => 0.0, AmpR => 2.5, Rev_Mix => 0.4, Rev_Decay => 5.0)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.3),
            pr!("Glockenspiel";
                A_P1 => SINE, B_Type => 2, B_Level => 0.4, B_P1 => FM_4, B_P2 => 0.25, B_Coarse => 12, A_Coarse => 12, AmpA => 0.001, AmpD => 1.6, AmpS => 0.0, AmpR => 0.6,
                Rev_Mix => 0.3, Rev_Decay => 3.0),
            pr!("Vibraphone";
                A_P1 => SINE, B_Type => 2, B_Level => 0.3, B_P1 => FM_4, B_P2 => 0.15, AmpA => 0.002, AmpD => 3.0, AmpS => 0.0, AmpR => 1.0, L1_Rate => 5.0, Rev_Mix => 0.3, Rev_Decay => 3.0)
                .m(1, SRC_LFO1, T_AMP, -0.35),
            pr!("Marimba";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.2, A_P3 => 0.0, B_Type => 2, B_Level => 0.25, B_P1 => FM_10, B_P2 => 0.15, F1_Type => 0, F1_Cut => 3500.0,
                AmpA => 0.001, AmpD => 0.5, AmpS => 0.0, AmpR => 0.2, Rev_Mix => 0.2, Rev_Decay => 1.8)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.2),
            pr!("Xylophone";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.15, B_Type => 2, B_Level => 0.3, B_P1 => FM_10, B_P2 => 0.2, F1_Cut => 9000.0, AmpA => 0.001, AmpD => 0.25, AmpS => 0.0, AmpR => 0.12,
                Rev_Mix => 0.15),
            pr!("Music Box";
                A_P1 => SINE, B_Type => 2, B_Level => 0.35, B_P1 => FM_5, B_P2 => 0.2, A_Coarse => 12, AmpA => 0.001, AmpD => 1.0, AmpS => 0.0, AmpR => 0.5, Rev_Mix => 0.35, Rev_Decay => 3.0, Dly_Mix => 0.1),
            pr!("Church Bell";
                A_Type => 2, A_P1 => FM_2, A_P2 => 0.55, A_P3 => 0.1, A_Coarse => -12, B_Type => 2, B_Level => 0.5, B_P1 => FM_3_5, B_P2 => 0.4, C_Type => 2, C_Level => 0.3, C_P1 => FM_5, C_P2 => 0.3, C_Coarse => 12,
                AmpA => 0.001, AmpD => 8.0, AmpS => 0.0, AmpR => 4.0, Rev_Mix => 0.5, Rev_Decay => 8.0, Rev_Size => 0.9),
            pr!("Handbell";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.4, B_Type => 2, B_Level => 0.4, B_P1 => FM_4, B_P2 => 0.3, B_Coarse => 12, AmpA => 0.001, AmpD => 2.5, AmpS => 0.0, AmpR => 1.2,
                Rev_Mix => 0.35, Rev_Decay => 4.0)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.25),
            pr!("Crystal Bell";
                A_Type => 1, A_P1 => 0.5, A_P2 => GLASS, B_Type => 2, B_Level => 0.4, B_P1 => FM_5, B_P2 => 0.3, B_Coarse => 12, AmpA => 0.001, AmpD => 3.0, AmpS => 0.0, AmpR => 1.8,
                Rev_Mix => 0.5, Rev_Decay => 6.0, Dly_Mix => 0.15),
            pr!("Gamelan";
                A_Type => 2, A_P1 => FM_1_5, A_P2 => 0.45, B_Type => 2, B_Level => 0.4, B_P1 => FM_3_5, B_P2 => 0.35, B_Fine => 18.0, AmpA => 0.001, AmpD => 2.0, AmpS => 0.0, AmpR => 1.0,
                Rev_Mix => 0.3, Rev_Decay => 3.0, Cho_Mix => 0.15)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.25),
            pr!("Kalimba";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.2, B_Level => 0.3, B_P1 => TRI, B_Coarse => 12, F1_Type => 0, F1_Cut => 4500.0, AmpA => 0.001, AmpD => 0.9, AmpS => 0.0, AmpR => 0.4,
                Rev_Mix => 0.25, Rev_Decay => 2.5),
            pr!("Steel Drum";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.35, A_P3 => 0.15, B_Level => 0.3, B_P1 => TRI, B_Coarse => 12, AmpA => 0.002, AmpD => 1.2, AmpS => 0.0, AmpR => 0.5,
                Cho_Mix => 0.3, Cho_Rate => 4.5, Rev_Mix => 0.25),
            pr!("Wind Chimes";
                A_P1 => SINE, B_Type => 2, B_Level => 0.5, B_P1 => FM_5, B_P2 => 0.3, B_Coarse => 12, A_Coarse => 12, Chord => 14, AmpA => 0.001, AmpD => 4.0, AmpS => 0.0, AmpR => 3.0,
                Rev_Mix => 0.5, Rev_Decay => 7.0, Dly_Mix => 0.2, Voices => 14),
            pr!("Celestial Bell";
                A_Type => 2, A_P1 => FM_3_5, A_P2 => 0.4, B_Type => 2, B_Level => 0.4, B_P1 => FM_2, B_P2 => 0.3, B_Coarse => 19, C_Type => 1, C_Level => 0.2, C_P1 => 0.5, C_P2 => GLASS,
                AmpA => 0.002, AmpD => 5.0, AmpS => 0.0, AmpR => 4.0, Rev_Mix => 0.6, Rev_Decay => 10.0, Rev_Size => 0.9, Dly_Mix => 0.25, Dly_Time => 500.0),
        ],
    )
}
