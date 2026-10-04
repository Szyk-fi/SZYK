//! Pulsar's genre library and pattern generator.
//!
//! Each genre is a tempo, a swing feel, a kit (synth model + sound per
//! lane) and a 16-step probability template per lane. The generator turns a
//! template into a concrete multi-bar pattern, shaped by Complexity,
//! Density, Ghosts, Rolls and Variation, and never touches locked lanes.
//!
//! Template characters, one per 16th step:
//!   x  always, accented      o  always, normal
//!   g  ghost note (70%, quiet)   1-9  10%-90% chance, normal
//!   .  never
//!
//! Lanes: Kick, Snare, Clap, Closed Hat, Open Hat, Perc 1, Perc 2, Crash.

use super::engine::{LaneSettings, Model, Pattern, Step, LANES, MAX_STEPS};

#[derive(Clone, Copy)]
pub struct KitLane {
    pub model: Model,
    pub tune: f32,
    pub decay: f32,
    pub tone: f32,
    pub punch: f32,
}

const fn k(model: Model, tune: f32, decay: f32, tone: f32, punch: f32) -> KitLane {
    KitLane { model, tune, decay, tone, punch }
}

use Model::*;

pub const KIT_909: [KitLane; LANES] = [
    k(Kick909, 0.0, 0.5, 0.5, 0.6),
    k(Snare, 0.0, 0.4, 0.6, 0.6),
    k(Clap, 0.0, 0.4, 0.5, 0.5),
    k(HatClosed, 0.0, 0.3, 0.6, 0.4),
    k(HatOpen, 0.0, 0.4, 0.6, 0.4),
    k(Rim, 0.0, 0.3, 0.5, 0.5),
    k(Conga, 0.0, 0.4, 0.4, 0.5),
    k(Crash, 0.0, 0.5, 0.5, 0.5),
];
pub const KIT_808: [KitLane; LANES] = [
    k(Kick808, 0.0, 0.7, 0.3, 0.5),
    k(Snare808, 0.0, 0.4, 0.5, 0.5),
    k(Clap, 0.0, 0.5, 0.5, 0.6),
    k(HatClosed, 0.0, 0.25, 0.5, 0.3),
    k(HatOpen, 0.0, 0.45, 0.5, 0.3),
    k(Cowbell, 0.0, 0.4, 0.5, 0.4),
    k(Conga, 2.0, 0.4, 0.4, 0.5),
    k(Crash, 0.0, 0.6, 0.5, 0.5),
];
pub const KIT_TRAP: [KitLane; LANES] = [
    k(Kick808, -2.0, 0.95, 0.4, 0.6),
    k(Snare808, 3.0, 0.3, 0.7, 0.8),
    k(Clap, 0.0, 0.35, 0.6, 0.7),
    k(HatClosed, 2.0, 0.12, 0.8, 0.4),
    k(HatOpen, 0.0, 0.35, 0.7, 0.4),
    k(Rim, 0.0, 0.3, 0.6, 0.5),
    k(Snap, 0.0, 0.3, 0.5, 0.5),
    k(Crash, 0.0, 0.6, 0.6, 0.5),
];
pub const KIT_TECHNO: [KitLane; LANES] = [
    k(KickHard, -1.0, 0.45, 0.6, 0.7),
    k(Snare, 2.0, 0.3, 0.7, 0.7),
    k(Clap, -2.0, 0.5, 0.4, 0.5),
    k(HatClosed, 0.0, 0.2, 0.6, 0.5),
    k(Ride, 0.0, 0.4, 0.5, 0.4),
    k(Rim, 0.0, 0.25, 0.6, 0.5),
    k(Clave, -3.0, 0.3, 0.5, 0.5),
    k(Crash, 0.0, 0.5, 0.5, 0.5),
];
pub const KIT_ACOUSTIC: [KitLane; LANES] = [
    k(Kick909, -3.0, 0.35, 0.2, 0.3),
    k(Snare, -1.0, 0.5, 0.5, 0.75),
    k(Snap, 0.0, 0.3, 0.5, 0.5),
    k(HatClosed, -1.0, 0.25, 0.4, 0.2),
    k(HatOpen, -1.0, 0.4, 0.4, 0.2),
    k(Tom, 0.0, 0.5, 0.4, 0.5),
    k(Shaker, 0.0, 0.4, 0.4, 0.4),
    k(Crash, -1.0, 0.7, 0.4, 0.5),
];
pub const KIT_LATIN: [KitLane; LANES] = [
    k(Kick909, -2.0, 0.45, 0.3, 0.5),
    k(Rim, 0.0, 0.4, 0.6, 0.5),
    k(Clap, 0.0, 0.4, 0.5, 0.5),
    k(HatClosed, 0.0, 0.25, 0.5, 0.3),
    k(HatOpen, 0.0, 0.4, 0.5, 0.3),
    k(Conga, 0.0, 0.45, 0.5, 0.5),
    k(Shaker, 0.0, 0.35, 0.5, 0.5),
    k(Cowbell, 0.0, 0.5, 0.5, 0.5),
];
pub const KIT_ELECTRO: [KitLane; LANES] = [
    k(Kick808, 2.0, 0.5, 0.5, 0.7),
    k(Snare808, 0.0, 0.35, 0.6, 0.6),
    k(Clap, 2.0, 0.4, 0.6, 0.5),
    k(HatClosed, 1.0, 0.2, 0.7, 0.4),
    k(HatOpen, 1.0, 0.35, 0.7, 0.4),
    k(Cowbell, 0.0, 0.35, 0.6, 0.5),
    k(Zap, 0.0, 0.3, 0.5, 0.4),
    k(Crash, 0.0, 0.5, 0.6, 0.5),
];
pub const KIT_DNB: [KitLane; LANES] = [
    k(KickHard, 2.0, 0.3, 0.6, 0.6),
    k(Snare, 4.0, 0.35, 0.8, 0.85),
    k(Clap, 2.0, 0.3, 0.6, 0.5),
    k(HatClosed, 2.0, 0.15, 0.7, 0.4),
    k(Ride, 0.0, 0.35, 0.6, 0.4),
    k(Shaker, 0.0, 0.3, 0.6, 0.5),
    k(Tom, 3.0, 0.4, 0.5, 0.6),
    k(Crash, 0.0, 0.5, 0.6, 0.5),
];
pub const KIT_LOFI: [KitLane; LANES] = [
    k(Kick909, -4.0, 0.4, 0.15, 0.25),
    k(Snare, -3.0, 0.45, 0.3, 0.6),
    k(Snap, -2.0, 0.3, 0.3, 0.5),
    k(HatClosed, -3.0, 0.25, 0.2, 0.2),
    k(HatOpen, -3.0, 0.35, 0.2, 0.2),
    k(Shaker, -2.0, 0.35, 0.2, 0.4),
    k(Rim, -3.0, 0.35, 0.3, 0.4),
    k(Crash, -3.0, 0.6, 0.2, 0.4),
];
pub const KIT_HARD: [KitLane; LANES] = [
    k(KickHard, 0.0, 0.6, 0.8, 1.0),
    k(Snare, 0.0, 0.4, 0.7, 0.7),
    k(Clap, 0.0, 0.5, 0.6, 0.6),
    k(HatClosed, 0.0, 0.2, 0.7, 0.4),
    k(HatOpen, 0.0, 0.4, 0.7, 0.4),
    k(Zap, -5.0, 0.4, 0.7, 0.5),
    k(Ride, 0.0, 0.4, 0.6, 0.4),
    k(Crash, 0.0, 0.6, 0.7, 0.5),
];

