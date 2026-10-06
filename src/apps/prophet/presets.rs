use super::spec::*;

#[derive(Clone)]
pub struct Preset {
    pub name: String,
    pub bank: String,
    pub values: [f32; N],
}

const NAMES: [[&str; 8]; 8] = [
    [
        "Walnut Brass",
        "Soft Horns",
        "Fanfare Five",
        "Muted Trumpet",
        "Toto Sunrise",
        "Low Brass",
        "Golden Stabs",
        "Cinema Horns",
    ],
    [
        "Velvet Strings",
        "Slow Orchestra",
        "Silk Ensemble",
        "Solstice Pad",
        "Warm Tape",
        "Night Choir",
        "Fifth Dimension",
        "Frozen Glass",
    ],
    [
        "Roundwood Bass",
        "Rubber Pulse",
        "Octave Bass",
        "Low Voltage",
        "Resonant Thumb",
        "Unison Weight",
        "Dark Triangle",
        "Acid Timber",
    ],
    [
        "Ribbon Lead",
        "Sync Skyline",
        "Pulse Solo",
        "Fifth Avenue",
        "Portamento Gold",
        "Reedy Mono",
        "Wide Unison",
        "Singing Saw",
    ],
    [
        "Wooden Tines",
        "Copper Clav",
        "Analog Harp",
        "Short Circuit",
        "Soft Mallet",
        "Glass Keys",
        "Midnight Piano",
        "Rubber Marimba",
    ],
    [
        "Poly Bell",
        "Crossmod Chime",
        "Metal Bloom",
        "Formant Wire",
        "B Low Drone",
        "Circuit Gong",
        "Sync Brass",
        "Broken Radio",
    ],
    [
        "PWM Clouds",
        "Lighthouse",
        "Random Tide",
        "Pulsing Amber",
        "Slow Sweep",
        "Square Orbit",
        "Noise Horizon",
        "Afterglow",
    ],
    [
        "Classic Saw",
        "Twin Squares",
        "Triangle Reed",
        "Seventies Organ",
        "Noise Snare",
        "Analog Kick",
        "Ocean Wind",
        "Init Patch",
    ],
];
pub const BANK_NAMES: [&str; 8] = [
    "BRASS",
    "STRINGS",
    "BASS",
    "LEADS",
    "KEYS",
    "POLY MOD",
    "MOTION",
    "ESSENTIALS",
];

