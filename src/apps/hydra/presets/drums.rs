//! Drums: synthesized percussion, played from the keys or pads. Pitch
//! matters little here; each sound sets its own octave.

use super::*;

const MACROS: MacroSet = [
    ("Tone", (MD_CUTOFF, 0.45), (MD_A1, 0.3)),
    ("Decay", (MD_DECAY, 0.5), (MD_RELEASE, 0.3)),
    ("Punch", (MD_DRIVE, 0.6), (MD_NOISE, 0.15)),
    ("Room", (MD_REVERB, 0.35), (MD_DELAY, 0.15)),
];

pub fn presets() -> Vec<Preset> {
    use P::*;
    finish(
        "Drums",
        &MACROS,
        vec![
            pr!("Kick";
                A_P1 => SINE, Octave => -2, F1_Cut => 6000.0, AmpA => 0.001, AmpD => 0.35, AmpS => 0.0, AmpR => 0.1, E3A => 0.0005, E3D => 0.07, E3S => 0.0, Mode => 1, VelSens => 0.5,
                Drv_Type => 1, Drv_Amt => 0.3, Drv_Mix => 0.7)
                .m(1, SRC_ENV3, T_PITCH, 0.8),
            pr!("Punch Kick";
                A_P1 => SINE, Octave => -2, B_Type => 4, B_Level => 0.0, B_P1 => 0.0, F1_Cut => 5000.0, AmpA => 0.001, AmpD => 0.3, AmpS => 0.0, AmpR => 0.08, E3D => 0.05, E3S => 0.0, Mode => 1,
                Drv_Type => 1, Drv_Amt => 0.45, Drv_Mix => 0.8)
                .m(1, SRC_ENV3, T_PITCH, 1.0).m(2, SRC_ENV3, T_NOISE, 0.4),
            pr!("808 Kick";
                A_P1 => SINE, Octave => -2, F1_Cut => 3000.0, AmpA => 0.001, AmpD => 1.3, AmpS => 0.0, AmpR => 0.3, E3D => 0.1, E3S => 0.0, Mode => 1, Drv_Type => 1, Drv_Amt => 0.25)
                .m(1, SRC_ENV3, T_PITCH, 0.55),
            pr!("Snare";
                A_P1 => TRI, Octave => -1, Noise => 0.7, NoiseColor => 0.1, F1_Type => 0, F1_Cut => 9000.0, AmpA => 0.001, AmpD => 0.2, AmpS => 0.0, AmpR => 0.08, E3D => 0.04, E3S => 0.0,
                Drv_Type => 1, Drv_Amt => 0.3, Rev_Mix => 0.12, Rev_Decay => 1.0)
                .m(1, SRC_ENV3, T_PITCH, 0.4),
            pr!("Clap";
                A_Type => 4, A_P1 => 0.05, F1_Type => 4, F1_Cut => 1400.0, F1_Res => 0.3, AmpA => 0.001, AmpD => 0.17, AmpS => 0.0, AmpR => 0.1, Rev_Mix => 0.25, Rev_Decay => 1.5,
                Drv_Type => 1, Drv_Amt => 0.2),
            pr!("Closed Hat";
                A_Type => 4, A_P1 => 0.0, F1_Type => 3, F1_Cut => 7500.0, F1_Res => 0.1, AmpA => 0.001, AmpD => 0.05, AmpS => 0.0, AmpR => 0.03),
            pr!("Open Hat";
                A_Type => 4, A_P1 => 0.0, F1_Type => 3, F1_Cut => 7000.0, F1_Res => 0.1, AmpA => 0.001, AmpD => 0.35, AmpS => 0.0, AmpR => 0.2, Rev_Mix => 0.1),
            pr!("Tom";
                A_P1 => SINE, Octave => -1, F1_Cut => 5000.0, AmpA => 0.001, AmpD => 0.45, AmpS => 0.0, AmpR => 0.15, E3D => 0.12, E3S => 0.0, Mode => 1, Rev_Mix => 0.15)
                .m(1, SRC_ENV3, T_PITCH, 0.35),
            pr!("Rimshot";
                A_P1 => TRI, Octave => 1, B_Level => 0.5, B_P1 => SINE, B_Coarse => 7, Noise => 0.3, F1_Type => 4, F1_Cut => 2500.0, F1_Res => 0.3, AmpA => 0.0005, AmpD => 0.06, AmpS => 0.0, AmpR => 0.03),
            pr!("Cowbell";
                A_P1 => PULSE, A_P2 => 0.5, Octave => 0, B_Level => 0.8, B_P1 => PULSE, B_P2 => 0.5, B_Coarse => 7, B_Fine => -3.0, F1_Type => 4, F1_Cut => 2400.0, F1_Res => 0.25,
                AmpA => 0.001, AmpD => 0.3, AmpS => 0.0, AmpR => 0.15),
            pr!("Conga";
                A_P1 => SINE, Octave => 0, B_Level => 0.3, B_P1 => TRI, F1_Cut => 4000.0, AmpA => 0.001, AmpD => 0.25, AmpS => 0.0, AmpR => 0.1, E3D => 0.04, E3S => 0.0, Mode => 1)
                .m(1, SRC_ENV3, T_PITCH, 0.2),
            pr!("Clave";
                A_P1 => SINE, Octave => 2, B_Type => 2, B_Level => 0.3, B_P1 => FM_3_5, B_P2 => 0.2, AmpA => 0.0005, AmpD => 0.07, AmpS => 0.0, AmpR => 0.04, Rev_Mix => 0.1),
            pr!("Shaker";
                A_Type => 4, A_P1 => 0.0, F1_Type => 4, F1_Cut => 6000.0, F1_Res => 0.15, AmpA => 0.015, AmpD => 0.09, AmpS => 0.0, AmpR => 0.05),
            pr!("Crash";
                A_Type => 4, A_P1 => 0.0, B_Type => 2, B_Level => 0.3, B_P1 => FM_10, B_P2 => 0.8, B_P3 => 0.3, F1_Type => 3, F1_Cut => 5000.0, AmpA => 0.002, AmpD => 1.6, AmpS => 0.0, AmpR => 0.6,
                Rev_Mix => 0.25, Rev_Decay => 2.5),
            pr!("Metal Hit";
                A_Type => 2, A_P1 => 0.8, A_P2 => 0.9, A_P3 => 0.3, B_Type => 4, B_Level => 0.3, B_P1 => 0.2, B_P3 => 0.5, Ring => 0.2,
                AmpD => 0.4, AmpS => 0.0, AmpR => 0.3, Rev_Mix => 0.3),
            pr!("Wood Block";
                A_Type => 2, A_P1 => FM_1, A_P2 => 0.3, B_Type => 2, B_Level => 0.3, B_P1 => FM_5, B_P2 => 0.2, F1_Type => 4, F1_Cut => 1800.0, AmpA => 0.0005, AmpD => 0.09, AmpS => 0.0, AmpR => 0.05,
                Rev_Mix => 0.12),
        ],
    )
}