pub struct Genre {
    pub name: &'static str,
    pub bpm: f32,
    pub swing: f32,
    pub swing8: bool,
    pub kit: &'static [KitLane; LANES],
    /// Hat roll tendency 0..1 (trap/drill).
    pub rolls: f32,
    pub lanes: [&'static str; LANES],
}

const fn g(
    name: &'static str,
    bpm: f32,
    swing: f32,
    swing8: bool,
    kit: &'static [KitLane; LANES],
    rolls: f32,
    lanes: [&'static str; LANES],
) -> Genre {
    Genre { name, bpm, swing, swing8, kit, rolls, lanes }
}

//                                   Kick                Snare               Clap                Closed hat          Open hat            Perc 1              Perc 2              Crash
pub const GENRES: &[Genre] = &[
    g("House", 124.0, 0.15, false, &KIT_909, 0.0, ["x...x...x...x...", "................", "....x.......x...", ".4.4.4.4.4.4.4.4", "..o...o...o...o.", "...3......3..3..", ".......3.....3..", "3..............."]),
    g("Deep House", 120.0, 0.25, false, &KIT_909, 0.0, ["x...x...x...x...", "................", "....3.......3...", ".3.5.3.5.3.5.3.5", "..o...o...o...o.", "....o.......o...", "......3.....3..3", "................"]),
    g("Tech House", 126.0, 0.12, false, &KIT_909, 0.0, ["x...x...x...x.3.", "................", "....x.......x...", "o4o4o4o4o4o4o4o4", "..o...o...o...o.", "..3..3....3..3..", ".3......3.....3.", "2..............."]),
    g("Techno", 132.0, 0.0, false, &KIT_TECHNO, 0.0, ["x...x...x...x...", "........3.......", "....3.......3...", "..o...o...o...o.", "o.o.o.o.o.o.o.o.", "..3..3....3..3..", ".3......3....3..", "3..............."]),
    g("Minimal", 125.0, 0.18, false, &KIT_TECHNO, 0.0, ["x...x...x...x...", "................", "....2.......3...", ".3..3.3..3..3.3.", "..3.......3.....", "..o.....3..o....", ".....3.....3...3", "................"]),
    g("Trance", 138.0, 0.0, false, &KIT_909, 0.0, ["x...x...x...x...", "................", "....x.......x...", "o3o3o3o3o3o3o3o3", "..o...o...o...o.", "................", "..........3.....", "x..............."]),
    g("UK Garage", 132.0, 0.4, false, &KIT_909, 0.0, ["x.......3.x.....", "....x.......x...", "....3.......3...", ".3o3.3o3.3o3.3o3", "..3.......3.....", "......3.3.....3.", "...3.......3....", "................"]),
    g("Drum & Bass", 174.0, 0.0, false, &KIT_DNB, 0.0, ["x.........x.....", "....x.......x...", "....3.......3...", "o.o.o.o.o.o.o.o.", "..............3.", "3.3.3.3.3.3.3.3.", ".......3......3.", "3..............."]),
    g("Jungle", 165.0, 0.08, false, &KIT_DNB, 0.0, ["x.....x...3.....", "....x..3.3..x..3", "............3...", "o.o.o.o.o.o.o.o.", "......3.......3.", "..3.3...3.3.3...", ".3.....3..3....3", "3..............."]),
    g("Dubstep", 140.0, 0.05, false, &KIT_TECHNO, 0.2, ["x.........3.....", "........x.......", "........x.......", "o.3.o.3.o.3.o.3.", "......3.......3.", "...3......3.....", "..............3.", "3..............."]),
    g("Trap", 140.0, 0.0, false, &KIT_TRAP, 0.9, ["x......3..x....3", "........x.......", "........x.......", "o.o.o.o.o.o.o.o.", ".............3..", "...........3....", "...3.........3..", "3..............."]),
    g("UK Drill", 142.0, 0.1, false, &KIT_TRAP, 0.7, ["x.....x...3...3.", "......x.......x.", "......3.......3.", "o.o3o.o.o3o.o.o3", "..........3.....", ".3.......3......", "......3.....3...", "3..............."]),
    g("Boom Bap", 90.0, 0.3, false, &KIT_ACOUSTIC, 0.0, ["x.....3.x.3.....", "....x.......x...", "....3.......3...", "o.o.o.o.o.o.o.o.", "..............3.", "...........3....", ".3..3..3..3..3..", "2..............."]),
    g("Lo-Fi", 82.0, 0.38, false, &KIT_LOFI, 0.0, ["x.....3.x..3....", "....x...g...x..g", "....3.......3...", "o.o.o.o.o.o.o.o.", "...........3....", "3.3.3.3.3.3.3.3.", ".......3......3.", "................"]),
    g("Afrobeats", 106.0, 0.15, false, &KIT_LATIN, 0.0, ["x..3..x...x..3..", "...x..x...3..x..", "......3.......3.", "o.oo.oo.o.oo.oo.", "..3.......3.....", "..3..3.3..3..3.3", "5555555555555555", "................"]),
    g("Amapiano", 112.0, 0.22, false, &KIT_LATIN, 0.0, ["x...x...x...x...", "..x..x....x..x..", "....3.......3...", ".3.3.3.3.3.3.3.3", "..o...o...o...o.", "...3..3....3..3.", "6666666666666666", "................"]),
    g("Reggaeton", 95.0, 0.05, false, &KIT_LATIN, 0.0, ["x...x...x...x...", "...x..x....x..x.", "...3..3....3..3.", "o.o.o.o.o.o.o.o.", "..............3.", "......3.......3.", "3.3.3.3.3.3.3.3.", "2..............."]),
    g("Dancehall", 100.0, 0.12, false, &KIT_LATIN, 0.0, ["x..x..x.x..x..x.", "...x..x....x..x.", "......3.......3.", "o.o.o.o.o.o.o.o.", "..3.......3.....", "..3....3..3....3", "3333333333333333", "................"]),
    g("Breakbeat", 130.0, 0.08, false, &KIT_ACOUSTIC, 0.0, ["x.........x.3...", "....x..g....x..g", "....3.......3...", "o.o.o.o.o.o.o.o.", "......3.......3.", ".....3.....3....", "3...3...3...3...", "3..............."]),
    g("Electro", 128.0, 0.05, false, &KIT_ELECTRO, 0.0, ["x.....x...x.....", "....x.......x...", "....3.......3...", "o3o3o3o3o3o3o3o3", "..3.......3.....", "..3.3.....3.3...", "...........3....", "3..............."]),
    g("Funk", 100.0, 0.22, false, &KIT_ACOUSTIC, 0.0, ["x.3.....x.3..3..", "....x..g.g..x..g", "....3.......3...", "o3o3o3o3o3o3o3o3", "..............3.", "......3.......3.", "3...3...3...3...", "3..............."]),
    g("Disco", 118.0, 0.1, false, &KIT_ACOUSTIC, 0.0, ["x...x...x...x...", "....x.......x...", "....3.......3...", "o3o3o3o3o3o3o3o3", "..o...o...o...o.", ".......3.......3", "3.3.3.3.3.3.3.3.", "x..............."]),
    g("Rock", 120.0, 0.0, false, &KIT_ACOUSTIC, 0.0, ["x.......x.x.....", "....x.......x...", "................", "o.o.o.o.o.o.o.o.", "..............3.", "..............33", "................", "x..............."]),
    g("Jersey Club", 140.0, 0.05, false, &KIT_808, 0.3, ["x..x..x...x.x.x.", "....x.......x...", "....x..3....x..3", "o.o.o.o.o.o.o.o.", "..3.......3.....", "...3.......3....", "..........3..3..", "3..............."]),
    g("Footwork", 160.0, 0.05, false, &KIT_808, 0.5, ["x..x..x...x..x..", "....x.......x...", "....3..3....3...", "o3o3o3o3o3o3o3o3", "..3.....3.......", "..3..3.3..3..3..", ".3......3.......", "................"]),
    g("Baile Funk", 130.0, 0.1, false, &KIT_808, 0.0, ["x..x..x...x..x..", "....x.......x...", "..x..x....x..x..", "o.o.o.o.o.o.o.o.", "..............3.", "...3..3....3..3.", "3.3.3.3.3.3.3.3.", "................"]),
    g("Synthwave", 100.0, 0.0, false, &KIT_ELECTRO, 0.0, ["x.......x.......", "....x.......x...", "....3.......3...", "o.o.o.o.o.o.o.o.", "..3.......3.....", "................", "..........3.....", "x..............."]),
    g("Phonk", 130.0, 0.08, false, &KIT_808, 0.4, ["x.......x.3.....", "........x.......", "....3.......x...", "o.o.o.o.o.o.o.o.", "..3.......3.....", "x..x..x.x..x..x.", "...........3....", "................"]),
    g("Hardstyle", 150.0, 0.0, false, &KIT_HARD, 0.0, ["x...x...x...x...", "................", "....x.......x...", "o3o3o3o3o3o3o3o3", "..o...o...o...o.", "..........3.....", ".3.....3........", "x..............."]),
    g("Gqom", 120.0, 0.12, false, &KIT_LATIN, 0.0, ["x..3..x...3..x..", "......3.......3.", "....x.......x...", ".3.3.3.3.3.3.3.3", "..3.......3.....", "3..3..3...3..3..", "..3.......3.....", "................"]),
    g("???", 0.0, 0.0, false, &KIT_909, 0.5, ["", "", "", "", "", "", "", ""]),
];

pub fn chaos_index() -> usize {
    GENRES.len() - 1
}

/// (probability 0..1, velocity 0..127, ghost?)
fn cell(c: u8) -> (f32, u8, bool) {
    match c {
        b'x' => (1.0, 122, false),
        b'o' => (1.0, 98, false),
        b'g' => (0.7, 38, true),
        b'1'..=b'9' => ((c - b'0') as f32 / 10.0, 88, false),
        _ => (0.0, 0, false),
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GenParams {
    pub complexity: f32,
    pub density: f32,
    pub variation: f32,
    pub ghosts: f32,
    pub rolls: f32,
    pub length: usize,
}

impl Default for GenParams {
    fn default() -> Self {
        Self { complexity: 0.4, density: 0.5, variation: 0.3, ghosts: 0.3, rolls: 0.4, length: 32 }
    }
}

/// xorshift RNG for generation (deterministic per seed, for tests).
pub struct Rng(pub u32);
impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed | 1)
    }
    pub fn seeded() -> Self {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(7);
        Rng(t | 1)
    }
    pub fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x as f32 / u32::MAX as f32
    }
    pub fn range(&mut self, n: usize) -> usize {
        ((self.next() * n as f32) as usize).min(n.saturating_sub(1))
    }
}

