//! Organ: drawbar-style stacks from the organ wavetable bank, plus reeds.

use super::*;

const MACROS: MacroSet = [
    ("Drawbars", (MD_BLEV, 0.5), (MD_CLEV, 0.4)),
    ("Perc", (MD_A2, 0.3), (MD_CUTOFF, 0.3)),
    ("Leslie", (MD_PHASER, 0.5), (MD_CHORUS, 0.4)),
    ("Grit", (MD_DRIVE, 0.6), (MD_REVERB, 0.15)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Organ",
        &MACROS,
        vec![
            pr!("Drawbar Organ";
                A_Type => 1, A_P1 => 0.5, A_P2 => 0.2, B_Type => 1, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.6, B_P2 => 0.2,
                AmpA => 0.003, AmpD => 0.1, AmpS => 1.0, AmpR => 0.08, Cho_Mix => 0.3, Cho_Rate => 0.8, Ph_Mix => 0.2, Ph_Rate => 0.7, Drv_Type => 1, Drv_Amt => 0.2),
            pr!("Rock Organ";
                A_Type => 1, A_P1 => 0.55, A_P2 => ORGAN, B_Type => 1, B_Level => 0.6, B_Coarse => 12, B_P1 => 0.55, B_P2 => ORGAN, C_Type => 1, C_Level => 0.4, C_Coarse => 19, C_P1 => 0.6, C_P2 => ORGAN,
                AmpA => 0.002, AmpD => 0.1, AmpS => 1.0, AmpR => 0.06, Drv_Type => 1, Drv_Amt => 0.55, Ph_Mix => 0.4, Ph_Rate => 0.8, Cho_Mix => 0.3, Rev_Mix => 0.15),
            pr!("Jazz Organ";
                A_Type => 1, A_P1 => 0.4, A_P2 => ORGAN, B_Type => 1, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.4, B_P2 => ORGAN, C_Type => 1, C_Level => 0.35, C_Coarse => 24, C_P1 => 0.4, C_P2 => ORGAN,
                AmpA => 0.002, AmpD => 0.3, AmpS => 0.85, AmpR => 0.06, F1_Cut => 5000.0, F1_Env => 0.2, E2D => 0.12, E2S => 0.0, Cho_Mix => 0.4, Cho_Rate => 0.7, Rev_Mix => 0.15),
            pr!("Church Organ";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, A_Coarse => -12, B_Type => 1, B_Level => 0.6, B_P1 => 0.5, B_P2 => ORGAN, C_Type => 1, C_Level => 0.5, C_Coarse => 12, C_P1 => 0.6, C_P2 => ORGAN,
                AmpA => 0.04, AmpS => 1.0, AmpR => 0.3, F1_Cut => 6000.0, Rev_Mix => 0.55, Rev_Decay => 7.0, Rev_Size => 0.9),
            pr!("Pipe Organ";
                A_Type => 1, A_P1 => 0.6, A_P2 => ORGAN, B_Type => 1, B_Level => 0.6, B_Coarse => 12, B_P1 => 0.7, B_P2 => ORGAN, C_Type => 1, C_Level => 0.4, C_Coarse => 19, C_P1 => 0.7, C_P2 => ORGAN, SubLevel => 0.3,
                AmpA => 0.06, AmpS => 1.0, AmpR => 0.4, F1_Type => 0, F1_Cut => 5000.0, Rev_Mix => 0.5, Rev_Decay => 6.0),
            pr!("Gospel Organ";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_Type => 1, B_Level => 0.6, B_Coarse => 12, B_P1 => 0.5, B_P2 => ORGAN, C_Type => 1, C_Level => 0.4, C_Coarse => 24, C_P1 => 0.5, C_P2 => ORGAN,
                AmpA => 0.004, AmpD => 0.15, AmpS => 1.0, AmpR => 0.1, Cho_Mix => 0.5, Cho_Rate => 1.2, Drv_Type => 1, Drv_Amt => 0.3, Rev_Mix => 0.25),
            pr!("Farfisa";
                A_P1 => PULSE, A_P2 => 0.5, B_Level => 0.5, B_P1 => PULSE, B_P2 => 0.35, B_Coarse => 12, F1_Type => 0, F1_Cut => 4500.0, AmpA => 0.005, AmpS => 1.0, AmpR => 0.08,
                L1_Rate => 6.0, L1_Fade => 0.4, Cho_Mix => 0.35, Ph_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.002),
            pr!("Vox Continental";
                A_P1 => PULSE, A_P2 => 0.45, B_Level => 0.5, B_P1 => TRI, B_Coarse => 12, C_Level => 0.3, C_P1 => PULSE, C_P2 => 0.3, C_Coarse => 19, F1_Type => 0, F1_Cut => 3800.0,
                AmpA => 0.003, AmpS => 1.0, AmpR => 0.06, Drv_Type => 1, Drv_Amt => 0.25, Rev_Mix => 0.15),
            pr!("Combo Organ";
                A_P1 => PULSE, A_P2 => 0.3, B_Level => 0.5, B_P1 => SAW, B_Coarse => 12, F1_Type => 0, F1_Cut => 3000.0, AmpA => 0.003, AmpS => 1.0, AmpR => 0.05,
                Drv_Type => 1, Drv_Amt => 0.4, Ph_Mix => 0.3, Rev_Mix => 0.15),
            pr!("Percussive Organ";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_Type => 1, B_Level => 0.5, B_Coarse => 19, B_P1 => 0.5, B_P2 => ORGAN, C_Type => 2, C_Level => 0.25, C_P1 => FM_3, C_P2 => 0.3,
                AmpA => 0.002, AmpD => 0.15, AmpS => 0.9, AmpR => 0.06, Cho_Mix => 0.35, Ph_Mix => 0.2, Drv_Type => 1, Drv_Amt => 0.3)
                .m(1, SRC_ENV3, T_A_LEVEL + 2, -0.6),
            pr!("Reed Organ";
                A_P1 => SAW, A_P2 => 0.5, A_P3 => 0.1, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.3, F1_Type => 0, F1_Cut => 2000.0, AmpA => 0.04, AmpS => 1.0, AmpR => 0.15,
                Drift => 0.3, Rev_Mix => 0.25),
            pr!("Accordion";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 12.0, C_Level => 0.4, C_P1 => PULSE, C_P2 => 0.3, C_Coarse => 12, F1_Type => 0, F1_Cut => 3500.0, AmpA => 0.04, AmpS => 1.0, AmpR => 0.1,
                L1_Rate => 5.5, Rev_Mix => 0.2, Drift => 0.2)
                .m(1, SRC_LFO1, T_AMP, -0.12),
            pr!("Harmonium";
                A_P1 => SAW, B_Level => 0.5, B_P1 => TRI, B_Coarse => -12, F1_Type => 0, F1_Cut => 1600.0, AmpA => 0.08, AmpS => 1.0, AmpR => 0.25, Drift => 0.35,
                Cho_Mix => 0.3, Rev_Mix => 0.3),
            pr!("Theater Organ";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_Type => 1, B_Level => 0.6, B_Coarse => 12, B_P1 => 0.5, B_P2 => ORGAN, C_P1 => SAW, C_Level => 0.35, C_Coarse => 7,
                AmpA => 0.01, AmpS => 1.0, AmpR => 0.15, L1_Rate => 6.5, Cho_Mix => 0.55, Cho_Depth => 0.6, Rev_Mix => 0.4, Rev_Decay => 4.0)
                .m(1, SRC_LFO1, T_PITCH, 0.002),
            pr!("Slow Rotary";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_Type => 1, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.55, B_P2 => ORGAN, AmpA => 0.003, AmpS => 1.0, AmpR => 0.08,
                Ph_Mix => 0.55, Ph_Rate => 0.35, Ph_Depth => 0.8, Cho_Mix => 0.5, Cho_Rate => 0.4, Cho_Depth => 0.7, Rev_Mix => 0.2),
            pr!("Fast Rotary";
                A_Type => 1, A_P1 => 0.5, A_P2 => ORGAN, B_Type => 1, B_Level => 0.5, B_Coarse => 12, B_P1 => 0.55, B_P2 => ORGAN, AmpA => 0.003, AmpS => 1.0, AmpR => 0.08,
                Ph_Mix => 0.55, Ph_Rate => 3.5, Ph_Depth => 0.6, Cho_Mix => 0.5, Cho_Rate => 4.0, Cho_Depth => 0.4, Rev_Mix => 0.2),
        ],
    )
}
