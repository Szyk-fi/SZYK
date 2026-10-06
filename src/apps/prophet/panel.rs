use super::{spec::*, ProphetApp};
use crate::{
    display::FrameBuffer,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Circle, Line, PrimitiveStyle, Rectangle},
    text::Text,
};
use std::f32::consts::PI;

const BLACK: Rgb565 = Rgb565::new(2, 3, 2);
const INK: Rgb565 = Rgb565::new(30, 57, 25);
const DIM: Rgb565 = Rgb565::new(16, 31, 14);
const RED: Rgb565 = Rgb565::new(31, 6, 2);
const GOLD: Rgb565 = Rgb565::new(30, 43, 15);
const LABELS: [&str; N] = [
    "FREQUENCY",
    "SAW",
    "PULSE",
    "PULSE WIDTH",
    "SYNC A <- B",
    "FREQUENCY",
    "FINE TUNE",
    "SAW",
    "PULSE",
    "TRIANGLE",
    "PULSE WIDTH",
    "LOW FREQ",
    "KEYBOARD",
    "OSC A",
    "OSC B",
    "NOISE",
    "CUTOFF",
    "RESONANCE",
    "ENV AMOUNT",
    "KEY TRACK",
    "DRIVE",
    "VCF ATTACK",
    "VCF DECAY",
    "VCF SUSTAIN",
    "VCF RELEASE",
    "VCA ATTACK",
    "VCA DECAY",
    "VCA SUSTAIN",
    "VCA RELEASE",
    "LFO RATE",
    "WAVEFORM",
    "NOISE MIX",
    "FREQUENCY",
    "PULSE WIDTH",
    "FILTER",
    "AMOUNT",
    "FILTER ENV",
    "OSC B",
    "A FREQUENCY",
    "A PULSE WIDTH",
    "FILTER",
    "MODE",
    "VOICES",
    "UNI DETUNE",
    "GLIDE",
    "VINTAGE",
    "VELOCITY",
    "SPREAD",
    "VOLUME",
    "CHORUS",
    "DELAY TIME",
    "FEEDBACK",
    "DELAY MIX",
    "PAD OCTAVE",
    "BEND RANGE",
];
fn rect(f: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, color: Rgb565) {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(f)
        .ok();
}
fn line(f: &mut FrameBuffer, x: i32, y: i32, xx: i32, yy: i32, color: Rgb565) {
    Line::new(Point::new(x, y), Point::new(xx, yy))
        .into_styled(PrimitiveStyle::with_stroke(color, 1))
        .draw(f)
        .ok();
}
fn text(f: &mut FrameBuffer, s: &str, x: i32, y: i32, color: Rgb565) {
    Text::new(s, Point::new(x, y), MonoTextStyle::new(&SPLEEN_6X12, color))
        .draw(f)
        .ok();
}
fn centered(f: &mut FrameBuffer, s: &str, x: i32, y: i32, color: Rgb565) {
    text(f, s, x - s.chars().count() as i32 * 3, y, color);
}
fn short(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
fn button(f: &mut FrameBuffer, s: &str, x: i32, y: i32, w: u32, on: bool) {
    rect(
        f,
        x,
        y,
        w,
        19,
        if on {
            Rgb565::new(8, 9, 3)
        } else {
            Rgb565::new(4, 6, 3)
        },
    );
    line(
        f,
        x,
        y + 18,
        x + w as i32,
        y + 18,
        if on { GOLD } else { DIM },
    );
    centered(f, s, x + w as i32 / 2, y + 13, if on { GOLD } else { INK });
}
fn cell(a: &ProphetApp, k: usize) -> (i32, i32, i32) {
    let items = GROUPS[a.group].1;
    let columns = if items.len() > 10 {
        7
    } else if items.len() > 5 {
        4
    } else {
        items.len()
    };
    let width = 588 / columns as i32;
    (
        26 + width / 2 + (k % columns) as i32 * width,
        151 + (k / columns) as i32 * 78,
        width,
    )
}
fn dial(f: &mut FrameBuffer, a: &ProphetApp, i: usize, x: i32, y: i32) {
    let selected = i == a.selected;
    let style = PrimitiveStyle::with_stroke(if selected { GOLD } else { DIM }, 1);
    Circle::new(Point::new(x - 20, y - 20), 41)
        .into_styled(style)
        .draw(f)
        .ok();
    for k in 0..11 {
        let angle = -PI * 0.75 + k as f32 * PI * 1.5 / 10.;
        let pt = |r: f32| Point::new(x + (angle.sin() * r) as i32, y - (angle.cos() * r) as i32);
        Line::new(pt(22.), pt(25.))
            .into_styled(PrimitiveStyle::with_stroke(DIM, 1))
            .draw(f)
            .ok();
    }
    Circle::new(Point::new(x - 15, y - 15), 31)
        .into_styled(PrimitiveStyle::with_fill(Rgb565::new(4, 7, 3)))
        .draw(f)
        .ok();
    Circle::new(Point::new(x - 13, y - 13), 27)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 18, 8), 1))
        .draw(f)
        .ok();
    let angle = -PI * 0.75 + a.shared.controls.norm(i) * PI * 1.5;
    line(
        f,
        x + (angle.sin() * 6.) as i32,
        y - (angle.cos() * 6.) as i32,
        x + (angle.sin() * 13.) as i32,
        y - (angle.cos() * 13.) as i32,
        INK,
    );
    centered(f, LABELS[i], x, y + 37, if selected { GOLD } else { INK });
    centered(
        f,
        &a.shared.controls.text(i),
        x,
        y - 29,
        if selected { GOLD } else { DIM },
    );
}
fn switch(f: &mut FrameBuffer, a: &ProphetApp, i: usize, x: i32, y: i32) {
    let selected = i == a.selected;
    rect(f, x - 20, y - 12, 40, 24, Rgb565::new(6, 9, 4));
    Rectangle::new(Point::new(x - 20, y - 12), Size::new(40, 24))
        .into_styled(PrimitiveStyle::with_stroke(
            if selected { GOLD } else { DIM },
            1,
        ))
        .draw(f)
        .ok();
    let choice = a.shared.controls.choice(i);
    Circle::new(Point::new(x - 3, y - 4), 7)
        .into_styled(PrimitiveStyle::with_fill(if choice > 0 {
            RED
        } else {
            Rgb565::new(10, 1, 0)
        }))
        .draw(f)
        .ok();
    centered(
        f,
        &a.shared.controls.text(i),
        x,
        y + 25,
        if choice > 0 { INK } else { DIM },
    );
    centered(f, LABELS[i], x, y + 39, if selected { GOLD } else { INK });
}
const WHITE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
fn white_note(k: usize) -> u8 {
    48 + (k / 7) as u8 * 12 + WHITE[k % 7]
}
fn black_note(k: usize) -> Option<u8> {
    match k % 7 {
        0 | 1 | 3 | 4 | 5 => Some(white_note(k) + 1),
        _ => None,
    }
}
fn keyboard(a: &ProphetApp, f: &mut FrameBuffer) {
    for k in 0..21 {
        let n = white_note(k);
        let held = a.input.midi_keys.0[n as usize] > 0
            || a.pointer_note == Some(n)
            || a.input.grid.iter().enumerate().any(|(i, &h)| {
                h && crate::apps::mi_kit::pad_note(i, a.shared.controls.raw(OCTAVE).round() as i32)
                    == n
            });
        rect(
            f,
            26 + k as i32 * 28,
            295,
            27,
            35,
            if held { GOLD } else { INK },
        );
        if k % 7 == 0 {
            text(
                f,
                &format!("C{}", n / 12 - 1),
                31 + k as i32 * 28,
                325,
                BLACK,
            );
        }
    }
    for k in 0..20 {
        if let Some(n) = black_note(k) {
            let held = a.input.midi_keys.0[n as usize] > 0 || a.pointer_note == Some(n);
            rect(
                f,
                46 + k as i32 * 28,
                295,
                15,
                21,
                if held { RED } else { Rgb565::new(1, 2, 1) },
            );
        }
    }
}