/// How much each lane wants syncopated extra hits.
const SYNC_AFFINITY: [f32; LANES] = [0.45, 0.25, 0.15, 0.8, 0.25, 0.7, 0.55, 0.0];

fn sync_weight(step: usize) -> f32 {
    match step % 4 {
        0 => 0.05,
        2 => 0.18,
        _ => 0.3,
    }
}

/// A random template for the "???" genre.
fn chaos_template(lane: usize, rng: &mut Rng) -> [u8; 16] {
    let mut t = [b'.'; 16];
    let base = [0.35, 0.2, 0.15, 0.6, 0.15, 0.4, 0.3, 0.05][lane];
    for (i, c) in t.iter_mut().enumerate() {
        let r = rng.next();
        if lane == 0 && i == 0 {
            *c = b'x';
        } else if r < base * 0.4 {
            *c = b'o';
        } else if r < base {
            *c = b'1' + rng.range(9) as u8;
        }
    }
    t
}

fn template(genre: &Genre, lane: usize, rng: &mut Rng) -> [u8; 16] {
    let s = genre.lanes[lane].as_bytes();
    if s.len() < 16 {
        return chaos_template(lane, rng);
    }
    let mut t = [b'.'; 16];
    t.copy_from_slice(&s[..16]);
    t
}

