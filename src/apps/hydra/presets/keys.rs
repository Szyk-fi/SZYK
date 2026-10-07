//! Keys: electric and acoustic keyboards, built from FM, pulses and strings.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.45), (MD_A2, 0.25)),
    ("Tine", (MD_A2, 0.35), (MD_B2, 0.25)),
    ("Sustain", (MD_DECAY, 0.5), (MD_RELEASE, 0.4)),
    ("Space", (MD_CHORUS, 0.3), (MD_REVERB, 0.25)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Keys",
        &MACROS,
        vec![
            pr!("FM Electric Piano";
                A_Type => 2, A_P1 => 0.2, A_P2 => 0.5, A_P3 => 0.1, A_Level => 0.7, B_Type => 2, B_Level => 0.3, B_P1 => 0.867, B_P2 => 0.35,
                AmpA => 0.002, AmpD => 2.2, AmpS => 0.0, AmpR => 0.5, VelSens => 0.8, Cho_Mix => 0.3, Cho_Depth => 0.4, Rev_Mix => 0.15)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.3).m(2, SRC_VELOCITY, T_A_P1 + 1, 0.25),
            pr!("Tine Rhodes";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.3, A_P3 => 0.05, B_Type => 2, B_Level => 0.25, B_P1 => FM_10, B_P2 => 0.18, AmpA => 0.002, AmpD => 2.8, AmpS => 0.0, AmpR => 0.6,
                VelSens => 0.85, L1_Rate => 4.5, Cho_Mix => 0.35, Rev_Mix => 0.2)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.25).m(2, SRC_VELOCITY, T_A_P1 + 1, 0.3).m(3, SRC_LFO1, T_AMP, -0.12),
            pr!("Wurly";
                A_P1 => PULSE, A_P2 => 0.3, B_Type => 2, B_Level => 0.3, B_P1 => FM_2, B_P2 => 0.2, F1_Type => 0, F1_Cut => 3500.0, F1_Env => 0.3, E2D => 0.3, E2S => 0.2,
                AmpA => 0.002, AmpD => 1.4, AmpS => 0.0, AmpR => 0.3, VelSens => 0.8, Drv_Type => 1, Drv_Amt => 0.3, Cho_Mix => 0.25, L1_Rate => 5.0)
                .m(1, SRC_LFO1, T_AMP, -0.15),
            pr!("Clavinet";
                A_P1 => PULSE, A_P2 => 0.2, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.35, B_Fine => 4.0, F1_Type => 2, F1_Cut => 400.0, F1_Res => 0.3, F1_Env => 0.4, E2A => 0.001, E2D => 0.12, E2S => 0.0,
                AmpA => 0.001, AmpD => 0.35, AmpS => 0.0, AmpR => 0.08, VelSens => 0.9, Ph_Mix => 0.3, Ph_Rate => 0.5),
            pr!("Harpsichord";
                A_Type => 3, A_P1 => 0.2, A_P2 => 1.0, A_P3 => 0.35, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.2, B_Coarse => 12, F1_Type => 2, F1_Cut => 300.0,
                AmpA => 0.001, AmpD => 0.8, AmpS => 0.0, AmpR => 0.1, VelSens => 0.2, Rev_Mix => 0.2, Rev_Decay => 2.0),
            pr!("Soft Piano";
                A_Type => 3, A_P1 => 0.45, A_P2 => 0.6, A_P3 => 0.55, B_Type => 2, B_Level => 0.2, B_P1 => FM_1, B_P2 => 0.15, AmpA => 0.002, AmpD => 3.0, AmpS => 0.0, AmpR => 0.5,
                VelSens => 0.8, Rev_Mix => 0.25, Rev_Decay => 3.0)
                .m(1, SRC_VELOCITY, T_F1_CUT, 0.15),
            pr!("Toy Piano";
                A_Type => 2, A_P1 => FM_3_5, A_P2 => 0.35, A_P3 => 0.0, B_Type => 2, B_Level => 0.3, B_P1 => FM_8, B_P2 => 0.3, B_Coarse => 12, AmpA => 0.001, AmpD => 0.5, AmpS => 0.0, AmpR => 0.3,
                Rev_Mix => 0.2, Rev_Decay => 1.5),
            pr!("Celesta";
                A_P1 => SINE, B_Type => 2, B_Level => 0.35, B_P1 => FM_4, B_P2 => 0.2, B_Coarse => 12, AmpA => 0.001, AmpD => 1.8, AmpS => 0.0, AmpR => 0.7, Rev_Mix => 0.4, Rev_Decay => 3.5)
                .m(1, SRC_AMP_ENV, T_A_P1 + 4, 0.2),
            pr!("Digital Piano";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.4, A_P3 => 0.0, A_Level => 0.7, B_Type => 2, B_Level => 0.35, B_P1 => FM_4, B_P2 => 0.3, C_Type => 3, C_Level => 0.2, C_P1 => 0.4, C_P2 => 0.8,
                AmpA => 0.001, AmpD => 2.4, AmpS => 0.0, AmpR => 0.4, VelSens => 0.9, Cho_Mix => 0.2, Rev_Mix => 0.2)
                .m(1, SRC_AMP_ENV, T_A_P1 + 1, 0.3).m(2, SRC_VELOCITY, T_A_P1 + 1, 0.2),
            pr!("Dyno EP";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.45, A_P3 => 0.12, B_Level => 0.3, B_P1 => TRI, F1_Cut => 6000.0, AmpA => 0.002, AmpD => 2.0, AmpS => 0.1, AmpR => 0.5,
                VelSens => 0.9, Drv_Type => 1, Drv_Amt => 0.35, Cho_Mix => 0.45, Cho_Depth => 0.5, Ph_Mix => 0.2)
                .m(1, SRC_VELOCITY, T_A_P1 + 1, 0.35),
            pr!("Stage Grand";
                A_Type => 3, A_P1 => 0.35, A_P2 => 0.8, A_P3 => 0.6, B_Type => 3, B_Level => 0.5, B_Coarse => 12, B_Fine => 3.0, B_P1 => 0.5, B_P2 => 0.7, B_P3 => 0.45,
                AmpA => 0.001, AmpD => 4.0, AmpS => 0.0, AmpR => 0.6, VelSens => 0.85, Rev_Mix => 0.3, Rev_Decay => 3.5, Cho_Mix => 0.1),
            pr!("Muted Keys";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.25, B_Level => 0.3, B_P1 => TRI, F1_Type => 0, F1_Cut => 1500.0, F1_Env => 0.2, E2D => 0.2, AmpA => 0.002, AmpD => 0.5, AmpS => 0.0, AmpR => 0.1,
                Drv_Type => 1, Drv_Amt => 0.2),
            pr!("Glass Keys";
                A_Type => 1, A_P1 => 0.5, A_P2 => GLASS, B_Type => 2, B_Level => 0.3, B_P1 => FM_5, B_P2 => 0.2, AmpA => 0.002, AmpD => 1.6, AmpS => 0.0, AmpR => 0.8,
                Rev_Mix => 0.4, Rev_Decay => 4.0, Dly_Mix => 0.15)
                .m(1, SRC_AMP_ENV, T_A_P1, -0.2),
            pr!("Mellow Keys";
                A_P1 => TRI, B_Type => 2, B_Level => 0.4, B_P1 => FM_1, B_P2 => 0.2, F1_Type => 0, F1_Cut => 2800.0, AmpA => 0.004, AmpD => 1.8, AmpS => 0.2, AmpR => 0.5,
                Cho_Mix => 0.4, Rev_Mix => 0.25, VelSens => 0.7),
            pr!("Phase Keys";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.35, B_Level => 0.35, B_P1 => TRI, B_Fine => 6.0, AmpA => 0.002, AmpD => 2.0, AmpS => 0.15, AmpR => 0.5,
                Ph_Mix => 0.6, Ph_Rate => 0.3, Ph_Depth => 0.7, Ph_Fb => 0.5, Rev_Mix => 0.2),
            pr!("Lofi Keys";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.3, B_Level => 0.3, B_P1 => TRI, Drift => 0.7, F1_Type => 0, F1_Cut => 2400.0, AmpA => 0.003, AmpD => 1.4, AmpS => 0.1, AmpR => 0.4,
                Drv_Type => 4, Drv_Amt => 0.15, Drv_Mix => 0.5, Cho_Mix => 0.45, Cho_Rate => 0.3, Rev_Mix => 0.2),
        ],
    )
}
