//! Lossless Rev2 program/edit-buffer codec. Unused bytes stay intact.
use super::spec;
pub const RAW_LEN: usize = 2046;
pub const PACKED_LEN: usize = 2339;
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Patch {
    pub data: [u8; 2048],
    pub location: Option<(u8, u8)>,
}
impl Default for Patch {
    fn default() -> Self {
        let mut p = Self {
            data: [0; 2048],
            location: None,
        };
        for layer in 0..2 {
            let b = layer * 1024;
            for (i, v) in [
                (0, 24),
                (1, 24),
                (2, 50),
                (3, 50),
                (4, 1),
                (5, 1),
                (6, 50),
                (7, 50),
                (10, 1),
                (11, 1),
                (20, 2),
                (22, 52),
                (24, 64),
                (26, 1),
                (28, 100),
                (32, 127),
                (33, 127),
                (34, 127),
                (44, 40),
                (45, 40),
                (47, 64),
                (48, 127),
                (50, 40),
                (51, 40),
                (130, 120),
                (139, 1),
                (232, 60),
            ] {
                p.data[b + i] = v;
            }
            for i in 85..=92 {
                p.data[b + i] = 127;
            }
            for i in [101, 103, 105, 107, 109] {
                p.data[b + i] = 127;
            }
            p.set_name(layer, "Init Rev2");
        }
        p
    }
}
impl Patch {
    pub fn name(&self, layer: usize) -> String {
        let b = layer.min(1) * 1024 + 235;
        self.data[b..b + 20]
            .iter()
            .map(|&c| {
                if (32..127).contains(&c) {
                    c as char
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .trim()
            .to_string()
    }
    pub fn set_name(&mut self, layer: usize, name: &str) {
        let b = layer.min(1) * 1024 + 235;
        self.data[b..b + 20].fill(b' ');
        for (dst, c) in self.data[b..b + 20].iter_mut().zip(name.chars()) {
            *dst = if c.is_ascii() && !c.is_control() {
                c as u8
            } else {
                b'?'
            };
        }
    }
    pub fn set(&mut self, offset: usize, value: u8) -> Result<(), String> {
        if offset >= 2048 {
            return Err("Parameter outside patch".into());
        }
        let i = offset % 1024;
        if offset >= 1024 && [231, 232].contains(&i) {
            return Err("Global parameter is only in layer A".into());
        }
        let max = if i >= 256 {
            if (i - 256) % 128 < 64 {
                128
            } else {
                255
            }
        } else {
            spec::parameter(i).ok_or("Reserved parameter")?.max
        };
        if value < spec::minimum(i) || value > max {
            return Err(format!(
                "Parameter {offset} outside {}..{max}",
                spec::minimum(i)
            ));
        }
        self.data[offset] = value;
        Ok(())
    }
    pub fn export(&self, location: Option<(u8, u8)>) -> Result<Vec<u8>, String> {
        let mut out = vec![0xf0, 1, 0x2f, if location.is_some() { 2 } else { 3 }];
        if let Some((bank, program)) = location {
            if bank > 7 || program > 127 {
                return Err("Invalid bank/program".into());
            }
            out.extend([bank, program]);
        }
        for chunk in self.data[..RAW_LEN].chunks(7) {
            let mask = chunk
                .iter()
                .enumerate()
                .fold(0, |m, (i, b)| m | ((b >> 7) << i));
            out.push(mask);
            out.extend(chunk.iter().map(|b| b & 127));
        }
        out.push(0xf7);
        Ok(out)
    }
}
fn decode(message: &[u8]) -> Result<Patch, String> {
    if message.len() < 5 || message[..3] != [0xf0, 1, 0x2f] || message.last() != Some(&0xf7) {
        return Err("Not a Sequential Rev2 SysEx dump".into());
    }
    let (start, location) = match message[3] {
        2 if message.len() >= 7 => {
            if message[4] > 7 || message[5] > 127 {
                return Err("Invalid bank/program".into());
            }
            (6, Some((message[4], message[5])))
        }
        3 => (4, None),
        _ => return Err("Expected a Rev2 program or edit-buffer dump".into()),
    };
    let packed = &message[start..message.len() - 1];
    if packed.len() != PACKED_LEN || packed.iter().any(|&v| v >= 128) {
        return Err("Invalid Rev2 dump length or MIDI data byte".into());
    }
    let mut p = Patch {
        data: [0; 2048],
        location,
    };
    let mut index = 0;
    for block in packed.chunks(8) {
        if block.len() < 2 || block[0] >> (block.len() - 1) != 0 {
            return Err("Invalid final packing mask".into());
        }
        for (i, b) in block[1..].iter().enumerate() {
            p.data[index] = b | (((block[0] >> i) & 1) << 7);
            index += 1;
        }
    }
    // The hardware omits B track 6 velocities 63/64. They are unavailable,
    // not duplicated from A or inferred from neighbouring steps.
    if index != RAW_LEN {
        return Err("Incomplete Rev2 patch".into());
    }
    Ok(p)
}
/// Atomic bank import: reject the whole input on a bad message, no partial bank.
pub fn import(bytes: &[u8]) -> Result<Vec<Patch>, String> {
    let mut message = Vec::new();
    let mut patches = Vec::new();
    let mut inside = false;
    for &b in bytes {
        if b >= 0xf8 {
            continue;
        } // MIDI realtime may interrupt SysEx.
        match b {
            0xf0 => {
                if inside {
                    return Err("Nested SysEx start".into());
                }
                inside = true;
                message.clear();
                message.push(b);
            }
            0xf7 => {
                if !inside {
                    return Err("Unexpected SysEx end".into());
                }
                message.push(b);
                patches.push(decode(&message)?);
                inside = false;
            }
            _ if inside => message.push(b),
            _ if b.is_ascii_whitespace() => {}
            _ => return Err("Data outside SysEx message".into()),
        }
    }
    if inside {
        return Err("Truncated SysEx".into());
    }
    if patches.is_empty() {
        return Err("No Rev2 patches found".into());
    }
    Ok(patches)
}