/// One bar for one lane.
fn gen_bar(genre: &Genre, lane: usize, p: &GenParams, rng: &mut Rng) -> [Step; 16] {
    let tpl = template(genre, lane, rng);
    let dens = p.density.clamp(0.0, 1.0);
    let cx = p.complexity.clamp(0.0, 1.0);
    let mut bar = [Step::OFF; 16];
    for i in 0..16 {
        let (prob, vel, ghost) = cell(tpl[i]);
        let mut chance = if prob >= 1.0 {
            if dens < 0.15 && lane != 0 && i % 8 != 0 {
                0.5
            } else {
                1.0
            }
        } else {
            (prob * (0.35 + dens * 1.3)).min(0.95)
        };
        let mut v = vel;
        let mut is_ghost = ghost;
        if prob == 0.0 {
            // complexity: syncopated extras where this lane likes them
            chance = cx * SYNC_AFFINITY[lane] * sync_weight(i) * (0.5 + dens);
            v = 80;
            // ghost snares
            if lane == 1 && i % 2 == 1 && rng.next() < p.ghosts * 0.45 {
                chance = 1.0;
                v = 34;
                is_ghost = true;
            }
        }
        if lane == 7 && i > 0 {
            chance *= 0.3;
        }
        if rng.next() < chance {
            let accent = if i % 4 == 0 && !is_ghost { 8 } else { 0 };
            let jitter = ((rng.next() - 0.5) * 20.0 * cx) as i32;
            let vel = (v as i32 + accent + jitter).clamp(20, 127) as u8;
            let mut st = Step::hit(vel);
            // rolls on hats
            if (lane == 3 || lane == 4) && rng.next() < p.rolls * genre.rolls * 0.35 {
                st.ratchet = 2 + rng.range(3) as u8;
            }
            if is_ghost {
                st.prob = 85;
            }
            bar[i] = st;
        }
    }
    bar
}

