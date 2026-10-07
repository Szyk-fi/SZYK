//! FX: risers, drops, lasers and other sound effects.

use super::*;

const MACROS: MacroSet = [
    ("Sweep", (MD_CUTOFF, 0.55), (MD_RES, 0.2)),
    ("Grit", (MD_DRIVE, 0.6), (MD_RING, 0.3)),
    ("Speed", (MD_LFO1, 0.5), (MD_LFO2, 0.4)),
    ("Space", (MD_REVERB, 0.35), (MD_DELAY, 0.3)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "FX",
        &MACROS,
        vec![
            pr!("Noise Sweep";
                A_Type => 4, A_P1 => 0.3, B_Level => 0.0, F1_Type => 4, F1_Cut => 200.0, F1_Res => 0.55, F1_Env => 0.9, E2A => 3.0, E2D => 0.5, E2S => 1.0,
                AmpA => 2.5, AmpS => 1.0, AmpR => 1.5, Rev_Mix => 0.5, Rev_Decay => 5.0),
            pr!("Riser";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 15.0, Unison => 5, UniDetune => 0.5, Noise => 0.3, F1_Type => 1, F1_Cut => 200.0, F1_Res => 0.3, F1_Env => 0.9, E2A => 6.0, E2D => 1.0, E2S => 1.0,
                AmpA => 4.0, AmpS => 1.0, AmpR => 1.0, Rev_Mix => 0.4, Rev_Decay => 4.0)
                .m(1, SRC_ENV2, T_PITCH, 0.35),
            pr!("Downlifter";
                A_P1 => SAW, B_Level => 0.7, B_P1 => SAW, B_Fine => 12.0, Noise => 0.3, F1_Type => 1, F1_Cut => 6000.0, F1_Res => 0.3, F1_Env => -0.8, E2A => 0.01, E2D => 3.0, E2S => 0.0,
                AmpA => 0.01, AmpD => 3.0, AmpS => 0.0, AmpR => 0.5, Rev_Mix => 0.4)
                .m(1, SRC_ENV2, T_PITCH, -0.5),
            pr!("Laser";
                A_P1 => SAW, F1_Type => 1, F1_Cut => 8000.0, F1_Res => 0.4, AmpA => 0.001, AmpD => 0.35, AmpS => 0.0, AmpR => 0.1, E3A => 0.0005, E3D => 0.3, E3S => 0.0, Dly_Mix => 0.2, Dly_Time => 120.0, Dly_Fb => 0.5)
                .m(1, SRC_ENV3, T_PITCH, 1.0),
            pr!("Zap";
                A_P1 => PULSE, A_P2 => 0.2, B_P1 => SINE, B_Level => 0.5, B_Coarse => 12, Ring => 0.4, AmpA => 0.001, AmpD => 0.2, AmpS => 0.0, AmpR => 0.05, E3D => 0.12, E3S => 0.0, Drv_Type => 2, Drv_Amt => 0.4)
                .m(1, SRC_ENV3, T_PITCH, 1.0),
            pr!("Alarm";
                A_P1 => PULSE, A_P2 => 0.5, B_Level => 0.4, B_P1 => SAW, B_Coarse => 12, Mode => 1, F1_Type => 0, F1_Cut => 3000.0, AmpA => 0.01, AmpS => 1.0, AmpR => 0.05, L1_Shape => 3, L1_Rate => 3.0,
                Drv_Type => 2, Drv_Amt => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.25),
            pr!("Sci-Fi Computer";
                A_Type => 2, A_P1 => FM_5, A_P2 => 0.6, A_P3 => 0.4, B_Type => 2, B_Level => 0.4, B_P1 => FM_10, B_P2 => 0.5, AmpA => 0.002, AmpD => 0.2, AmpS => 0.0, AmpR => 0.1, L1_Shape => 4, L1_Rate => 12.0,
                Arp_On => 1, Arp_Mode => 3, Arp_Rate => 4, Arp_Oct => 3, Arp_Gate => 0.3, Dly_Mix => 0.35, Dly_Time => 180.0, Dly_Fb => 0.55, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_A_P1, 0.3),
            pr!("Siren";
                A_P1 => SAW, B_P1 => SINE, B_Level => 0.4, Mode => 1, F1_Type => 0, F1_Cut => 4000.0, AmpA => 0.1, AmpS => 1.0, AmpR => 0.4, L1_Shape => 1, L1_Rate => 0.5, Rev_Mix => 0.3)
                .m(1, SRC_LFO1, T_PITCH, 0.4),
            pr!("Impact";
                A_P1 => SINE, A_Coarse => -24, B_Type => 4, B_Level => 0.6, B_P1 => 0.8, SubLevel => 0.5, F1_Type => 1, F1_Cut => 2000.0, F1_Env => -0.3, E2D => 1.5, E2S => 0.0,
                AmpA => 0.001, AmpD => 2.5, AmpS => 0.0, AmpR => 1.0, E3D => 0.15, E3S => 0.0, Drv_Type => 1, Drv_Amt => 0.4, Rev_Mix => 0.5, Rev_Decay => 6.0)
                .m(1, SRC_ENV3, T_PITCH, 0.6),
            pr!("Whoosh";
                A_Type => 4, A_P1 => 0.2, F1_Type => 4, F1_Cut => 300.0, F1_Res => 0.45, F1_Env => 0.9, E2A => 0.8, E2D => 0.8, E2S => 0.0, AmpA => 0.6, AmpD => 1.0, AmpS => 0.0, AmpR => 0.5,
                Rev_Mix => 0.35, Ph_Mix => 0.3),
            pr!("Glitch";
                A_Type => 2, A_P1 => FM_6, A_P2 => 0.7, A_P3 => 0.5, B_Type => 4, B_Level => 0.3, B_P1 => 0.0, B_P2 => 0.8, AmpA => 0.001, AmpD => 0.08, AmpS => 0.0, AmpR => 0.03, Drv_Type => 4, Drv_Amt => 0.7,
                Arp_On => 1, Arp_Mode => 3, Arp_Rate => 5, Arp_Oct => 4, Arp_Gate => 0.3, Dly_Mix => 0.3, Dly_Time => 125.0, Dly_Fb => 0.5),
            pr!("Radio Static";
                A_Type => 4, A_P1 => 0.0, A_P2 => 0.4, F1_Type => 4, F1_Cut => 2500.0, F1_Res => 0.3, AmpA => 0.05, AmpS => 1.0, AmpR => 0.2, L1_Shape => 4, L1_Rate => 9.0, Drv_Type => 4, Drv_Amt => 0.5)
                .m(1, SRC_LFO1, T_F1_CUT, 0.4),
            pr!("Sub Drop";
                A_P1 => SINE, A_Coarse => -12, SubLevel => 0.4, AmpA => 0.001, AmpD => 2.5, AmpS => 0.0, AmpR => 0.5, E3A => 0.0005, E3D => 2.0, E3S => 0.0, F1_Cut => 2500.0, Rev_Mix => 0.2)
                .m(1, SRC_ENV3, T_PITCH, 0.7),
            pr!("Robot Voice";
                A_P1 => SAW, B_P1 => PULSE, B_P2 => 0.5, B_Level => 0.5, Ring => 0.5, Mode => 2, Glide => 0.02, F1_Type => 4, F1_Cut => 1500.0, F1_Res => 0.5, L1_Shape => 3, L1_Rate => 7.0, AmpA => 0.005, AmpS => 1.0, AmpR => 0.1,
                Drv_Type => 2, Drv_Amt => 0.3)
                .m(1, SRC_LFO1, T_F1_CUT, 0.3),
            pr!("Spaceship Engine";
                A_P1 => SAW, A_Coarse => -24, B_Type => 4, B_Level => 0.4, B_P1 => 0.6, F1_Type => 1, F1_Cut => 400.0, F1_Res => 0.4, AmpA => 1.5, AmpS => 1.0, AmpR => 2.0, L1_Rate => 7.0, L2_Rate => 0.2,
                Ph_Mix => 0.5, Ph_Rate => 0.15, Rev_Mix => 0.4)
                .m(1, SRC_LFO1, T_F1_CUT, 0.15).m(2, SRC_LFO2, T_F1_CUT, 0.3),
            pr!("Telephone Ring";
                A_P1 => SINE, A_Coarse => 12, B_P1 => SINE, B_Level => 0.8, B_Coarse => 15, Mode => 1, AmpA => 0.005, AmpS => 1.0, AmpR => 0.02, L1_Shape => 3, L1_Rate => 16.0, F1_Cut => 5000.0)
                .m(1, SRC_LFO1, T_AMP, -0.9),
        ],
    )
}
