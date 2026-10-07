//! Arp: sounds with the arpeggiator already running. Hold a chord.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.1)),
    ("Gate", (MD_DECAY, 0.45), (MD_RELEASE, 0.35)),
    ("Movement", (MD_A1, 0.4), (MD_LFO1, 0.3)),
    ("Echo", (MD_DELAY, 0.3), (MD_DFB, 0.2)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Arp",
        &MACROS,
        vec![
            pr!("Glass Arp";
                A_Type => 1, A_P1 => 0.5, A_P2 => 1.0, AmpA => 0.002, AmpD => 0.25, AmpS => 0.2, AmpR => 0.3, Arp_On => 1, Arp_Mode => 2, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.4,
                Dly_Mix => 0.3, Dly_Time => 300.0, Rev_Mix => 0.35, Rev_Decay => 3.0),
            pr!("Classic Up";
                A_P1 => SAW, B_Level => 0.5, B_P1 => PULSE, B_P2 => 0.4, B_Fine => 6.0, F1_Type => 1, F1_Cut => 1800.0, F1_Res => 0.25, F1_Env => 0.4, E2D => 0.2, E2S => 0.0,
                AmpA => 0.002, AmpD => 0.2, AmpS => 0.0, AmpR => 0.1, Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.5, Dly_Mix => 0.25, Dly_Time => 375.0, Rev_Mix => 0.2),
            pr!("Trance Gate";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 8.0, Unison => 3, UniDetune => 0.3, F1_Type => 1, F1_Cut => 3500.0, AmpA => 0.002, AmpS => 1.0, AmpR => 0.05,
                Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Oct => 1, Arp_Gate => 0.35, Arp_Bpm => 138.0, Dly_Mix => 0.3, Dly_Time => 326.0, Dly_Fb => 0.45, Rev_Mix => 0.25),
            pr!("Pluck Arp";
                A_Type => 3, A_P1 => 0.3, A_P2 => 0.9, A_P3 => 0.35, AmpS => 1.0, AmpR => 0.3, F1_Cut => 10000.0, Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.8,
                Dly_Mix => 0.25, Dly_Time => 300.0, Rev_Mix => 0.3),
            pr!("Bass Arp";
                A_P1 => SAW, SubLevel => 0.4, Mode => 1, F1_Type => 1, F1_Cut => 400.0, F1_Res => 0.3, F1_Env => 0.7, E2D => 0.15, E2S => 0.0, AmpD => 0.2, AmpS => 0.0, AmpR => 0.08,
                Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.5, Drv_Type => 1, Drv_Amt => 0.3),
            pr!("Sequencer 16th";
                A_P1 => PULSE, A_P2 => 0.4, F1_Type => 1, F1_Cut => 1500.0, F1_Res => 0.35, F1_Env => 0.5, E2D => 0.12, E2S => 0.0, AmpD => 0.15, AmpS => 0.0, AmpR => 0.05,
                Arp_On => 1, Arp_Mode => 4, Arp_Rate => 3, Arp_Oct => 1, Arp_Gate => 0.4, Dly_Mix => 0.25, Dly_Time => 375.0, Dly_Fb => 0.4),
            pr!("Minor Arp";
                A_P1 => TRI, B_Level => 0.5, B_P1 => SAW, B_Coarse => 12, F1_Type => 0, F1_Cut => 4000.0, AmpA => 0.003, AmpD => 0.3, AmpS => 0.0, AmpR => 0.2, Chord => 4,
                Arp_On => 1, Arp_Mode => 2, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.55, Dly_Mix => 0.3, Dly_Time => 300.0, Rev_Mix => 0.3),
            pr!("Octave Pulse";
                A_P1 => PULSE, A_P2 => 0.5, B_Level => 0.5, B_P1 => SAW, F1_Type => 1, F1_Cut => 2500.0, F1_Res => 0.2, AmpA => 0.002, AmpD => 0.12, AmpS => 0.0, AmpR => 0.05, Chord => 1,
                Arp_On => 1, Arp_Mode => 4, Arp_Rate => 3, Arp_Oct => 1, Arp_Gate => 0.4),
            pr!("Random Sparkle";
                A_Type => 2, A_P1 => FM_3_5, A_P2 => 0.3, B_Type => 1, B_Level => 0.4, B_P1 => 0.5, B_P2 => GLASS, AmpA => 0.001, AmpD => 0.4, AmpS => 0.0, AmpR => 0.5,
                Arp_On => 1, Arp_Mode => 3, Arp_Rate => 3, Arp_Oct => 3, Arp_Gate => 0.4, Dly_Mix => 0.35, Dly_Time => 450.0, Dly_Fb => 0.5, Rev_Mix => 0.4),
            pr!("Maj7 Arp";
                A_P1 => SAW, B_Level => 0.5, B_P1 => PULSE, B_P2 => 0.4, B_Fine => 6.0, F1_Type => 1, F1_Cut => 3500.0, AmpA => 0.002, AmpD => 0.3, AmpS => 0.0, AmpR => 0.2, Chord => 7,
                Arp_On => 1, Arp_Mode => 2, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.5, Cho_Mix => 0.3, Dly_Mix => 0.3, Dly_Time => 330.0, Rev_Mix => 0.3),
            pr!("Slow Arp Pad";
                A_P1 => SAW, B_Level => 0.6, B_P1 => TRI, B_Fine => 6.0, Unison => 3, UniDetune => 0.25, F1_Type => 0, F1_Cut => 2500.0, AmpA => 0.3, AmpS => 1.0, AmpR => 1.2,
                Arp_On => 1, Arp_Mode => 2, Arp_Rate => 1, Arp_Oct => 2, Arp_Gate => 1.0, Arp_Bpm => 90.0, Cho_Mix => 0.4, Rev_Mix => 0.5, Rev_Decay => 5.0),
            pr!("Fast 32nd";
                A_P1 => PULSE, A_P2 => 0.3, F1_Type => 1, F1_Cut => 4000.0, AmpA => 0.001, AmpD => 0.05, AmpS => 0.0, AmpR => 0.03, Arp_On => 1, Arp_Mode => 0, Arp_Rate => 5, Arp_Oct => 2, Arp_Gate => 0.5, Arp_Bpm => 110.0,
                Dly_Mix => 0.2, Dly_Time => 270.0),
            pr!("Triplet Arp";
                A_Type => 1, A_P1 => 0.4, A_P2 => DIGITAL, AmpA => 0.002, AmpD => 0.25, AmpS => 0.1, AmpR => 0.2, F1_Cut => 7000.0, Arp_On => 1, Arp_Mode => 2, Arp_Rate => 4, Arp_Oct => 2, Arp_Gate => 0.6,
                Dly_Mix => 0.3, Dly_Time => 330.0, Rev_Mix => 0.3)
                .m(1, SRC_AMP_ENV, T_A_P1, 0.2),
            pr!("Echo Cascade";
                A_P1 => SINE, B_Type => 2, B_Level => 0.35, B_P1 => FM_4, B_P2 => 0.2, AmpA => 0.002, AmpD => 0.4, AmpS => 0.0, AmpR => 0.3, Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Oct => 3, Arp_Gate => 0.3,
                Dly_Mix => 0.5, Dly_Time => 375.0, Dly_Fb => 0.65, Dly_Ping => 1, Rev_Mix => 0.4, Rev_Decay => 4.0),
            pr!("Down Spiral";
                A_P1 => SAW, B_Level => 0.4, B_P1 => PULSE, B_P2 => 0.4, B_Coarse => 12, F1_Type => 1, F1_Cut => 2500.0, F1_Res => 0.3, AmpA => 0.002, AmpD => 0.2, AmpS => 0.0, AmpR => 0.1,
                Arp_On => 1, Arp_Mode => 1, Arp_Rate => 3, Arp_Oct => 3, Arp_Gate => 0.45, Dly_Mix => 0.3, Dly_Time => 300.0),
            pr!("Scale Arp";
                A_P1 => TRI, B_Level => 0.5, B_P1 => SAW, B_Coarse => 12, F1_Type => 0, F1_Cut => 4500.0, AmpA => 0.002, AmpD => 0.3, AmpS => 0.0, AmpR => 0.2, Scale => 1, Chord => 17,
                Arp_On => 1, Arp_Mode => 2, Arp_Rate => 3, Arp_Oct => 2, Arp_Gate => 0.5, Dly_Mix => 0.3, Dly_Time => 330.0, Rev_Mix => 0.3, Voices => 14),
        ],
    )
}