/// Generate a full pattern. Locked lanes are copied from `current`.
pub fn generate(genre: &Genre, p: &GenParams, locks: &[bool; LANES], current: &Pattern, rng: &mut Rng) -> Pattern {
    let mut out = *current;
    let length = p.length.clamp(16, MAX_STEPS) / 16 * 16;
    out.length = length;
    let bars = length / 16;
    for lane in 0..LANES {
        if locks[lane] {
            continue;
        }
        let first = gen_bar(genre, lane, p, rng);
        for b in 0..bars {
            let mut bar = first;
            if b > 0 {
                // later bars: mutate the first bar a little
                let alt = gen_bar(genre, lane, p, rng);
                for i in 0..16 {
                    let anchor = cell(template(genre, lane, rng)[i]).0 >= 1.0;
                    if !anchor && rng.next() < p.variation * 0.35 {
                        bar[i] = alt[i];
                    }
                }
                if lane == 7 {
                    bar = [Step::OFF; 16];
                }
            }
            // last bar of a multi-bar phrase: small turnaround
            if bars > 1 && b == bars - 1 && (lane == 1 || lane == 5 || lane == 6) {
                for i in 12..16 {
                    if !bar[i].on() && rng.next() < p.variation * 0.5 {
                        bar[i] = Step::hit(70 + rng.range(40) as u8);
                    }
                }
            }
            out.steps[lane][b * 16..b * 16 + 16].copy_from_slice(&bar);
        }
        for i in length..MAX_STEPS {
            out.steps[lane][i] = Step::OFF;
        }
    }
    out
}

