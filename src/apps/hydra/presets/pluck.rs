//! Pluck: struck and plucked strings, mostly Karplus-Strong.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_A2, 0.4), (MD_CUTOFF, 0.35)),
    ("Damp", (MD_A1, 0.45), (MD_RELEASE, -0.25)),
    ("Ring", (MD_RELEASE, 0.45), (MD_REVERB, 0.15)),
    ("Space", (MD_CHORUS, 0.25), (MD_REVERB, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Pluck",
        &MACROS,
        vec![
            pr!("Pluck Harp";
                A_Type => 3, A_P1 => 0.4, A_P2 => 0.8, A_P3 => 0.4, B_Type => 3, B_Level => 0.3, B_Coarse => 12, B_P1 => 0.5, B_P2 => 0.8, B_P3 => 0.35,
                AmpS => 1.0, AmpR => 1.2, F1_Cut => 12000.0, Cho_Mix => 0.2, Rev_Mix => 0.3, Rev_Decay => 3.5),
            pr!("Nylon Guitar";
                A_Type => 3, A_P1 => 0.5, A_P2 => 0.45, A_P3 => 0.45, B_Type => 3, B_Level => 0.3, B_P1 => 0.55, B_P2 => 0.4, B_P3 => 0.4, B_Fine => 4.0,
                AmpS => 1.0, AmpR => 0.6, F1_Cut => 5500.0, Rev_Mix => 0.2, Rev_Decay => 2.0, VelSens => 0.7),
            pr!("Steel Guitar";
                A_Type => 3, A_P1 => 0.25, A_P2 => 0.95, A_P3 => 0.5, B_Type => 3, B_Level => 0.4, B_Fine => 6.0, B_P1 => 0.3, B_P2 => 0.9, B_P3 => 0.45,
                AmpS => 1.0, AmpR => 0.5, F1_Cut => 12000.0, Cho_Mix => 0.25, Rev_Mix => 0.2),
            pr!("Banjo";
                A_Type => 3, A_P1 => 0.15, A_P2 => 1.0, A_P3 => 0.3, B_Level => 0.3, B_P1 => PULSE, B_P2 => 0.15, F1_Type => 2, F1_Cut => 400.0, AmpS => 1.0, AmpR => 0.2,
                Rev_Mix => 0.15, Rev_Decay => 1.5),
            pr!("Koto";
                A_Type => 3, A_P1 => 0.35, A_P2 => 0.85, A_P3 => 0.4, B_Type => 3, B_Level => 0.3, B_Coarse => 12, B_P1 => 0.4, B_P2 => 0.8, B_P3 => 0.3,
                AmpS => 1.0, AmpR => 0.8, F1_Cut => 9000.0, Rev_Mix => 0.3, Rev_Decay => 3.0)
                .m(1, SRC_AMP_ENV, T_PITCH, 0.0015),
            pr!("Harp";
                A_Type => 3, A_P1 => 0.55, A_P2 => 0.6, A_P3 => 0.55, B_Type => 3, B_Level => 0.4, B_Coarse => 12, B_P1 => 0.6, B_P2 => 0.55, B_P3 => 0.5,
                AmpS => 1.0, AmpR => 1.4, F1_Cut => 8000.0, Rev_Mix => 0.4, Rev_Decay => 4.0, Cho_Mix => 0.15),
            pr!("Mandolin";
                A_Type => 3, A_P1 => 0.2, A_P2 => 0.9, A_P3 => 0.35, B_Type => 3, B_Level => 0.8, B_Fine => 9.0, B_P1 => 0.2, B_P2 => 0.9, B_P3 => 0.35,
                AmpS => 1.0, AmpR => 0.3, F1_Cut => 10000.0, Cho_Mix => 0.2, Rev_Mix => 0.2),
            pr!("Sitar Buzz";
                A_Type => 3, A_P1 => 0.2, A_P2 => 1.0, A_P3 => 0.6, B_Level => 0.3, B_P1 => PULSE, B_P2 => 0.1, F1_Type => 4, F1_Cut => 1800.0, F1_Res => 0.5,
                AmpS => 1.0, AmpR => 1.0, Drv_Type => 2, Drv_Amt => 0.25, Drv_Mix => 0.5, Rev_Mix => 0.25)
                .m(1, SRC_AMP_ENV, T_F1_CUT, 0.15),
            pr!("Bouzouki";
                A_Type => 3, A_P1 => 0.3, A_P2 => 0.85, A_P3 => 0.4, B_Type => 3, B_Level => 0.6, B_Fine => 7.0, B_P1 => 0.3, B_P2 => 0.85, B_P3 => 0.4,
                AmpS => 1.0, AmpR => 0.5, F1_Cut => 7000.0, Rev_Mix => 0.2),
            pr!("Muted Pluck";
                A_Type => 3, A_P1 => 0.9, A_P2 => 0.5, A_P3 => 0.15, AmpS => 1.0, AmpR => 0.1, F1_Cut => 3500.0),
            pr!("Zither";
                A_Type => 3, A_P1 => 0.2, A_P2 => 0.8, A_P3 => 0.65, B_Type => 3, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.25, B_P2 => 0.8, B_P3 => 0.65,
                AmpS => 1.0, AmpR => 2.0, F1_Cut => 14000.0, Rev_Mix => 0.35, Rev_Decay => 4.5),
            pr!("Dulcimer";
                A_Type => 3, A_P1 => 0.3, A_P2 => 1.0, A_P3 => 0.5, B_Type => 3, B_Level => 0.7, B_Fine => 5.0, B_P1 => 0.3, B_P2 => 1.0, B_P3 => 0.5,
                AmpS => 1.0, AmpR => 1.2, F1_Cut => 12000.0, Cho_Mix => 0.3, Rev_Mix => 0.3),
            pr!("Lute";
                A_Type => 3, A_P1 => 0.6, A_P2 => 0.5, A_P3 => 0.4, B_Type => 3, B_Level => 0.3, B_Coarse => 12, B_P1 => 0.65, B_P2 => 0.45, B_P3 => 0.35,
                AmpS => 1.0, AmpR => 0.5, F1_Cut => 4500.0, Rev_Mix => 0.25),
            pr!("Bright Pluck";
                A_Type => 3, A_P1 => 0.1, A_P2 => 1.0, A_P3 => 0.3, B_Level => 0.3, B_P1 => SAW, F1_Type => 1, F1_Cut => 2000.0, F1_Env => 0.4, E2D => 0.2, E2S => 0.0,
                AmpS => 1.0, AmpR => 0.2, Dly_Mix => 0.2, Dly_Time => 280.0, Rev_Mix => 0.2),
            pr!("Pizzicato";
                A_Type => 3, A_P1 => 0.7, A_P2 => 0.7, A_P3 => 0.1, B_P1 => SAW, B_Level => 0.2, F1_Type => 0, F1_Cut => 2500.0, AmpS => 1.0, AmpR => 0.12, Rev_Mix => 0.2),
            pr!("Glass Harp Pluck";
                A_Type => 3, A_P1 => 0.1, A_P2 => 0.4, A_P3 => 0.85, B_Type => 3, B_Level => 0.6, B_Coarse => 19, B_P1 => 0.1, B_P2 => 0.4, B_P3 => 0.85,
                AmpS => 1.0, AmpR => 3.0, F1_Cut => 14000.0, Rev_Mix => 0.5, Rev_Decay => 6.0, Dly_Mix => 0.2),
        ],
    )
}