fn filter_response(a: &ProphetApp, f: &mut FrameBuffer) {
    use rustfft::num_complex::Complex;
    text(f, "24 dB/OCT / FILTER RESPONSE", 32, 211, DIM);
    let cutoff = cutoff(a.shared.controls.raw(CUTOFF));
    let g = (PI * cutoff / 96000.).tan();
    let k = 3.92 * a.shared.controls.raw(RES);
    line(f, 33, 255, 306, 255, DIM);
    let mut last = None;
    for x in 0..272 {
        let hz = 20. * 1000.0f32.powf(x as f32 / 271.);
        let h = Complex::new(g, 0.) / Complex::new(g, (PI * hz / 96000.).tan());
        let h4 = h * h * h * h;
        let response = h4 / (Complex::new(1., 0.) + h4 * k);
        let db = 20. * response.norm().max(0.00001).log10();
        let y = 216 + ((8. - db.clamp(-60., 8.)) / 68. * 36.) as i32;
        if let Some((px, py)) = last {
            line(f, px, py, 34 + x, y, GOLD);
        }
        last = Some((34 + x, y));
    }
    text(f, "20Hz", 34, 268, DIM);
    text(f, "20kHz", 276, 268, DIM);
    text(f, "FILTER ENVELOPE", 347, 215, DIM);
    text(
        f,
        &format!(
            "A {}  D {}",
            a.shared.controls.text(FA),
            a.shared.controls.text(FD)
        ),
        347,
        233,
        INK,
    );
    text(
        f,
        &format!(
            "S {}  R {}",
            a.shared.controls.text(FS),
            a.shared.controls.text(FR)
        ),
        347,
        250,
        INK,
    );
}
pub fn draw(a: &ProphetApp, f: &mut FrameBuffer) {
    rect(f, 0, 0, 640, 360, BLACK);
    for x in [0, 620] {
        rect(f, x, 0, 20, 360, Rgb565::new(10, 11, 3));
        for k in 0..9 {
            let xx = x + 1 + k * 2;
            line(
                f,
                xx,
                0,
                xx + ((k % 3) - 1),
                359,
                Rgb565::new(13 + (k % 3) as u8, 14, 4),
            );
        }
    }
    text(f, "SZYK", 28, 17, DIM);
    Text::new(
        "PROPHET",
        Point::new(26, 46),
        MonoTextStyle::new(&SPLEEN_16X32, INK),
    )
    .draw(f)
    .ok();
    text(f, "POLYPHONIC VIRTUAL ANALOG", 157, 39, DIM);
    rect(f, 348, 9, 48, 40, Rgb565::new(7, 1, 0));
    Text::new(
        &format!("{:02}", a.program + 1),
        Point::new(353, 41),
        MonoTextStyle::new(&SPLEEN_16X32, RED),
    )
    .draw(f)
    .ok();
    text(f, &short(a.name(), 28), 410, 22, INK);
    text(
        f,
        &format!(
            "{} / {}V {}{}",
            a.preset(a.program).map_or("USER", |p| p.bank.as_str()),
            a.shared.controls.text(VOICES),
            a.shared.controls.text(MODE),
            if a.dirty() { " *" } else { "" }
        ),
        410,
        39,
        DIM,
    );
    for i in 0..8 {
        rect(
            f,
            545 + i as i32 * 8,
            45,
            5,
            3,
            if a.shared.meters[i].get() > 0.01 {
                RED
            } else {
                Rgb565::new(7, 1, 0)
            },
        );
    }
    line(f, 26, 55, 613, 55, DIM);
    button(f, "<", 26, 62, 26, false);
    button(f, ">", 57, 62, 26, false);
    button(f, "PATCHES", 91, 62, 66, a.view == 1);
    button(f, "COMPARE", 165, 62, 66, a.edited.is_some());
    button(f, "INIT", 239, 62, 44, false);
    button(f, "SAVE", 291, 62, 44, false);
    text(f, "TO", 347, 76, DIM);
    button(f, "-", 370, 62, 22, false);
    button(f, &format!("U{:02}", a.user_slot + 1), 399, 62, 40, false);
    button(f, "+", 446, 62, 22, false);
    button(
        f,
        if a.view == 2 { "PANEL" } else { "MENU" },
        478,
        62,
        56,
        a.view == 2,
    );
    rect(f, 549, 67, 62, 5, Rgb565::new(7, 9, 4));
    rect(
        f,
        549,
        67,
        (a.shared.peak.get() * 62.).clamp(0., 62.) as u32,
        5,
        GOLD,
    );
    text(f, "OUTPUT", 563, 83, DIM);
    if a.view == 1 {
        for b in 0..10 {
            button(
                f,
                &format!(
                    "{}",
                    if b < 8 {
                        (b + 1).to_string()
                    } else {
                        format!("U{}", b - 7)
                    }
                ),
                26 + b as i32 * 59,
                91,
                54,
                a.bank == b,
            );
        }
        text(f, &format!("{} / 8 PROGRAMS", a.bank_name()), 31, 126, INK);
        for slot in 0..8 {
            let i = a.bank * 8 + slot;
            let p = a.preset(i);
            let x = 30 + (slot % 2) as i32 * 295;
            let y = 138 + (slot / 2) as i32 * 34;
            button(
                f,
                &format!(
                    "{:02} {}",
                    i + 1,
                    p.map_or("-- empty --", |p| p.name.as_str())
                ),
                x,
                y,
                283,
                a.program == i,
            );
        }
        text(f, "Pads 1-8: load / pads 9-16: factory bank", 31, 280, DIM);
    } else {
        for (g, (name, _)) in GROUPS.iter().enumerate() {
            button(f, name, 26 + g as i32 * 74, 91, 70, a.group == g);
        }
        if a.view == 2 {
            for (k, &i) in GROUPS[a.group].1.iter().enumerate() {
                let y = 126 + k as i32 * 11;
                if i == a.selected {
                    rect(f, 28, y - 9, 582, 12, Rgb565::new(6, 9, 3));
                }
                text(
                    f,
                    SPECS[i].name,
                    36,
                    y,
                    if i == a.selected { GOLD } else { INK },
                );
                text(
                    f,
                    &a.shared.controls.text(i),
                    376,
                    y,
                    if i == a.selected { GOLD } else { DIM },
                );
            }
        } else {
            for (k, &i) in GROUPS[a.group].1.iter().enumerate() {
                let (x, y, _) = cell(a, k);
                if SPECS[i].is_switch() {
                    switch(f, a, i, x, y);
                } else {
                    dial(f, a, i, x, y);
                }
            }
            if a.group == 3 {
                filter_response(a, f);
            }
        }
        line(f, 26, 273, 613, 273, DIM);
        text(
            f,
            &short(
                &format!(
                    "{}  /  {}",
                    SPECS[a.selected].name,
                    a.shared.controls.text(a.selected)
                ),
                95,
            ),
            31,
            286,
            GOLD,
        );
    }
    keyboard(a, f);
    text(
        f,
        &short(&a.status, 96),
        29,
        347,
        if a.edited.is_some() { GOLD } else { DIM },
    );
}

