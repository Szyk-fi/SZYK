//! Bounded geometry built on the UI thread from measured audio telemetry.
//! Seven floats per segment: endpoints (normalized), opacity, palette blend, width.
use std::f32::consts::TAU;
pub const MODES: [&str; 10] = [
    "Waveform",
    "Spectral terrain",
    "Interference Loom",
    "Prismatic Shards",
    "Harmonic Orrery",
    "Phase Portrait",
    "Spectral Crown",
    "Spectrogram",
    "Contour Field",
    "Wave Ribbons",
];
fn at(v: &[f32], i: usize) -> f32 {
    v.get(i)
        .copied()
        .filter(|x| x.is_finite())
        .unwrap_or(0.)
        .clamp(-1., 1.)
}
fn line(out: &mut Vec<f32>, a: (f32, f32), b: (f32, f32), alpha: f32, tint: f32, width: f32) {
    if out.len() < 560 * 7 {
        out.extend([a.0, a.1, b.0, b.1, alpha.clamp(0., 1.), tint, width]);
    }
}
fn polar(a: f32, r: f32) -> (f32, f32) {
    (0.5 + a.cos() * r, 0.51 + a.sin() * r * 0.83)
}
pub fn geometry(
    mode: usize,
    wave: &[f32],
    spectrum: &[f32],
    history: &[f32],
    layers: &[f32],
    notes: &[f32],
    levels: &[f32],
    reverse: bool,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(2800);
    match mode {
        2 => {
            // Four signed memory traces. Scope uses successive delayed waveform windows.
            for layer in 0..4 {
                for i in 0..63 {
                    let sample = |n| {
                        if layers.is_empty() {
                            at(wave, (n + layer * 8) % wave.len().max(1))
                        } else {
                            at(layers, layer * 64 + n)
                        }
                    };
                    let point = |n: usize| {
                        (
                            0.06 + n as f32 / 63. * 0.88,
                            0.24 + layer as f32 * 0.17 + sample(n) * 0.13,
                        )
                    };
                    line(
                        &mut out,
                        point(i),
                        point(i + 1),
                        0.9 - layer as f32 * 0.13,
                        layer as f32 / 3.,
                        1.4,
                    );
                    if i % 8 == 0 && layer < 3 {
                        let next = if layers.is_empty() {
                            at(wave, (i + (layer + 1) * 8) % wave.len().max(1))
                        } else {
                            at(layers, (layer + 1) * 64 + i)
                        };
                        line(
                            &mut out,
                            point(i),
                            (point(i).0, 0.24 + (layer + 1) as f32 * 0.17 + next * 0.13),
                            0.18,
                            0.5,
                            0.7,
                        );
                    }
                }
            }
        }
        3 => {
            // Faceted slice constellation: geometry is displaced by each slice's samples.
            for i in 0..16 {
                let energy = if layers.is_empty() {
                    at(spectrum, i * 3).abs() * 6.
                } else {
                    (0..8)
                        .map(|j| at(layers, i * 8 + j).abs())
                        .fold(0., f32::max)
                }
                .min(1.);
                let x = 0.12 + (i % 8) as f32 * 0.108;
                let y = 0.31 + (i / 8) as f32 * 0.38;
                let r = 0.026 + energy * 0.028;
                let tilt = if reverse { -1. } else { 1. };
                let points = [
                    (x - r, y + 0.07),
                    (x + tilt * r * 0.4, y - 0.06 - energy * 0.08),
                    (x + r, y + 0.035),
                    (x, y + 0.09 + energy * 0.02),
                ];
                for j in 0..4 {
                    line(
                        &mut out,
                        points[j],
                        points[(j + 1) % 4],
                        0.35 + energy * 0.6,
                        i as f32 / 15.,
                        1.5,
                    );
                }
                line(
                    &mut out,
                    points[1],
                    points[3],
                    0.25 + energy * 0.5,
                    0.8,
                    0.8,
                );
                for j in 0..7 {
                    let sample = |k| {
                        if layers.is_empty() {
                            at(wave, (i * 6 + k) % wave.len().max(1))
                        } else {
                            at(layers, i * 8 + k)
                        }
                    };
                    line(
                        &mut out,
                        (x - r + j as f32 * r / 3.5, y + sample(j) * 0.06),
                        (x - r + (j + 1) as f32 * r / 3.5, y + sample(j + 1) * 0.06),
                        0.65,
                        0.,
                        0.8,
                    );
                }
            }
        }
        4 => {
            // Three harmonic planes. Live instrument note class / octave / envelope.
            for plane in 0..3 {
                let r = 0.16 + plane as f32 * 0.105;
                for i in 0..64 {
                    let point = |n| {
                        let t = n as f32 / 64. * TAU;
                        (
                            0.5 + t.cos() * r,
                            0.51 + t.sin() * r * (0.3 + plane as f32 * 0.2),
                        )
                    };
                    line(
                        &mut out,
                        point(i),
                        point(i + 1),
                        0.16 + plane as f32 * 0.05,
                        plane as f32 / 2.,
                        0.8,
                    );
                }
            }
            for i in 0..12 {
                let (angle, r, energy) = if notes.is_empty() {
                    (
                        i as f32 / 12. * TAU - TAU / 4.,
                        0.16 + (i % 3) as f32 * 0.105,
                        (at(spectrum, i * 4).abs() * 8.).min(1.),
                    )
                } else {
                    let n = at_note(notes, i);
                    (
                        n.rem_euclid(12.) / 12. * TAU - TAU / 4.,
                        0.16 + ((n / 12.).floor().rem_euclid(3.)) * 0.105,
                        at(levels, i).abs(),
                    )
                };
                let plane = ((r - 0.16) / 0.105).round();
                let p = (
                    0.5 + angle.cos() * r,
                    0.51 + angle.sin() * r * (0.3 + plane * 0.2),
                );
                line(&mut out, (0.5, 0.51), p, energy * 0.6, i as f32 / 12., 1.);
                for j in 0..12 {
                    let t = j as f32 / 12. * TAU;
                    let u = (j + 1) as f32 / 12. * TAU;
                    let s = 0.005 + energy * 0.014;
                    line(
                        &mut out,
                        (p.0 + t.cos() * s, p.1 + t.sin() * s),
                        (p.0 + u.cos() * s, p.1 + u.sin() * s),
                        0.22 + energy * 0.78,
                        i as f32 / 12.,
                        1.5,
                    );
                }
            }
        }
        5 => {
            // Mono delay embedding, explicitly not a stereo phase meter.
            for lag in [3, 7, 13] {
                for i in 0..wave.len().saturating_sub(14).min(96) {
                    let p = |n| (0.5 + at(wave, n) * 0.42, 0.5 + at(wave, n + lag) * 0.4);
                    line(
                        &mut out,
                        p(i),
                        p(i + 1),
                        0.3 + lag as f32 * 0.035,
                        lag as f32 / 13.,
                        1.3,
                    );
                }
            }
        }
        6 => {
            for i in 0..96 {
                let a = i as f32 / 96. * TAU - TAU / 4.;
                let b = (i + 1) as f32 / 96. * TAU - TAU / 4.;
                let e = (at(spectrum, i / 2).abs() * 6.).min(1.);
                let f = (at(spectrum, i.div_ceil(2) % 48).abs() * 6.).min(1.);
                line(
                    &mut out,
                    polar(a, 0.15),
                    polar(a, 0.17 + e * 0.27),
                    0.3 + e * 0.7,
                    i as f32 / 96.,
                    1.4,
                );
                line(
                    &mut out,
                    polar(a, 0.17 + e * 0.27),
                    polar(b, 0.17 + f * 0.27),
                    0.8,
                    0.7,
                    1.2,
                );
            }
        }
        7 => {
            for row in 0..12 {
                for bin in 0..32 {
                    let e = (at(history, row * 32 + bin).abs() * 8.).min(1.);
                    let x = 0.06 + bin as f32 / 32. * 0.88;
                    let y = 0.15 + row as f32 / 12. * 0.72;
                    line(&mut out, (x, y), (x + 0.023, y), 0.035 + e * 0.965, e, 5.);
                }
            }
        }
        8 => {
            // Iso-energy contours via marching squares across measured time/frequency.
            for threshold in [0.08, 0.2, 0.4, 0.7] {
                for row in 0..11 {
                    for bin in 0..31 {
                        let ids = [
                            row * 32 + bin,
                            row * 32 + bin + 1,
                            (row + 1) * 32 + bin + 1,
                            (row + 1) * 32 + bin,
                        ];
                        let points = [
                            (bin as f32, row as f32),
                            ((bin + 1) as f32, row as f32),
                            ((bin + 1) as f32, (row + 1) as f32),
                            (bin as f32, (row + 1) as f32),
                        ];
                        let mut crossings = Vec::new();
                        for edge in 0..4 {
                            let n = (edge + 1) % 4;
                            let a = at(history, ids[edge]).abs() * 8.;
                            let b = at(history, ids[n]).abs() * 8.;
                            if (a >= threshold) != (b >= threshold) {
                                let t = (threshold - a) / (b - a);
                                crossings.push((
                                    0.06 + (points[edge].0 + (points[n].0 - points[edge].0) * t)
                                        / 31.
                                        * 0.88,
                                    0.15 + (points[edge].1 + (points[n].1 - points[edge].1) * t)
                                        / 11.
                                        * 0.72,
                                ));
                            }
                        }
                        for pair in crossings.as_chunks::<2>().0 {
                            line(
                                &mut out,
                                pair[0],
                                pair[1],
                                0.4 + threshold * 0.6,
                                threshold,
                                1.2,
                            );
                        }
                    }
                }
            }
        }
        9 => {
            for ribbon in 0..5 {
                for i in 0..63 {
                    let p = |n: usize| {
                        let t = n as f32 / 63.;
                        let a = at(wave, n * wave.len().max(1) / 64);
                        let b = at(wave, (n * wave.len().max(1) / 64 + 9) % wave.len().max(1));
                        (
                            0.07 + t * 0.86,
                            0.22 + ribbon as f32 * 0.14
                                + a * 0.09
                                + b * 0.035 * (ribbon as f32 - 2.),
                        )
                    };
                    line(
                        &mut out,
                        p(i),
                        p(i + 1),
                        0.85 - ribbon as f32 * 0.1,
                        ribbon as f32 / 4.,
                        1.8,
                    );
                    if i % 4 == 0 {
                        let a = p(i);
                        line(&mut out, a, (a.0, a.1 + at(wave, i) * 0.045), 0.3, 0.8, 0.8);
                    }
                }
            }
        }
        _ => {}
    }
    out
}
fn at_note(v: &[f32], i: usize) -> f32 {
    v.get(i).copied().filter(|n| n.is_finite()).unwrap_or(60.)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_modes_are_bounded_finite_and_signal_driven() {
        let wave: Vec<_> = (0..96).map(|i| (i as f32 * 0.31).sin() * 0.6).collect();
        let spectrum: Vec<_> = (0..48)
            .map(|i| 0.02 + (i as f32 * 0.7).sin().abs() * 0.1)
            .collect();
        let history: Vec<_> = (0..384)
            .map(|i| (i as f32 * 0.1).sin().abs() * 0.1)
            .collect();
        for mode in 2..10 {
            let g = geometry(mode, &wave, &spectrum, &history, &[], &[], &[], false);
            assert!(!g.is_empty(), "mode {mode}");
            assert!(g.len() <= 560 * 7);
            assert_eq!(g.len() % 7, 0);
            assert!(g.iter().all(|x| x.is_finite()));
            assert_ne!(
                g,
                geometry(
                    mode,
                    &vec![0.; 96],
                    &vec![0.; 48],
                    &vec![0.; 384],
                    &[],
                    &[],
                    &[],
                    false
                )
            );
        }
    }
}