pub fn factory() -> Vec<Preset> {
    let mut out = Vec::with_capacity(64);
    for bank in 0..8 {
        for slot in 0..8 {
            let mut p = std::array::from_fn(|i| SPECS[i].default);
            let mut set = |pairs: &[(usize, f32)]| {
                for &(i, v) in pairs {
                    p[i] = v;
                }
            };
            match bank {
                0 => {
                    set(&[
                        (
                            CUTOFF,
                            cutoff_norm([700., 480., 1150., 650., 900., 360., 1500., 580.][slot]),
                        ),
                        (RES, [0.12, 0.08, 0.2, 0.32, 0.16, 0.14, 0.24, 0.1][slot]),
                        (ENV_AMT, [0.38, 0.28, 0.5, 0.22, 0.45, 0.3, 0.5, 0.35][slot]),
                        (
                            FA,
                            time_norm([0.018, 0.055, 0.008, 0.025, 0.04, 0.09, 0.002, 0.16][slot]),
                        ),
                        (
                            AA,
                            time_norm([0.015, 0.05, 0.006, 0.012, 0.035, 0.05, 0.003, 0.12][slot]),
                        ),
                        (
                            FD,
                            time_norm([0.45, 0.8, 0.3, 0.55, 0.6, 0.9, 0.22, 1.2][slot]),
                        ),
                        (FS, 0.35),
                        (
                            AR,
                            time_norm([0.22, 0.45, 0.15, 0.18, 0.4, 0.6, 0.12, 1.0][slot]),
                        ),
                        (B_FINE, [4., -7., 8., 2., 6., -4., 10., 9.][slot]),
                        (CHORUS, if slot == 7 { 0.25 } else { 0.0 }),
                    ]);
                    if slot == 5 {
                        p[A_TUNE] = -12.;
                        p[B_TUNE] = -12.;
                    }
                    if slot == 3 {
                        p[B_SAW] = 0.;
                        p[B_PULSE] = 1.;
                        p[B_PW] = 0.32;
                    }
                }
                1 => {
                    set(&[
                        (
                            CUTOFF,
                            cutoff_norm(
                                [1800., 1100., 2400., 950., 1600., 650., 2100., 4000.][slot],
                            ),
                        ),
                        (ENV_AMT, 0.16),
                        (
                            FA,
                            time_norm([0.6, 1.4, 0.28, 2.0, 0.45, 0.8, 1.0, 0.18][slot]),
                        ),
                        (
                            AA,
                            time_norm([0.55, 1.6, 0.3, 2.2, 0.5, 0.9, 1.2, 0.2][slot]),
                        ),
                        (
                            AR,
                            time_norm([1.4, 2.4, 0.9, 3.6, 1.2, 2.0, 2.8, 3.0][slot]),
                        ),
                        (FR, time_norm(2.0)),
                        (AS, 0.88),
                        (FS, 0.6),
                        (B_FINE, [8., -11., 6., 14., -5., 3., 9., 2.][slot]),
                        (
                            CHORUS,
                            [0.32, 0.22, 0.45, 0.35, 0.28, 0.4, 0.25, 0.18][slot],
                        ),
                        (SPREAD, 0.85),
                    ]);
                    if slot == 5 {
                        p[RES] = 0.42;
                        p[B_SAW] = 0.;
                        p[B_PULSE] = 1.;
                        p[B_PW] = 0.24;
                    }
                    if slot == 6 {
                        p[B_TUNE] = 7.;
                    }
                    if slot == 7 {
                        p[A_SAW] = 0.;
                        p[A_PULSE] = 1.;
                        p[POLY_B] = 0.08;
                        p[P_FREQ] = 1.;
                    }
                }
                2 => {
                    set(&[
                        (A_TUNE, -12.),
                        (B_TUNE, -12.),
                        (MODE, 1.),
                        (SPREAD, 0.),
                        (
                            CUTOFF,
                            cutoff_norm([420., 550., 680., 180., 350., 600., 800., 260.][slot]),
                        ),
                        (
                            ENV_AMT,
                            [0.3, 0.44, 0.42, 0.18, 0.55, 0.32, 0.1, 0.65][slot],
                        ),
                        (RES, [0.12, 0.28, 0.15, 0.1, 0.6, 0.18, 0.04, 0.78][slot]),
                        (FA, time_norm(0.002)),
                        (AA, time_norm(0.003)),
                        (
                            FD,
                            time_norm([0.24, 0.18, 0.32, 0.5, 0.16, 0.38, 0.55, 0.12][slot]),
                        ),
                        (FS, 0.05),
                        (AS, 0.65),
                        (AR, time_norm(0.11)),
                        (FR, time_norm(0.08)),
                        (DRIVE, 0.32),
                    ]);
                    if slot == 1 || slot == 7 {
                        p[A_SAW] = 0.;
                        p[A_PULSE] = 1.;
                        p[A_PW] = 0.36;
                    }
                    if slot == 2 {
                        p[B_TUNE] = -24.;
                    }
                    if slot == 5 {
                        p[MODE] = 2.;
                        p[UNISON_DETUNE] = 0.08;
                    }
                    if slot == 6 {
                        p[MIX_A] = 0.;
                        p[B_SAW] = 0.;
                        p[B_TRI] = 1.;
                    }
                }
                3 => {
                    set(&[
                        (MODE, 1.),
                        (
                            CUTOFF,
                            cutoff_norm(
                                [2100., 900., 3200., 1800., 1600., 1400., 2800., 3800.][slot],
                            ),
                        ),
                        (ENV_AMT, 0.24),
                        (AA, time_norm(0.006)),
                        (FA, time_norm(0.004)),
                        (AS, 0.82),
                        (AR, time_norm(0.23)),
                        (
                            GLIDE,
                            [0.16, 0.08, 0.06, 0.12, 0.36, 0.07, 0.18, 0.14][slot],
                        ),
                        (DELAY_MIX, 0.18),
                        (DELAY_FB, 0.3),
                        (B_FINE, 7.),
                    ]);
                    if slot == 1 {
                        p[SYNC] = 1.;
                        p[A_TUNE] = 12.;
                        p[POLY_ENV] = 0.48;
                        p[P_FREQ] = 1.;
                    }
                    if slot == 2 || slot == 5 {
                        p[A_SAW] = 0.;
                        p[A_PULSE] = 1.;
                        p[A_PW] = if slot == 2 { 0.3 } else { 0.17 };
                    }
                    if slot == 3 {
                        p[B_TUNE] = 7.;
                    }
                    if slot == 6 {
                        p[MODE] = 2.;
                        p[UNISON_DETUNE] = 0.32;
                        p[SPREAD] = 0.9;
                    }
                    if slot == 7 {
                        p[MOD_AMOUNT] = 0.04;
                        p[LFO_RATE] = 0.8;
                    }
                }
                4 => {
                    set(&[
                        (
                            CUTOFF,
                            cutoff_norm([800., 1800., 1300., 2700., 600., 3000., 900., 720.][slot]),
                        ),
                        (ENV_AMT, [0.35, 0.48, 0.4, 0.6, 0.18, 0.25, 0.3, 0.55][slot]),
                        (FA, time_norm(0.001)),
                        (AA, time_norm(0.002)),
                        (
                            FD,
                            time_norm([0.38, 0.13, 0.46, 0.09, 0.6, 0.8, 0.55, 0.21][slot]),
                        ),
                        (
                            AD,
                            time_norm([0.65, 0.19, 0.85, 0.12, 0.9, 1.4, 1.1, 0.33][slot]),
                        ),
                        (FS, 0.),
                        (AS, 0.),
                        (AR, time_norm(0.28)),
                        (FR, time_norm(0.2)),
                        (VELOCITY, 0.8),
                        (DELAY_MIX, if slot == 2 { 0.24 } else { 0.0 }),
                    ]);
                    if slot == 1 {
                        p[A_SAW] = 0.;
                        p[A_PULSE] = 1.;
                        p[A_PW] = 0.22;
                        p[RES] = 0.38;
                    }
                    if slot == 4 || slot == 7 {
                        p[MIX_A] = 0.;
                        p[B_SAW] = 0.;
                        p[B_TRI] = 1.;
                        p[RES] = 0.35;
                    }
                    if slot == 5 {
                        p[B_TUNE] = 12.;
                        p[POLY_B] = 0.09;
                        p[P_FREQ] = 1.;
                        p[B_SAW] = 0.;
                        p[B_TRI] = 1.;
                    }
                }
                5 => {
                    set(&[
                        (
                            CUTOFF,
                            cutoff_norm(
                                [6500., 9000., 3500., 2200., 600., 5200., 1200., 4000.][slot],
                            ),
                        ),
                        (POLY_B, [0.14, 0.25, 0.42, 0.18, 0.65, 0.5, 0.1, 0.72][slot]),
                        (P_FREQ, 1.),
                        (B_TUNE, [19., 7., 15., -7., -12., 23., 0., -17.][slot]),
                        (B_SAW, 0.),
                        (B_TRI, 1.),
                        (MIX_B, 0.0),
                        (AA, time_norm(0.002)),
                        (AD, time_norm(1.8)),
                        (AS, 0.),
                        (AR, time_norm(1.4)),
                        (POLY_ENV, [0., 0.12, 0.3, 0.0, 0.2, 0.45, 0.6, 0.0][slot]),
                        (FS, 0.0),
                        (FD, time_norm(0.5)),
                    ]);
                    if slot == 3 {
                        p[P_FILTER] = 1.;
                        p[RES] = 0.55;
                    }
                    if slot == 4 {
                        p[B_LOW] = 1.;
                        p[AS] = 0.8;
                        p[AA] = time_norm(0.3);
                        p[P_PW] = 1.;
                        p[A_PULSE] = 1.;
                    }
                    if slot == 6 {
                        p[SYNC] = 1.;
                        p[A_TUNE] = 12.;
                        p[POLY_B] = 0.;
                        p[AS] = 0.7;
                        p[AD] = time_norm(0.4);
                    }
                    if slot == 7 {
                        p[B_SAW] = 1.;
                        p[B_TRI] = 0.;
                        p[P_FILTER] = 1.;
                        p[NOISE] = 0.25;
                        p[MOD_AMOUNT] = 0.3;
                        p[LFO_SHAPE] = 4.;
                    }
                }
                6 => {
                    set(&[
                        (
                            CUTOFF,
                            cutoff_norm(
                                [1800., 1100., 2200., 900., 700., 1500., 1800., 1400.][slot],
                            ),
                        ),
                        (AA, time_norm(0.4)),
                        (AS, 0.85),
                        (AR, time_norm(1.6)),
                        (
                            MOD_AMOUNT,
                            [0.3, 0.32, 0.4, 0.24, 0.42, 0.28, 0.32, 0.18][slot],
                        ),
                        (W_FREQ, 0.),
                        (W_FILTER, 1.),
                        (
                            LFO_RATE,
                            [0.3, 0.5, 0.42, 0.7, 0.22, 0.55, 0.18, 0.25][slot],
                        ),
                        (LFO_SHAPE, [0., 3., 4., 2., 0., 2., 4., 3.][slot]),
                        (CHORUS, 0.25),
                        (DELAY_MIX, 0.22),
                        (DELAY_FB, 0.38),
                        (SPREAD, 0.85),
                    ]);
                    if slot == 0 {
                        p[A_PULSE] = 1.;
                        p[A_SAW] = 0.;
                        p[B_PULSE] = 1.;
                        p[B_SAW] = 0.;
                        p[W_PW] = 1.;
                        p[W_FILTER] = 0.;
                    }
                    if slot == 3 {
                        p[AA] = time_norm(0.003);
                        p[AR] = time_norm(0.2);
                        p[RES] = 0.48;
                    }
                    if slot == 6 {
                        p[NOISE] = 0.5;
                        p[MIX_A] = 0.25;
                        p[MIX_B] = 0.25;
                        p[WHEEL_NOISE] = 0.35;
                    }
                }
                _ => {
                    set(&[
                        (CUTOFF, cutoff_norm(5500.)),
                        (ENV_AMT, 0.),
                        (CHORUS, 0.),
                        (B_FINE, 0.),
                    ]);
                    match slot {
                        0 => {
                            p[MIX_B] = 0.;
                        }
                        1 => {
                            p[A_SAW] = 0.;
                            p[A_PULSE] = 1.;
                            p[B_SAW] = 0.;
                            p[B_PULSE] = 1.;
                            p[B_FINE] = 6.;
                        }
                        2 => {
                            p[MIX_A] = 0.;
                            p[B_SAW] = 0.;
                            p[B_TRI] = 1.;
                        }
                        3 => {
                            p[B_TUNE] = 12.;
                            p[A_PULSE] = 1.;
                            p[A_SAW] = 0.;
                            p[AR] = time_norm(0.05);
                        }
                        4 => {
                            p[MIX_A] = 0.;
                            p[MIX_B] = 0.;
                            p[NOISE] = 1.;
                            p[AA] = 0.;
                            p[AD] = time_norm(0.16);
                            p[AS] = 0.;
                            p[AR] = time_norm(0.08);
                            p[CUTOFF] = cutoff_norm(3500.);
                        }
                        5 => {
                            p[MIX_A] = 0.;
                            p[B_SAW] = 0.;
                            p[B_TRI] = 1.;
                            p[B_TUNE] = -24.;
                            p[POLY_ENV] = 0.35;
                            p[P_FILTER] = 1.;
                            p[ENV_AMT] = 0.3;
                            p[CUTOFF] = cutoff_norm(120.);
                            p[RES] = 0.68;
                            p[AS] = 0.;
                            p[AD] = time_norm(0.22);
                            p[FD] = time_norm(0.07);
                            p[FS] = 0.;
                            p[AA] = 0.;
                            p[FA] = 0.;
                        }
                        6 => {
                            p[MIX_A] = 0.;
                            p[MIX_B] = 0.;
                            p[NOISE] = 1.;
                            p[AA] = time_norm(1.5);
                            p[AR] = time_norm(2.5);
                            p[CUTOFF] = cutoff_norm(1600.);
                            p[RES] = 0.35;
                            p[MOD_AMOUNT] = 0.2;
                            p[W_FREQ] = 0.;
                            p[W_FILTER] = 1.;
                            p[LFO_RATE] = 0.2;
                        }
                        _ => {
                            p = std::array::from_fn(|i| SPECS[i].default);
                            p[MIX_B] = 0.;
                            p[ENV_AMT] = 0.;
                            p[VINTAGE] = 0.;
                            p[CUTOFF] = cutoff_norm(12000.);
                            p[AA] = time_norm(0.005);
                            p[AR] = time_norm(0.08);
                        }
                    }
                }
            }
            out.push(Preset {
                name: NAMES[bank][slot].into(),
                bank: BANK_NAMES[bank].into(),
                values: p,
            });
        }
    }
    out
}
