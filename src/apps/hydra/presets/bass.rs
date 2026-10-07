//! Bass: mono and legato low end, from clean subs to growls.

use super::*;

const MACROS: MacroSet = [
    ("Cutoff", (MD_CUTOFF, 0.5), (MD_RES, 0.12)),
    ("Drive", (MD_DRIVE, 0.6), (MD_FDRIVE, 0.25)),
    ("Sub", (MD_SUB, 0.6), (MD_DETUNE, 0.3)),
    ("Space", (MD_DELAY, 0.2), (MD_REVERB, 0.18)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Bass",
        &MACROS,
        vec![
            pr!("Sub Bass";
                A_P1 => SINE, A_Level => 0.9, SubLevel => 0.6, F1_Cut => 900.0, F1_Res => 0.1, F1_Env => 0.25,
                E2A => 0.001, E2D => 0.2, E2S => 0.0, AmpA => 0.002, AmpD => 0.3, AmpS => 0.9, AmpR => 0.12,
                Mode => 1, Glide => 0.05, Drv_Type => 1, Drv_Amt => 0.25, Drv_Mix => 0.6),
            pr!("Acid Ladder";
                A_P1 => SAW, Mode => 1, Glide => 0.04, F1_Type => 0, F1_Cut => 14000.0, F2_Cut => 450.0, F2_Res => 0.8, F2_Env => 0.65, F2_Key => 0.4, F2_Drive => 0.4,
                E2A => 0.001, E2D => 0.25, E2S => 0.0, AmpD => 0.3, AmpS => 0.6, AmpR => 0.1, Drv_Type => 1, Drv_Amt => 0.3,
                Dly_Mix => 0.18, Dly_Time => 375.0, Dly_Fb => 0.4),
            pr!("Reese Bass";
                A_P1 => SAW, Unison => 3, UniDetune => 0.4, UniSpread => 0.2, SubLevel => 0.4, Mode => 1, F1_Cut => 500.0, F1_Res => 0.2,
                F2_Cut => 1400.0, F2_Res => 0.2, Route => 0, AmpD => 0.4, AmpS => 0.9, AmpR => 0.2, Drv_Type => 3, Drv_Amt => 0.25, Drv_Mix => 0.5)
                .m(1, SRC_LFO1, T_F1_CUT, 0.12),
            pr!("Wobble";
                A_P1 => SAW, SubLevel => 0.5, Mode => 1, F1_Cut => 400.0, F1_Res => 0.45, L1_Rate => 3.0, AmpS => 1.0, AmpR => 0.1, Drv_Type => 1, Drv_Amt => 0.35)
                .m(1, SRC_LFO1, T_F1_CUT, 0.6),
            pr!("Karplus Bass";
                A_Type => 3, A_Coarse => -12, A_P1 => 0.8, A_P2 => 0.7, A_P3 => 0.3, SubLevel => 0.3, AmpS => 1.0, AmpR => 0.3, F1_Cut => 6000.0, Mode => 1),
            pr!("808 Sub";
                A_P1 => SINE, AmpA => 0.001, AmpD => 0.7, AmpS => 0.0, AmpR => 0.2, E3A => 0.0005, E3D => 0.1, E3S => 0.0, Mode => 1, Drv_Type => 1, Drv_Amt => 0.2)
                .m(1, SRC_ENV3, T_PITCH, 0.35),
            pr!("Moog Bass";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Coarse => -12, B_Fine => 6.0, F1_Type => 1, F1_Cut => 500.0, F1_Res => 0.25, F1_Env => 0.55,
                E2A => 0.002, E2D => 0.25, E2S => 0.2, AmpA => 0.002, AmpD => 0.3, AmpS => 0.8, AmpR => 0.15, Mode => 1, Glide => 0.03, Drv_Type => 1, Drv_Amt => 0.2, Drv_Mix => 0.5),
            pr!("FM Bass";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.35, A_P3 => 0.12, SubLevel => 0.3, AmpA => 0.002, AmpD => 0.35, AmpS => 0.6, AmpR => 0.12,
                E3D => 0.2, E3S => 0.0, Mode => 1, F1_Cut => 7000.0)
                .m(1, SRC_ENV3, T_A_P1 + 1, 0.3),
            pr!("Square Pump";
                A_P1 => PULSE, A_P2 => 0.5, B_Level => 0.6, B_P1 => PULSE, B_Fine => 8.0, F1_Type => 0, F1_Cut => 900.0, F1_Res => 0.25, F1_Env => 0.4,
                E2D => 0.2, E2S => 0.2, AmpD => 0.2, AmpS => 0.7, AmpR => 0.1, Mode => 1, Drv_Type => 1, Drv_Amt => 0.3),
            pr!("Growl";
                A_Type => 1, A_P1 => 0.3, A_P2 => VOWEL, SubLevel => 0.4, Mode => 1, F1_Type => 1, F1_Cut => 1500.0, F1_Res => 0.3, L1_Rate => 6.0, L2_Rate => 0.7,
                AmpS => 1.0, AmpR => 0.1, Drv_Type => 2, Drv_Amt => 0.5, Drv_Mix => 0.7)
                .m(1, SRC_LFO1, T_A_P1, 0.45).m(2, SRC_LFO2, T_F1_CUT, 0.3),
            pr!("Wub Wub";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 9.0, Unison => 3, UniDetune => 0.3, SubLevel => 0.35, Mode => 1,
                F1_Type => 1, F1_Cut => 250.0, F1_Res => 0.5, L1_Rate => 2.0, AmpS => 1.0, AmpR => 0.08, Drv_Type => 1, Drv_Amt => 0.45)
                .m(1, SRC_LFO1, T_F1_CUT, 0.75),
            pr!("Synth Pluck Bass";
                A_P1 => SAW, SubLevel => 0.3, F1_Type => 1, F1_Cut => 300.0, F1_Res => 0.2, F1_Env => 0.9, E2A => 0.001, E2D => 0.12, E2S => 0.0,
                AmpA => 0.001, AmpD => 0.25, AmpS => 0.0, AmpR => 0.1, Mode => 1),
            pr!("Pure Sine Sub";
                A_P1 => SINE, SubLevel => 0.5, F1_Cut => 2000.0, AmpA => 0.005, AmpD => 0.1, AmpS => 1.0, AmpR => 0.12, Mode => 2, Glide => 0.08),
            pr!("Fold Sub";
                A_P1 => SINE, A_P3 => 0.55, SubLevel => 0.4, F1_Cut => 5000.0, AmpA => 0.002, AmpD => 0.3, AmpS => 0.8, AmpR => 0.12, Mode => 1,
                Drv_Type => 2, Drv_Amt => 0.35, Drv_Mix => 0.6),
            pr!("Rubber Bass";
                A_P1 => SAW, B_Level => 0.6, B_P1 => PULSE, B_P2 => 0.4, F1_Type => 1, F1_Cut => 350.0, F1_Res => 0.35, F1_Env => 0.8, E2D => 0.18, E2S => 0.1,
                AmpD => 0.3, AmpS => 0.7, AmpR => 0.12, Mode => 2, Glide => 0.06, Drv_Type => 1, Drv_Amt => 0.3),
            pr!("Table Bass";
                A_Type => 1, A_P1 => 0.3, A_P2 => DIGITAL, SubLevel => 0.35, F1_Type => 0, F1_Cut => 4000.0, E3D => 0.25, E3S => 0.0,
                AmpA => 0.002, AmpD => 0.3, AmpS => 0.8, AmpR => 0.12, Mode => 1)
                .m(1, SRC_ENV3, T_A_P1, 0.4),
            pr!("303 Square";
                A_P1 => PULSE, A_P2 => 0.5, Mode => 1, Glide => 0.05, F1_Type => 1, F1_Cut => 500.0, F1_Res => 0.8, F1_Env => 0.7, E2D => 0.2, E2S => 0.0,
                AmpD => 0.25, AmpS => 0.5, AmpR => 0.08, Drv_Type => 1, Drv_Amt => 0.3),
        ],
    )
}