fn keyboard_pick(x: f32, y: f32) -> Option<u8> {
    if !(26.0..614.0).contains(&x) || !(295.0..330.0).contains(&y) {
        return None;
    }
    if y < 316. {
        for k in 0..20 {
            if let Some(n) = black_note(k) {
                if (46. + k as f32 * 28. ..61. + k as f32 * 28.).contains(&x) {
                    return Some(n);
                }
            }
        }
    }
    Some(white_note(((x - 26.) / 28.) as usize))
}
pub fn pointer(a: &mut ProphetApp, x: f32, y: f32) {
    if x < 0. || y < 0. {
        a.pointer_note = None;
        a.drag = None;
        a.publish_keys();
        return;
    }
    if let Some(n) = keyboard_pick(x, y) {
        a.pointer_note = Some(n);
        a.drag = None;
        a.publish_keys();
        return;
    }
    if a.pointer_note.take().is_some() {
        a.publish_keys();
    }
    if let Some((i, sy, initial)) = a.drag {
        if !SPECS[i].is_switch() {
            a.shared.controls.set_norm(i, initial + (sy - y) / 120.);
            a.status = format!("{}: {}", SPECS[i].name, a.shared.controls.text(i));
        }
        return;
    }
    // A sentinel drag also prevents a held touch from repeatedly loading,
    // saving, or toggling an action while its pointer moves.
    if (62.0..81.0).contains(&y) {
        a.drag = Some((A_SAW, y, 0.));
        if x < 53. {
            a.step_program(-1);
        } else if x < 84. {
            a.step_program(1);
        } else if x < 158. {
            a.view = if a.view == 1 { 0 } else { 1 };
            a.publish_keys();
        } else if x < 232. {
            a.compare();
        } else if x < 284. {
            a.load(63);
        } else if x < 336. {
            a.save_action();
        } else if (370.0..393.0).contains(&x) {
            a.user_slot = (a.user_slot + 15) % 16;
        } else if (446.0..469.0).contains(&x) {
            a.user_slot = (a.user_slot + 1) % 16;
        } else if (478.0..534.0).contains(&x) {
            a.view = if a.view == 2 { 0 } else { 2 };
            a.publish_keys();
        }
        return;
    }
    if (91.0..110.0).contains(&y) && (26.0..614.0).contains(&x) {
        a.drag = Some((A_SAW, y, 0.));
        if a.view == 1 {
            a.bank = (((x - 26.) / 59.) as usize).min(9);
        } else {
            a.select_group((((x - 26.) / 74.) as usize).min(7));
        }
        return;
    }
    if a.view == 1 {
        for slot in 0..8 {
            let sx = 30. + (slot % 2) as f32 * 295.;
            let sy = 138. + (slot / 2) as f32 * 34.;
            if (sx..sx + 283.).contains(&x) && (sy..sy + 19.).contains(&y) {
                a.load(a.bank * 8 + slot);
                a.drag = Some((A_SAW, y, 0.));
                return;
            }
        }
    } else if a.view == 2 {
        let k = ((y - 117.) / 11.).floor() as i32;
        if y >= 117. && k >= 0 {
            if let Some(&i) = GROUPS[a.group].1.get(k as usize) {
                a.selected = i;
            }
        }
    } else {
        for (k, &i) in GROUPS[a.group].1.iter().enumerate() {
            let (cx, cy, w) = cell(a, k);
            if (x - cx as f32).abs() < w as f32 * 0.45 && (y - cy as f32).abs() < 30. {
                a.leave_compare();
                a.selected = i;
                if SPECS[i].is_switch() {
                    a.shared.controls.edit(i, 1, 0.5);
                }
                a.drag = Some((i, y, a.shared.controls.norm(i)));
                return;
            }
        }
    }
}