/// A one-bar fill in the genre's style: building snare/perc rolls,
/// kick pickups, an open hat to lead into the next phrase.
pub fn fill_bar(genre: &Genre, base: &Pattern, intensity: f32, rng: &mut Rng) -> Pattern {
    let mut f = Pattern::default();
    let p = GenParams { complexity: 0.5 + intensity * 0.5, density: 0.6, variation: 0.0, ghosts: 0.5, rolls: 0.6, length: 16 };
    let intensity = intensity.clamp(0.0, 1.0);
    for lane in 0..LANES {
        let bar = gen_bar(genre, lane, &p, rng);
        f.steps[lane][..16].copy_from_slice(&bar);
        // first half follows the groove
        for i in 0..8 {
            f.steps[lane][i] = base.steps[lane][i];
        }
    }
    // second half: rolls, denser toward the end
    for i in 8..16 {
        let ramp = (i - 7) as f32 / 8.0;
        for &lane in &[1usize, 5, 6] {
            if rng.next() < (0.25 + intensity * 0.6) * ramp {
                let mut st = Step::hit((60.0 + ramp * 60.0) as u8);
                if lane == 1 && i >= 12 && rng.next() < intensity * 0.6 {
                    st.ratchet = 2;
                }
                f.steps[lane][i] = st;
            }
        }
        if i >= 14 && rng.next() < 0.5 {
            f.steps[0][i] = Step::hit(110);
        }
    }
    f.steps[4][14] = Step::hit(96);
    f.steps[3][14] = Step::OFF;
    f
}

