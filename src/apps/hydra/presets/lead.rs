//! Lead: solo voices, mono and legato, bright and cutting.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.1)),
    ("Bite", (MD_DRIVE, 0.5), (MD_A1, 0.25)),
    ("Movement", (MD_LFO1, 0.4), (MD_DETUNE, 0.3)),
    ("Echo", (MD_DELAY, 0.3), (MD_REVERB, 0.15)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Lead",
        &MACROS,
        vec![
            pr!("Super Saw";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 7.0, Unison => 7, UniDetune => 0.5, UniSpread => 0.9,
                F1_Cut => 6500.0, F1_Res => 0.15, AmpA => 0.006, AmpD => 0.6, AmpS => 0.8, AmpR => 0.45,
                Cho_Mix => 0.25, Rev_Mix => 0.22, Rev_Size => 0.7, Rev_Decay => 2.5),
            pr!("Sync Lead";
                A_P1 => PULSE, A_P2 => 0.35, B_Level => 0.6, B_P1 => SAW, B_Coarse => 7, B_Sync => 1, Mode => 2, Glide => 0.06,
                F1_Cut => 5200.0, F1_Res => 0.25, F1_Env => 0.15, E2D => 0.4, AmpD => 0.3, AmpS => 0.8, AmpR => 0.2, Dly_Mix => 0.2, Dly_Time => 330.0, Rev_Mix => 0.2)
                .m(1, SRC_ENV3, T_PITCH_A + 1, 0.2).m(2, SRC_MOD_WHEEL, T_PITCH, 0.01),
            pr!("Detuned Square";
                A_P1 => PULSE, A_P2 => 0.5, Unison => 5, UniDetune => 0.35, UniSpread => 0.8, F1_Type => 0, F1_Cut => 4000.0, AmpA => 0.01, AmpS => 0.9, Cho_Mix => 0.3),
            pr!("Hoover";
                A_P1 => PULSE, A_P2 => 0.3, B_Level => 0.8, B_P1 => SAW, B_Coarse => -12, Unison => 5, UniDetune => 0.7, UniSpread => 0.9, Mode => 2, Glide => 0.12,
                F1_Type => 1, F1_Cut => 3000.0, F1_Res => 0.3, AmpA => 0.01, AmpS => 1.0, AmpR => 0.3, Drv_Type => 2, Drv_Amt => 0.3, Drv_Mix => 0.6, Cho_Mix => 0.3)
                .m(1, SRC_LFO2, T_PITCH, 0.004),
            pr!("Mono Saw";
                A_P1 => SAW, SubLevel => 0.2, Mode => 2, Glide => 0.05, F1_Type => 1, F1_Cut => 3500.0, F1_Res => 0.2, F1_Env => 0.2, E2D => 0.3, E2S => 0.4,
                AmpA => 0.003, AmpS => 0.9, AmpR => 0.2, Dly_Mix => 0.15, Dly_Time => 300.0, Rev_Mix => 0.15),
            pr!("Pulse Lead";
                A_P1 => PULSE, A_P2 => 0.25, B_Level => 0.5, B_P1 => PULSE, B_P2 => 0.7, B_Fine => 5.0, Mode => 2, Glide => 0.04, F1_Cut => 5000.0, F1_Res => 0.15,
                AmpS => 0.9, AmpR => 0.15, Dly_Mix => 0.2, Dly_Time => 280.0)
                .m(1, SRC_LFO2, T_A_P1 + 1, 0.2),
            pr!("FM Lead";
                A_Type => 2, A_P1 => FM_2, A_P2 => 0.45, A_P3 => 0.2, B_Type => 2, B_Level => 0.4, B_P1 => FM_1, B_P2 => 0.3, B_Fine => 7.0,
                Mode => 2, Glide => 0.05, AmpA => 0.004, AmpD => 0.4, AmpS => 0.8, AmpR => 0.2, Dly_Mix => 0.2, Rev_Mix => 0.2)
                .m(1, SRC_VELOCITY, T_A_P1 + 1, 0.3),
            pr!("Sine Whistle";
                A_P1 => SINE, B_Level => 0.15, B_P1 => TRI, B_Coarse => 12, Mode => 2, Glide => 0.1, AmpA => 0.04, AmpS => 1.0, AmpR => 0.25, L1_Rate => 5.5, L1_Fade => 0.8,
                Rev_Mix => 0.3, Rev_Decay => 3.0)
                .m(1, SRC_LFO1, T_PITCH, 0.005),
            pr!("Trance Lead";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 12.0, C_Level => 0.5, C_P1 => PULSE, C_Fine => -9.0, Unison => 5, UniDetune => 0.45, UniSpread => 0.9,
                F1_Type => 1, F1_Cut => 7000.0, F1_Res => 0.1, AmpA => 0.004, AmpS => 0.9, AmpR => 0.3, Dly_Mix => 0.3, Dly_Time => 375.0, Dly_Fb => 0.45, Rev_Mix => 0.25),
            pr!("Screamer";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Coarse => 12, Mode => 2, Glide => 0.03, F1_Type => 1, F1_Cut => 2200.0, F1_Res => 0.7, F1_Key => 0.8,
                AmpS => 1.0, AmpR => 0.15, Drv_Type => 2, Drv_Amt => 0.55, Drv_Mix => 0.8),
            pr!("Theremin";
                A_P1 => SINE, B_Level => 0.2, B_P1 => TRI, Mode => 2, Glide => 0.25, AmpA => 0.12, AmpS => 1.0, AmpR => 0.3, L1_Rate => 6.0, L1_Fade => 1.2,
                F1_Cut => 4500.0, Rev_Mix => 0.3, Rev_Decay => 3.5, Rev_Size => 0.7)
                .m(1, SRC_LFO1, T_PITCH, 0.008),
            pr!("Moog Lead";
                A_P1 => SAW, B_Level => 0.9, B_P1 => SAW, B_Fine => 5.0, C_Level => 0.6, C_P1 => PULSE, C_Coarse => -12, Mode => 2, Glide => 0.09,
                F1_Type => 1, F1_Cut => 1800.0, F1_Res => 0.3, F1_Env => 0.4, E2A => 0.004, E2D => 0.5, E2S => 0.5, AmpA => 0.004, AmpD => 0.3, AmpS => 0.9, AmpR => 0.2,
                Drv_Type => 1, Drv_Amt => 0.25),
            pr!("Vowel Lead";
                A_Type => 1, A_P1 => 0.45, A_P2 => VOWEL, B_Type => 1, B_Level => 0.4, B_P1 => 0.4, B_P2 => VOWEL, B_Fine => 8.0, Mode => 2, Glide => 0.05,
                AmpA => 0.02, AmpS => 0.9, AmpR => 0.2, L2_Rate => 0.3, Dly_Mix => 0.2, Rev_Mix => 0.25)
                .m(1, SRC_LFO2, T_A_P1, 0.4).m(2, SRC_MOD_WHEEL, T_A_P1, 0.4),
            pr!("Digital Lead";
                A_Type => 1, A_P1 => 0.55, A_P2 => DIGITAL, B_Type => 1, B_Level => 0.5, B_P1 => 0.6, B_P2 => SWEEP, B_Fine => 6.0, Unison => 3, UniDetune => 0.25,
                Mode => 2, Glide => 0.04, AmpA => 0.006, AmpS => 0.85, AmpR => 0.25, F1_Cut => 9000.0, Dly_Mix => 0.2, Rev_Mix => 0.2)
                .m(1, SRC_ENV3, T_A_P1, 0.3).m(2, SRC_LFO2, T_A_P1 + 3, 0.3),
            pr!("Acid Lead";
                A_P1 => SAW, Mode => 2, Glide => 0.07, F1_Type => 1, F1_Cut => 900.0, F1_Res => 0.7, F1_Env => 0.7, E2D => 0.3, E2S => 0.2,
                AmpS => 0.8, AmpR => 0.12, Drv_Type => 1, Drv_Amt => 0.4, Dly_Mix => 0.25, Dly_Time => 375.0, Dly_Fb => 0.5),
            pr!("Smooth Legato";
                A_P1 => TRI, B_Level => 0.6, B_P1 => SAW, B_Fine => 4.0, Mode => 2, Glide => 0.12, F1_Type => 0, F1_Cut => 2800.0, AmpA => 0.03, AmpS => 1.0, AmpR => 0.35,
                L1_Rate => 5.0, L1_Fade => 0.6, Cho_Mix => 0.3, Rev_Mix => 0.25)
                .m(1, SRC_LFO1, T_PITCH, 0.004),
            pr!("Ring Lead";
                A_P1 => SINE, B_Level => 0.7, B_P1 => SINE, B_Coarse => 7, B_Fine => 5.0, Ring => 0.7, A_Level => 0.4, Mode => 2, Glide => 0.05,
                AmpA => 0.004, AmpS => 0.9, AmpR => 0.2, F1_Cut => 6000.0, Dly_Mix => 0.2),
        ],
    )
}
