//! Retro: chip, console, 80s synth and early rave sounds.

use super::*;

const MACROS: MacroSet = [
    ("Bright", (MD_CUTOFF, 0.5), (MD_RES, 0.1)),
    ("Crunch", (MD_DRIVE, 0.6), (MD_A2, 0.3)),
    ("Wobble", (MD_LFO1, 0.45), (MD_DETUNE, 0.3)),
    ("Echo", (MD_DELAY, 0.3), (MD_CHORUS, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Retro",
        &MACROS,
        vec![
            pr!("Chiptune";
                A_P1 => PULSE, A_P2 => 0.25, AmpS => 1.0, AmpR => 0.05, Drv_Type => 4, Drv_Amt => 0.3, Drv_Mix => 1.0,
                Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Gate => 0.6, F1_Cut => 14000.0),
            pr!("Rave Stab";
                A_P1 => SAW, Unison => 5, UniDetune => 0.5, UniSpread => 0.8, F1_Cut => 1800.0, F1_Res => 0.3, F1_Env => 0.5,
                E2A => 0.002, E2D => 0.2, E2S => 0.1, AmpD => 0.3, AmpS => 0.0, AmpR => 0.2, Rev_Mix => 0.2, Dly_Mix => 0.25, Dly_Time => 250.0),
            pr!("Dub Chord";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Coarse => 7, F1_Type => 0, F1_Cut => 1200.0, F1_Res => 0.3, AmpA => 0.01, AmpD => 0.4, AmpS => 0.5, AmpR => 0.3,
                Dly_Mix => 0.5, Dly_Time => 500.0, Dly_Fb => 0.6, Rev_Mix => 0.3),
            pr!("8-bit Bass";
                A_P1 => TRI, Mode => 1, F1_Cut => 10000.0, AmpA => 0.001, AmpD => 0.15, AmpS => 0.8, AmpR => 0.05, Drv_Type => 4, Drv_Amt => 0.45, Drv_Mix => 1.0),
            pr!("NES Lead";
                A_P1 => PULSE, A_P2 => 0.125, Mode => 2, AmpA => 0.001, AmpS => 0.9, AmpR => 0.03, L1_Rate => 6.0, L1_Fade => 0.3, Drv_Type => 4, Drv_Amt => 0.4, Drv_Mix => 1.0)
                .m(1, SRC_LFO1, T_PITCH, 0.003),
            pr!("C64 Pulse";
                A_P1 => PULSE, A_P2 => 0.5, F1_Type => 1, F1_Cut => 2500.0, F1_Res => 0.4, F1_Env => 0.3, E2D => 0.2, E2S => 0.3, AmpA => 0.002, AmpD => 0.2, AmpS => 0.7, AmpR => 0.06,
                L1_Rate => 1.5, Drv_Type => 1, Drv_Amt => 0.2)
                .m(1, SRC_LFO1, T_A_P1 + 1, 0.4),
            pr!("Gameboy Wave";
                A_P1 => TRI, A_P3 => 0.2, Mode => 2, AmpA => 0.001, AmpS => 1.0, AmpR => 0.04, Drv_Type => 4, Drv_Amt => 0.6, Drv_Mix => 1.0, F1_Cut => 7000.0),
            pr!("Synthwave Lead";
                A_P1 => SAW, B_Level => 0.8, B_P1 => SAW, B_Fine => 10.0, Unison => 3, UniDetune => 0.25, Mode => 2, Glide => 0.05, F1_Type => 1, F1_Cut => 4500.0, F1_Res => 0.15, AmpA => 0.005, AmpS => 0.9, AmpR => 0.3,
                L1_Rate => 5.0, L1_Fade => 0.8, Cho_Mix => 0.4, Dly_Mix => 0.3, Dly_Time => 375.0, Dly_Fb => 0.45, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.003),
            pr!("Synthwave Bass";
                A_P1 => SAW, B_Level => 0.6, B_P1 => PULSE, B_P2 => 0.4, B_Coarse => -12, Mode => 1, F1_Type => 1, F1_Cut => 700.0, F1_Res => 0.2, F1_Env => 0.35, E2D => 0.2, E2S => 0.4,
                AmpA => 0.002, AmpD => 0.2, AmpS => 0.8, AmpR => 0.1, Arp_On => 1, Arp_Mode => 0, Arp_Rate => 3, Arp_Gate => 0.7, Drv_Type => 1, Drv_Amt => 0.2),
            pr!("Vaporwave Pad";
                A_P1 => SAW, B_Level => 0.6, B_P1 => SAW, B_Fine => 12.0, Unison => 3, UniDetune => 0.4, Drift => 0.7, F1_Type => 0, F1_Cut => 1400.0, AmpA => 1.2, AmpS => 1.0, AmpR => 2.0,
                Cho_Mix => 0.7, Cho_Rate => 0.2, Cho_Depth => 0.8, Drv_Type => 4, Drv_Amt => 0.1, Drv_Mix => 0.3, Rev_Mix => 0.5, Rev_Decay => 6.0),
            pr!("Sega FM Lead";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.5, A_P3 => 0.3, B_Type => 2, B_Level => 0.35, B_P1 => FM_2, B_P2 => 0.35, B_Fine => 5.0, Mode => 2, Glide => 0.03, AmpA => 0.002, AmpD => 0.3, AmpS => 0.8, AmpR => 0.1,
                Drv_Type => 4, Drv_Amt => 0.2, Drv_Mix => 0.5)
                .m(1, SRC_ENV3, T_A_P1 + 1, 0.2),
            pr!("DX Bass 80s";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.4, A_P3 => 0.2, B_Level => 0.3, B_P1 => SINE, B_Coarse => -12, Mode => 1, AmpA => 0.001, AmpD => 0.4, AmpS => 0.6, AmpR => 0.1, E3D => 0.15, E3S => 0.0,
                Cho_Mix => 0.2)
                .m(1, SRC_ENV3, T_A_P1 + 1, 0.3),
            pr!("Juno Chord";
                A_P1 => SAW, B_Level => 0.6, B_P1 => PULSE, B_P2 => 0.5, B_Coarse => -12, Unison => 2, UniDetune => 0.2, F1_Type => 1, F1_Cut => 2800.0, AmpA => 0.02, AmpD => 0.4, AmpS => 0.7, AmpR => 0.5,
                Cho_Mix => 0.8, Cho_Rate => 0.6, Cho_Depth => 0.6, Rev_Mix => 0.2),
            pr!("Italo Stab";
                A_P1 => SAW, B_Level => 0.7, B_P1 => PULSE, B_P2 => 0.4, B_Fine => 8.0, Unison => 2, UniDetune => 0.2, F1_Type => 1, F1_Cut => 1500.0, F1_Env => 0.4, E2D => 0.25, E2S => 0.1, AmpD => 0.35, AmpS => 0.0, AmpR => 0.2,
                Cho_Mix => 0.3, Dly_Mix => 0.25, Dly_Time => 300.0, Rev_Mix => 0.25),
            pr!("Eurodance Lead";
                A_P1 => SAW, B_Level => 0.8, B_P1 => PULSE, B_P2 => 0.3, B_Coarse => 12, Mode => 2, Glide => 0.02, F1_Type => 1, F1_Cut => 5500.0, F1_Res => 0.2, AmpS => 0.9, AmpR => 0.15,
                Drv_Type => 1, Drv_Amt => 0.25, Dly_Mix => 0.2, Dly_Time => 280.0, Rev_Mix => 0.2),
            pr!("Hoover Rave";
                A_P1 => PULSE, A_P2 => 0.3, B_Level => 0.9, B_P1 => SAW, B_Coarse => -12, C_Level => 0.5, C_P1 => PULSE, C_P2 => 0.45, Unison => 5, UniDetune => 0.8, UniSpread => 1.0, Mode => 2, Glide => 0.15,
                F1_Type => 1, F1_Cut => 2400.0, F1_Res => 0.3, AmpS => 1.0, AmpR => 0.3, Drv_Type => 2, Drv_Amt => 0.4, Drv_Mix => 0.6, Cho_Mix => 0.3)
                .m(1, SRC_LFO2, T_PITCH, 0.003),
        ],
    )
}