/// Apply a genre's kit to lane settings (unlocked lanes only).
pub fn apply_kit(genre: &Genre, lanes: &mut [LaneSettings; LANES], locks: &[bool; LANES]) {
    for (i, kl) in genre.kit.iter().enumerate() {
        if locks[i] {
            continue;
        }
        let l = &mut lanes[i];
        l.model = kl.model;
        l.use_sample = false;
        l.tune = kl.tune;
        l.decay = kl.decay;
        l.tone = kl.tone;
        l.punch = kl.punch;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_thirty_genres_plus_mystery_all_well_formed() {
        assert_eq!(GENRES.len(), 31);
        assert_eq!(GENRES[chaos_index()].name, "???");
        for g in &GENRES[..GENRES.len() - 1] {
            assert!(g.bpm >= 60.0 && g.bpm <= 200.0, "{}", g.name);
            for (i, l) in g.lanes.iter().enumerate() {
                assert_eq!(l.len(), 16, "{} lane {i}", g.name);
                assert!(l.bytes().all(|c| b".xog123456789".contains(&c)), "{} lane {i}: {l}", g.name);
            }
        }
    }

    #[test]
    fn four_on_the_floor_genres_keep_their_kick() {
        let mut rng = Rng::new(3);
        for name in ["House", "Techno", "Disco", "Hardstyle"] {
            let g = GENRES.iter().find(|g| g.name == name).unwrap();
            for _ in 0..20 {
                let p = generate(g, &GenParams::default(), &[false; LANES], &Pattern::default(), &mut rng);
                for beat in [0, 4, 8, 12, 16, 20, 24, 28] {
                    assert!(p.steps[0][beat].on(), "{name}: kick missing on step {beat}");
                }
            }
        }
    }

    #[test]
    fn locked_lanes_are_untouched_and_unlocked_change() {
        let mut rng = Rng::new(11);
        let g = &GENRES[0];
        let mut cur = Pattern::default();
        cur.steps[1][3] = Step::hit(99);
        let mut locks = [false; LANES];
        locks[1] = true;
        let p = generate(g, &GenParams::default(), &locks, &cur, &mut rng);
        assert_eq!(p.steps[1], cur.steps[1]);
        assert!(p.hits(0) > 0);
    }

    #[test]
    fn every_genre_generates_a_usable_beat_at_every_length() {
        let mut rng = Rng::new(5);
        for g in GENRES {
            for len in [16, 32, 48, 64] {
                let p = generate(g, &GenParams { length: len, ..GenParams::default() }, &[false; LANES], &Pattern::default(), &mut rng);
                assert_eq!(p.length, len);
                let total: usize = (0..LANES).map(|l| p.hits(l)).sum();
                assert!(total >= 4, "{} at {len}: only {total} hits", g.name);
                for lane in 0..LANES {
                    assert!(p.steps[lane][len..].iter().all(|s| !s.on()), "{}: hits past the end", g.name);
                }
            }
        }
    }

    #[test]
    fn density_and_complexity_move_hit_counts_the_right_way() {
        let g = GENRES.iter().find(|g| g.name == "Techno").unwrap();
        let count = |d: f32, c: f32| {
            let mut rng = Rng::new(9);
            (0..30)
                .map(|_| {
                    let p = generate(g, &GenParams { density: d, complexity: c, ..GenParams::default() }, &[false; LANES], &Pattern::default(), &mut rng);
                    (0..LANES).map(|l| p.hits(l)).sum::<usize>()
                })
                .sum::<usize>()
        };
        assert!(count(0.9, 0.4) > count(0.1, 0.4));
        assert!(count(0.5, 1.0) > count(0.5, 0.0));
    }

    #[test]
    fn trap_rolls_appear_and_house_rolls_dont() {
        let mut rng = Rng::new(21);
        let rolls = |name: &str, rng: &mut Rng| {
            let g = GENRES.iter().find(|g| g.name == name).unwrap();
            (0..40)
                .map(|_| {
                    let p = generate(g, &GenParams { rolls: 1.0, ..GenParams::default() }, &[false; LANES], &Pattern::default(), rng);
                    p.steps[3].iter().filter(|s| s.ratchet > 1).count()
                })
                .sum::<usize>()
        };
        assert!(rolls("Trap", &mut rng) > 10);
        assert_eq!(rolls("House", &mut rng), 0);
    }
}
