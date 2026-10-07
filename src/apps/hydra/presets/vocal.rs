//! Vocal: formant-like voices from the vowel wavetable bank.

use super::*;

const MACROS: MacroSet = [
    ("Vowel", (MD_A1, 0.6), (MD_B1, 0.6)),
    ("Breath", (MD_NOISE, 0.25), (MD_CUTOFF, 0.3)),
    ("Choir", (MD_DETUNE, 0.4), (MD_CHORUS, 0.35)),
    ("Hall", (MD_REVERB, 0.4), (MD_RDECAY, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Vocal",
        &MACROS,
        vec![
            pr!("Vowel Choir";
                A_Type => 1, A_P1 => 0.5, A_P2 => 0.4, B_Type => 1, B_Level => 0.6, B_Fine => 6.0, B_P1 => 0.5, B_P2 => 0.4,
                L1_Rate => 0.15, AmpA => 0.6, AmpS => 0.9, AmpR => 1.2, F1_Cut => 7000.0, Cho_Mix => 0.3, Rev_Mix => 0.5, Rev_Decay => 5.0)
                .m(1, SRC_LFO1, T_A_P1, 0.5).m(2, SRC_LFO1, T_A_P1 + 3, 0.45),
            pr!("Choir Ah";
                A_Type => 1, A_P1 => 0.15, A_P2 => VOWEL, B_Type => 1, B_Level => 0.7, B_P1 => 0.15, B_P2 => VOWEL, B_Fine => 8.0, C_Type => 1, C_Level => 0.5, C_P1 => 0.15, C_P2 => VOWEL, C_Coarse => -12,
                Unison => 3, UniDetune => 0.2, AmpA => 0.5, AmpS => 1.0, AmpR => 1.0, F1_Cut => 6500.0, Cho_Mix => 0.35, Rev_Mix => 0.5, Rev_Decay => 5.0, Drift => 0.3),
            pr!("Choir Oo";
                A_Type => 1, A_P1 => 0.0, A_P2 => VOWEL, B_Type => 1, B_Level => 0.7, B_P1 => 0.0, B_P2 => VOWEL, B_Fine => 7.0, Unison => 3, UniDetune => 0.2, AmpA => 0.6, AmpS => 1.0, AmpR => 1.2,
                F1_Cut => 4000.0, Cho_Mix => 0.35, Rev_Mix => 0.5, Rev_Decay => 5.0, Drift => 0.3),
            pr!("Male Choir";
                A_Type => 1, A_P1 => 0.25, A_P2 => VOWEL, A_Coarse => -12, B_Type => 1, B_Level => 0.7, B_P1 => 0.25, B_P2 => VOWEL, B_Coarse => -12, B_Fine => 7.0, Unison => 3, UniDetune => 0.2,
                AmpA => 0.5, AmpS => 1.0, AmpR => 1.0, F1_Cut => 3500.0, Cho_Mix => 0.3, Rev_Mix => 0.5, Rev_Decay => 5.0)
                .m(1, SRC_LFO1, T_A_P1, 0.2),
            pr!("Female Choir";
                A_Type => 1, A_P1 => 0.35, A_P2 => VOWEL, A_Coarse => 12, B_Type => 1, B_Level => 0.7, B_P1 => 0.35, B_P2 => VOWEL, B_Coarse => 12, B_Fine => 8.0, Unison => 3, UniDetune => 0.2,
                AmpA => 0.5, AmpS => 1.0, AmpR => 1.0, F1_Cut => 9000.0, Cho_Mix => 0.35, Rev_Mix => 0.5, Rev_Decay => 5.0)
                .m(1, SRC_LFO1, T_A_P1, 0.2),
            pr!("Formant Lead";
                A_Type => 1, A_P1 => 0.45, A_P2 => VOWEL, B_Type => 1, B_Level => 0.4, B_P1 => 0.45, B_P2 => VOWEL, B_Fine => 5.0, Mode => 2, Glide => 0.06, AmpA => 0.03, AmpS => 1.0, AmpR => 0.25,
                L2_Rate => 0.5, Dly_Mix => 0.2, Rev_Mix => 0.25)
                .m(1, SRC_LFO2, T_A_P1, 0.35).m(2, SRC_MOD_WHEEL, T_A_P1, 0.5),
            pr!("Talk Box";
                A_P1 => SAW, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.4, Mode => 2, Glide => 0.05, F1_Type => 4, F1_Cut => 800.0, F1_Res => 0.6, L1_Shape => 3, L1_Rate => 3.0, AmpA => 0.01, AmpS => 1.0, AmpR => 0.1,
                Drv_Type => 1, Drv_Amt => 0.3)
                .m(1, SRC_LFO1, T_F1_CUT, 0.35),
            pr!("Vowel Pad";
                A_Type => 1, A_P1 => 0.3, A_P2 => VOWEL, B_Type => 1, B_Level => 0.6, B_P1 => 0.6, B_P2 => VOWEL, B_Fine => 7.0, Unison => 3, UniDetune => 0.25, AmpA => 1.2, AmpS => 1.0, AmpR => 2.5,
                L1_Rate => 0.08, L2_Rate => 0.13, F1_Cut => 6000.0, Rev_Mix => 0.55, Rev_Decay => 7.0, Cho_Mix => 0.3)
                .m(1, SRC_LFO1, T_A_P1, 0.5).m(2, SRC_LFO2, T_A_P1 + 3, 0.5),
            pr!("Whisper";
                A_Type => 4, A_P1 => 0.3, A_P2 => 0.0, A_P3 => 0.0, B_Type => 1, B_Level => 0.3, B_P1 => 0.4, B_P2 => VOWEL, F1_Type => 4, F1_Cut => 1800.0, F1_Res => 0.5, AmpA => 0.3, AmpS => 1.0, AmpR => 0.8,
                L1_Rate => 0.2, Rev_Mix => 0.4, Rev_Decay => 4.0)
                .m(1, SRC_LFO1, T_F1_CUT, 0.3),
            pr!("Angelic";
                A_Type => 1, A_P1 => 0.3, A_P2 => VOWEL, A_Coarse => 12, B_Type => 1, B_Level => 0.6, B_P1 => 0.3, B_P2 => VOWEL, B_Coarse => 19, B_Fine => 5.0, Unison => 3, UniDetune => 0.2,
                AmpA => 1.0, AmpS => 1.0, AmpR => 2.5, F1_Cut => 10000.0, Cho_Mix => 0.4, Rev_Mix => 0.6, Rev_Decay => 9.0, Rev_Size => 0.9),
            pr!("Monks";
                A_Type => 1, A_P1 => 0.2, A_P2 => VOWEL, A_Coarse => -12, B_Type => 1, B_Level => 0.6, B_P1 => 0.2, B_P2 => VOWEL, B_Coarse => -5, C_Type => 1, C_Level => 0.4, C_P1 => 0.2, C_P2 => VOWEL, C_Coarse => 0,
                AmpA => 0.8, AmpS => 1.0, AmpR => 1.5, F1_Cut => 2800.0, Rev_Mix => 0.6, Rev_Decay => 8.0, Rev_Size => 0.9, Drift => 0.4),
            pr!("Gregorian";
                A_Type => 1, A_P1 => 0.25, A_P2 => VOWEL, A_Coarse => -12, B_Type => 1, B_Level => 0.8, B_P1 => 0.25, B_P2 => VOWEL, B_Coarse => -12, B_Fine => 12.0, Unison => 2, UniDetune => 0.15, Chord => 2,
                AmpA => 0.4, AmpS => 1.0, AmpR => 0.8, F1_Cut => 3000.0, Rev_Mix => 0.6, Rev_Decay => 8.0, Voices => 12),
            pr!("Vocoder Saw";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 8.0, Unison => 3, UniDetune => 0.25, F1_Type => 4, F1_Cut => 1200.0, F1_Res => 0.55, F2_Cut => 3000.0, F2_Res => 0.4, Route => 1, FMix => 0.5,
                L1_Rate => 0.6, L1_Shape => 5, AmpA => 0.02, AmpS => 1.0, AmpR => 0.2)
                .m(1, SRC_LFO1, T_F1_CUT, 0.35).m(2, SRC_LFO1, T_F2_CUT, -0.3),
            pr!("Scat Voice";
                A_Type => 1, A_P1 => 0.4, A_P2 => VOWEL, Mode => 2, Glide => 0.03, AmpA => 0.01, AmpD => 0.2, AmpS => 0.6, AmpR => 0.15, F1_Cut => 6000.0, E3D => 0.15, E3S => 0.0,
                Rev_Mix => 0.25)
                .m(1, SRC_ENV3, T_A_P1, 0.5),
            pr!("Opera Soprano";
                A_Type => 1, A_P1 => 0.25, A_P2 => VOWEL, A_Coarse => 12, B_Type => 1, B_Level => 0.4, B_P1 => 0.25, B_P2 => VOWEL, B_Coarse => 12, B_Fine => 5.0, Mode => 2, Glide => 0.05,
                AmpA => 0.15, AmpS => 1.0, AmpR => 0.5, L1_Rate => 5.5, L1_Fade => 1.0, F1_Cut => 10000.0, Rev_Mix => 0.5, Rev_Decay => 4.5)
                .m(1, SRC_LFO1, T_PITCH, 0.004),
        ],
    )
}
