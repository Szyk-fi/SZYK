use super::{ProphetApp, SECTIONS};
use crate::{
    display::FrameBuffer,
    spleen_fonts::{SPLEEN_16X32, SPLEEN_6X12},
};
use embedded_graphics::{
    mono_font::MonoTextStyle,
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::Text,
};
const BG: Rgb565 = Rgb565::new(2, 3, 3);
const INK: Rgb565 = Rgb565::new(29, 56, 25);
const GOLD: Rgb565 = Rgb565::new(30, 44, 12);
const DIM: Rgb565 = Rgb565::new(14, 27, 13);
fn rect(f: &mut FrameBuffer, x: i32, y: i32, w: u32, h: u32, c: Rgb565) {
    Rectangle::new(Point::new(x, y), Size::new(w, h))
        .into_styled(PrimitiveStyle::with_fill(c))
        .draw(f)
        .ok();
}
fn text(f: &mut FrameBuffer, s: &str, x: i32, y: i32, c: Rgb565) {
    Text::new(s, Point::new(x, y), MonoTextStyle::new(&SPLEEN_6X12, c))
        .draw(f)
        .ok();
}
pub fn draw(a: &ProphetApp, f: &mut FrameBuffer) {
    f.clear(BG).ok();
    rect(f, 0, 0, 9, 360, Rgb565::new(12, 8, 4));
    rect(f, 631, 0, 9, 360, Rgb565::new(12, 8, 4));
    Text::new(
        "REV2",
        Point::new(22, 35),
        MonoTextStyle::new(&SPLEEN_16X32, INK),
    )
    .draw(f)
    .ok();
    text(f, "PORTAMAX / SOFTWARE INSTRUMENT", 112, 19, DIM);
    text(
        f,
        &format!(
            "{}{}",
            a.state.patch.name(0),
            if a.dirty() { " *" } else { "" }
        ),
        112,
        37,
        GOLD,
    );
    text(
        f,
        &format!(
            "EDIT {}   {} voices",
            if a.layer == 0 { "A" } else { "B" },
            a.state.voices
        ),
        480,
        20,
        GOLD,
    );
    text(
        f,
        if a.state.playing {
            "SEQ PLAY"
        } else {
            "SEQ STOP"
        },
        480,
        37,
        INK,
    );
    for (i, s) in SECTIONS.iter().enumerate() {
        let x = 20 + (i % 8) as i32 * 75;
        let y = 51 + (i / 8) as i32 * 24;
        rect(
            f,
            x,
            y,
            72,
            21,
            if i == a.section {
                Rgb565::new(8, 14, 7)
            } else {
                Rgb565::new(3, 6, 4)
            },
        );
        let label: String = s.chars().take(11).collect();
        text(
            f,
            &label,
            x + 3,
            y + 14,
            if i == a.section { GOLD } else { DIM },
        );
    }
    let rows = a.rows();
    let start = a.selected / 7 * 7;
    for (line, r) in rows.iter().skip(start).take(7).enumerate() {
        let y = 110 + line as i32 * 23;
        if start + line == a.selected {
            rect(f, 20, y - 14, 599, 22, Rgb565::new(5, 10, 6));
        }
        text(
            f,
            &r.name,
            28,
            y,
            if start + line == a.selected {
                GOLD
            } else {
                INK
            },
        );
        text(f, &r.value, 370, y, INK);
    }
    if (10..=13).contains(&a.section) {
        text(
            f,
            &format!(
                "Steps {}-{} | page: tap here | velocity track {}",
                a.seq_page * 16 + 1,
                a.seq_page * 16 + 16,
                a.track + 1
            ),
            22,
            283,
            GOLD,
        );
    }
    let status: String = a.status.chars().take(96).collect();
    text(f, &status, 22, 301, DIM);
    for key in 0..24 {
        let x = 20 + key * 25;
        let n = (48 + key + a.octave * 12 + a.transpose).clamp(0, 127) as usize;
        let on = a.state.notes[n] > 0;
        rect(
            f,
            x,
            310,
            23,
            29,
            if on { GOLD } else { Rgb565::new(20, 38, 19) },
        );
        if [1, 3, 6, 8, 10].contains(&(n % 12)) {
            rect(
                f,
                x + 9,
                310,
                12,
                17,
                if on {
                    Rgb565::new(16, 22, 4)
                } else {
                    Rgb565::new(1, 2, 1)
                },
            );
        }
    }
    text(
        f,
        "L1 layer  R1 section  Encoders select/edit  F3 play  Click key again: release",
        22,
        354,
        DIM,
    );
}
pub fn pointer(a: &mut ProphetApp, x: f32, y: f32) {
    if !x.is_finite() || !y.is_finite() {
        return;
    }
    if (51.0..99.0).contains(&y) && (20.0..620.0).contains(&x) {
        a.section = ((y as usize - 51) / 24 * 8 + (x as usize - 20) / 75).min(15);
        a.selected = 0;
    } else if (270.0..289.0).contains(&y) && (10..=13).contains(&a.section) {
        a.seq_page = (a.seq_page + 1) % 4;
        a.track = (a.track + 1) % 6;
        a.selected = 0;
    } else if (100.0..270.0).contains(&y) {
        let i = a.selected / 7 * 7 + (y as usize - 100) / 23;
        if i < a.rows().len() {
            if a.selected == i {
                a.edit(1, true);
            } else {
                a.selected = i;
            }
        }
    } else if (310.0..340.0).contains(&y) && (20.0..620.0).contains(&x) {
        let n = (48 + (x as i32 - 20) / 25 + a.octave * 12 + a.transpose).clamp(0, 127) as u8;
        a.pointer = if a.pointer == Some(n) { None } else { Some(n) };
        let input = a.input;
        a.keys(&input);
    } else if y < 50. && x > 470. {
        a.layer = 1 - a.layer;
    }
}
