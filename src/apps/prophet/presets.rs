//! Original starting points, never Sequential factory content.
use super::{patch::Patch, Program};
pub(super) fn factory() -> Vec<Program> {
    let names = [
        "Init Rev2",
        "Warm Stack",
        "Split Bass + Pad",
        "Pulse Motion",
        "Resonant Lead",
        "Triangle Keys",
        "Slow Ensemble",
        "Sync Bite",
        "Sub Bass",
        "Mod Wheel Brass",
        "Random Texture",
        "Stereo Arp",
        "Gated Steps",
        "Poly Sequence",
        "Ring Bell",
        "Wide Unison",
    ];
    names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let mut p = Patch::default();
            p.set_name(0, name);
            p.set_name(1, name);
            match i {
                1 => {
                    p.data[231] = 2;
                    p.data[1027] = 57;
                    p.data[1066] = 65;
                    p.data[1075] = 75;
                    p.data[115] = 4;
                    p.data[116] = 1;
                    p.data[117] = 30;
                }
                2 => {
                    p.data[231] = 1;
                    p.data[232] = 60;
                    p.data[15] = 70;
                    p.data[22] = 70;
                    p.data[48] = 80;
                    p.data[1066] = 80;
                    p.data[1075] = 70;
                }
                3 => {
                    p.data[4] = 4;
                    p.data[5] = 4;
                    p.data[61] = 30;
                    p.data[65] = 9;
                    p.data[53] = 70;
                }
                4 => {
                    p.data[23] = 105;
                    p.data[22] = 65;
                    p.data[32] = 170;
                    p.data[44] = 65;
                }
                5 => {
                    p.data[4] = 3;
                    p.data[5] = 3;
                    p.data[1] = 36;
                }
                6 => {
                    p.data[42] = 85;
                    p.data[51] = 95;
                    p.data[115] = 4;
                    p.data[116] = 1;
                    p.data[117] = 50;
                    p.data[118] = 70;
                    p.data[119] = 60;
                }
                7 => {
                    p.data[17] = 1;
                    p.data[1] = 36;
                    p.data[30] = 1;
                    p.data[34] = 150;
                    p.data[43] = 0;
                    p.data[46] = 50;
                }
                8 => {
                    p.data[0] = 12;
                    p.data[1] = 12;
                    p.data[15] = 100;
                    p.data[22] = 75;
                }
                9 => {
                    p.data[101] = 185;
                    p.data[102] = 10;
                    p.data[32] = 160;
                    p.data[41] = 20;
                }
                10 => {
                    p.data[57] = 4;
                    p.data[61] = 25;
                    p.data[65] = 10;
                    p.data[53] = 60;
                    p.data[115] = 10;
                    p.data[116] = 1;
                    p.data[117] = 45;
                }
                11 => {
                    p.data[136] = 1;
                    p.data[131] = 2;
                    p.data[133] = 2;
                    p.data[115] = 2;
                    p.data[116] = 1;
                    p.data[117] = 40;
                    p.data[118] = 75;
                    p.data[119] = 40;
                }
                12 => {
                    p.data[139] = 0;
                    p.data[111] = 3;
                    for s in 0..16 {
                        p.data[140 + s] = [0, 7, 12, 19][s % 4] * 2;
                    }
                }
                13 => {
                    for s in 0..8 {
                        p.data[256 + s] = [60, 64, 67, 72][s % 4];
                        p.data[320 + s] = 227;
                    }
                }
                14 => {
                    p.data[4] = 3;
                    p.data[115] = 11;
                    p.data[116] = 1;
                    p.data[117] = 90;
                    p.data[118] = 80;
                    p.data[119] = 1;
                    p.data[48] = 0;
                    p.data[45] = 70;
                }
                15 => {
                    p.data[123] = 1;
                    p.data[124] = 7;
                    p.data[208] = 8;
                    p.data[29] = 90;
                }
                _ => {}
            }
            Program {
                patch: p,
                source: "Original starting points".into(),
            }
        })
        .collect()
}
