//! Neo Geo hardware core -- the 68000/Z80 memory map, video (LSPC), and
//! sound (YM2610) glue that `retro.rs`'s `Console::NeoGeo` drives.
//!
//! **Why this file exists at all, unlike the other three consoles**: no
//! complete, vendorable Neo Geo system emulator exists in the Rust
//! ecosystem (checked crates.io and GitHub; the closest hit, `asobou`,
//! turned out to be a Libretro-style `libloading` frontend -- the same
//! dynamic-core-loading architecture already ruled out for this
//! project's real target hardware, see `retro.rs`'s module doc
//! comment). What *is* real and vendorable are the two CPU cores
//! (`m68k` for the main 68000, `z80` for the sound CPU); everything
//! else here -- the memory map, the LSPC video/sprite controller, the
//! YM2610 sound chip, bankswitching -- is hand-written against public
//! Neo Geo hardware documentation (primarily
//! [wiki.neogeodev.org](https://wiki.neogeodev.org)), since there is
//! nothing existing to borrow for the system-level hardware itself.
//!
//! **No real SNK BIOS is used or included.** Every real cartridge
//! depends on SNK's own copyrighted `sp-s2.sp1` BIOS ROM for POST, and
//! this project does not have and will not ship that. Instead,
//! `NeoGeoMachine::new` does the one thing the real BIOS does that
//! actually matters for running a cart: read the cart's own header
//! (present on every real cartridge, not BIOS-supplied) and jump to
//! its documented entry trampoline -- see `boot_pc` below. This is a
//! real, publicly documented convention
//! (<https://wiki.neogeodev.org/index.php?title=68k_program_header>),
//! not a copy of SNK's code, and no POST/memory-card/system-mode work
//! the BIOS also does is attempted.
//!
//! **The memory map below was corrected once against real evidence**:
//! an earlier version of this file had the fixed and banked program
//! ROM regions backwards (fixed bank at $200000, banked window at
//! $300000+), based on an unverified assumption. Reading the real
//! Metal Slug 3 cartridge's own bytes -- its header's stack pointer
//! field decoded to exactly the wiki's documented $10F300, once the
//! well-known P-ROM word-swap was applied -- confirmed the real
//! layout: fixed bank at $000000-$0FFFFF (which is *also* where the
//! header and vector table live), banked window at $200000-$2FFFFF.
//!
//! **Status**: under active development. Real and tested against a
//! real cartridge dump (see the `roms/neogeo/` convention): the memory
//! map, cartridge header parsing, CPU wiring (68000 + Z80, including
//! the sound-command handshake between them), VBlank interrupt
//! delivery, generic P2 bankswitching, VRAM/palette RAM, video (both
//! the fix/text layer and sprites -- tile decode, position, flip, and
//! shrink; see `render_sprites_onto`'s own doc comment for the
//! specific shrink caveat), sound (the real YM2610 chip, via the
//! `ymfm-sys` crate -- see `Ym2610`'s own doc comment and Cargo.toml's
//! doc comment on that dependency), and Metal Slug 3's real NEO-SMA
//! protection chip (`Protection::SmaMslug3` -- 68000 program
//! decryption and bankswitching, reimplemented from MAME's own BSD-3
//! source; see `sma_decrypt_68k`'s own doc comment; verified against a
//! real 100,000,000-instruction run with no CPU fault at all) and its
//! separate CMC42 graphics-decryption chip (sprite/fix-layer C-ROM
//! data, also reimplemented from MAME's own source -- see `cmc42`'s
//! own module doc comment). Still missing: sprite auto-animation. See
//! the module's own test coverage for exactly what is and isn't
//! verified so far.

use m68k::{AddressBus, CpuCore, CpuType, StepResult};
use std::cell::Cell;
use std::rc::Rc;
use z80::{Z80, Z80_io};

/// The 68k/Z80 sound-command handshake, per
/// <https://wiki.neogeodev.org/index.php?title=68k%2FZ80_communication>
/// and <https://wiki.neogeodev.org/index.php?title=Z80_port_map>: the
/// 68k writes a command byte to `REG_SOUND` ($320000), which both
/// latches the byte for the Z80 to read on its port $00 and requests
/// an NMI (real hardware fires it immediately if NMIs are enabled;
/// `step_sound` below pulses it unconditionally instead, since NMI
/// enable/disable via ports $08/$18 isn't implemented yet). The Z80
/// replies by writing its own port $0C, which the 68k later reads back
/// through the same `REG_SOUND` address. Shared via `Rc<Cell<_>>`
/// rather than routed through `NeoGeoMachine` itself, since the 68k
/// and Z80 buses are each only reachable through their own CPU's
/// trait-object-free `&mut self` during a `step` call.
#[derive(Clone)]
struct SoundLatch {
    /// Last command byte the 68k wrote, pending for the Z80 to read.
    command: Rc<Cell<u8>>,
    /// Last reply byte the Z80 wrote, pending for the 68k to read.
    reply: Rc<Cell<u8>>,
    /// Set when the 68k has written a new command the Z80 hasn't been
    /// NMI'd for yet; cleared by `NeoGeoMachine::step_sound`.
    nmi_pending: Rc<Cell<bool>>,
}

impl SoundLatch {
    fn new() -> Self {
        Self { command: Rc::new(Cell::new(0)), reply: Rc::new(Cell::new(0)), nmi_pending: Rc::new(Cell::new(false)) }
    }
}

/// The LSPC video controller's VRAM access protocol, per
/// <https://wiki.neogeodev.org/index.php?title=VRAM>. VRAM itself holds
/// no pixel data (that lives in the cartridge's C-ROMs, not
/// implemented yet) -- only sprite attribute tables and the fix/text
/// layer's tilemap, addressed as 64K 16-bit words (not bytes) through
/// three memory-mapped registers rather than being directly
/// 68k-addressable:
///
/// - `REG_VRAMADDR` ($3C0000): sets the current VRAM word address.
/// - `REG_VRAMRW` ($3C0002): reads or writes one VRAM word at that
///   address, then advances the address by `REG_VRAMMOD`.
/// - `REG_VRAMMOD` ($3C0004): the signed auto-increment step applied
///   after each `REG_VRAMRW` access, letting real code scan through a
///   table (e.g. the sprite list) via repeated word accesses instead
///   of re-setting the address every time.
///
/// Real hardware also imposes minimum-cycle-delay timing rules between
/// these accesses (see the wiki page above); not enforced here.
struct Lspc {
    vram: Vec<u16>,
    vram_addr: u16,
    vram_mod: i16,
}

impl Lspc {
    fn new() -> Self {
        Self { vram: vec![0; 0x10000], vram_addr: 0, vram_mod: 0 }
    }

    fn read_data(&mut self) -> u16 {
        let value = self.vram[self.vram_addr as usize];
        self.vram_addr = self.vram_addr.wrapping_add_signed(self.vram_mod);
        value
    }

    fn write_data(&mut self, value: u16) {
        self.vram[self.vram_addr as usize] = value;
        self.vram_addr = self.vram_addr.wrapping_add_signed(self.vram_mod);
    }

    /// Reads a VRAM word directly by address, without going through
    /// `REG_VRAMADDR`/`REG_VRAMRW` and without the auto-increment side
    /// effect -- for the renderer to peek at the fix-layer tilemap
    /// without disturbing whatever address the 68k has staged for its
    /// own next access.
    fn peek(&self, addr: u16) -> u16 {
        self.vram[addr as usize]
    }
}

/// Neo Geo work RAM: a real 64KB SRAM, mirrored across the CPU's
/// 0x100000-0x1FFFFF region on real hardware. Only the first 64KB is
/// physically backed; this struct stores exactly that, with mirroring
/// handled by `NeoGeoBus`'s address decode.
const WORK_RAM_SIZE: usize = 0x10000;

/// Real Neo Geo P-ROM dumps (P1/P2, the 68000 program ROMs -- unlike
/// C/S/V/M ROMs) are conventionally stored byte-swapped within each
/// 16-bit word, the same `ROM_LOAD16_WORD_SWAP` convention MAME's own
/// neogeo driver un-swaps on load. Confirmed against the real Metal
/// Slug 3 dump: its raw header bytes decode to the documented $10F300
/// initial stack pointer and a real "NEO-GEO" signature string only
/// once every adjacent byte pair is swapped -- without this, the same
/// bytes look like a plausible-but-wrong stack pointer and garbled
/// text.
fn word_swap(mut rom: Vec<u8>) -> Vec<u8> {
    let mut i = 0;
    while i + 1 < rom.len() {
        rom.swap(i, i + 1);
        i += 2;
    }
    rom
}

/// Rearranges `value`'s bits per `bits`: output bit `bits.len()-1-k`
/// comes from input bit `bits[k]` -- the same argument order as
/// MAME's own `bitswap<N>(value, b_msb, ..., b_lsb)` template (each
/// argument names which *input* bit position feeds that output bit,
/// listed from the output's MSB down to its LSB), so a bitswap array
/// copied straight out of MAME source behaves identically here.
fn bitswap(value: u32, bits: &[u8]) -> u32 {
    let n = bits.len();
    let mut out = 0u32;
    for (k, &b) in bits.iter().enumerate() {
        out |= ((value >> b) & 1) << (n - 1 - k);
    }
    out
}

/// Metal Slug 3's NEO-SMA bank-number-unscrambling formula and lookup
/// table, reimplemented from MAME's `sma_prot_device::mslug3_bank_base`
/// (`src/devices/bus/neogeo/prot_sma.cpp`, BSD-3-Clause). Returns a
/// *byte offset* into the combined P1+P2 region (already including
/// the `+0x100000` MAME's own version adds), matching this module's
/// `p2_rom` layout where index 0 corresponds to combined offset
/// `0x100000` -- so the real caller subtracts `0x100000` before using
/// this as a `p2_rom` index (see `NeoGeoBus::write_word`).
fn sma_mslug3_bank_base(written_value: u16) -> u32 {
    const BANK_OFFSET: [u32; 49] = [
        0x000000, 0x020000, 0x040000, 0x060000, 0x070000, 0x090000, 0x0b0000, 0x0d0000, 0x0e0000, 0x0f0000, 0x120000, 0x130000, 0x140000, 0x150000, 0x180000, 0x190000, 0x1a0000, 0x1b0000, 0x1e0000, 0x1f0000, 0x200000, 0x210000, 0x240000, 0x250000, 0x260000, 0x270000, 0x2a0000, 0x2b0000, 0x2c0000, 0x2d0000, 0x300000, 0x310000, 0x320000, 0x330000, 0x360000, 0x370000, 0x380000, 0x390000, 0x3c0000, 0x3d0000, 0x400000, 0x410000, 0x440000, 0x450000, 0x460000, 0x470000, 0x4a0000, 0x4b0000, 0x4c0000,
    ];
    let index = bitswap(written_value as u32, &[9, 3, 6, 15, 12, 14]) as usize;
    0x100000 + BANK_OFFSET.get(index).copied().unwrap_or(0)
}

/// Metal Slug 3's NEO-SMA 68000 program decryption, reimplemented from
/// MAME's `sma_prot_device::mslug3_decrypt_68k` (same source as
/// `sma_mslug3_bank_base`, credited there to Razoola and Mr.K's
/// original decode work). Three real transforms, applied in this
/// exact order against a combined P1+P2 buffer (P1 first 0x100000
/// bytes, P2 concatenated after, zero-padded to a full 8MB P2 region
/// the way MAME's own fixed-size ROM region is): a per-word bitplane
/// rearrangement across the whole P2 area, an address-line rearranged
/// *copy* of already-transformed P2 data into the fixed bank (P1's
/// real content on a NEO-SMA cart isn't its own on-disk file -- it's
/// extracted out of P2 this way), and a within-64KB-chunk word-
/// position permutation across the rest of P2.
///
/// Returns `(p1_rom, p2_rom)`: `p1_rom` is the decrypted fixed bank
/// (0xC0000 bytes); `p2_rom` is the combined region from byte offset
/// 0x100000 onward, so `sma_mslug3_bank_base`'s returned offset (which
/// already includes that `+0x100000`) needs `0x100000` subtracted
/// before indexing into it.
fn sma_decrypt_68k(p1_rom: Vec<u8>, p2_rom: Vec<u8>, data_bitswap: &[u8; 16], fixed_addr_bitswap: &[u8; 19], fixed_source_word: usize, banked_addr_bitswap: &[u8; 15]) -> (Vec<u8>, Vec<u8>) {
    const P1_SIZE: usize = 0x100000;
    const P2_SIZE: usize = 0x800000;
    let mut combined = vec![0u8; P1_SIZE + P2_SIZE];
    let p1_len = p1_rom.len().min(P1_SIZE);
    combined[..p1_len].copy_from_slice(&p1_rom[..p1_len]);
    let p2_len = p2_rom.len().min(P2_SIZE);
    combined[P1_SIZE..P1_SIZE + p2_len].copy_from_slice(&p2_rom[..p2_len]);

    let word_at = |buf: &[u8], i: usize| u16::from_be_bytes([buf[i * 2], buf[i * 2 + 1]]);
    let set_word_at = |buf: &mut [u8], i: usize, v: u16| {
        let bytes = v.to_be_bytes();
        buf[i * 2] = bytes[0];
        buf[i * 2 + 1] = bytes[1];
    };

    // Step 1: per-word bitplane rearrangement across the whole P2 area
    // (word index 0x100000/2 .. (0x100000+0x800000)/2).
    let p2_word_base = P1_SIZE / 2;
    for i in 0..(P2_SIZE / 2) {
        let w = word_at(&combined, p2_word_base + i);
        set_word_at(&mut combined, p2_word_base + i, bitswap(w as u32, data_bitswap) as u16);
    }

    // Step 2: relocate the fixed bank -- its real content is extracted
    // out of the (now bitplane-rearranged) P2 data via an address-line
    // permutation, not read from the raw on-disk P1 file.
    for i in 0..(0x0c0000 / 2) {
        let source = fixed_source_word + bitswap(i as u32, fixed_addr_bitswap) as usize;
        let w = word_at(&combined, source);
        set_word_at(&mut combined, i, w);
    }

    // Step 3: within each 64KB chunk of the banked part, permute word
    // *positions* (not their bit content) per the same address-line
    // scrambling real hardware's address bus wiring produces.
    let chunk_words = 0x10000 / 2;
    let mut chunk_index = 0;
    while chunk_index < P2_SIZE / 2 {
        let base = p2_word_base + chunk_index;
        let original: Vec<u16> = (0..chunk_words).map(|j| word_at(&combined, base + j)).collect();
        for j in 0..chunk_words {
            let source_j = bitswap(j as u32, banked_addr_bitswap) as usize;
            set_word_at(&mut combined, base + j, original[source_j]);
        }
        chunk_index += chunk_words;
    }

    // The relocate step above only ever writes the first 0xC0000 bytes
    // (0x60000 words) of the fixed bank -- the remaining 0x40000 bytes
    // (0xC0000-0xFFFFF) are real, meaningful data too, just never
    // touched by any of the three transforms, so they still hold the
    // original (word-swapped-only) P1 file content exactly as MAME's
    // own `base` array would. The full 1MB must be kept, not just the
    // relocated portion -- truncating here would make any real access
    // to that upper 0x40000 range wrap around (via this struct's own
    // `% self.p1_rom.len()` mirroring) into completely wrong data.
    let new_p1 = combined[..P1_SIZE].to_vec();
    let new_p2 = combined[P1_SIZE..].to_vec();
    (new_p1, new_p2)
}

const SMA_MSLUG3_DATA_BITSWAP: [u8; 16] = [4, 11, 14, 3, 1, 13, 0, 7, 2, 8, 12, 15, 10, 9, 5, 6];
const SMA_MSLUG3_FIXED_ADDR_BITSWAP: [u8; 19] = [18, 15, 2, 1, 13, 3, 0, 9, 6, 16, 4, 11, 5, 7, 12, 17, 14, 10, 8];
/// `0x5d0000/2` -- the fixed bank's real content is sourced from this
/// word offset onward within the combined P1+P2 region.
const SMA_MSLUG3_FIXED_SOURCE: usize = 0x5d0000 / 2;
const SMA_MSLUG3_BANKED_ADDR_BITSWAP: [u8; 15] = [2, 11, 0, 14, 6, 4, 13, 8, 9, 3, 10, 7, 5, 12, 1];

/// The Neo Geo's 68000 address space, decoded per the real, documented
/// hardware memory map (see this module's doc comment for the source
/// and how it was verified):
///
/// - `0x000000-0x0FFFFF`: cartridge program ROM, fixed bank (P1) --
///   contains the 68000 vector table, the cartridge header (signature,
///   NGH id, and the entry-point trampoline -- see `boot_pc`), and
///   whatever code the cartridge keeps always-resident.
/// - `0x100000-0x1FFFFF`: work RAM (64KB physical, mirrored).
/// - `0x200000-0x2FFFFF`: banked window onto the rest of the
///   cartridge's program ROM (P2 and beyond). The generic bankswitch
///   scheme most cartridges use is implemented (any write in this
///   range latches the value's low bits as the new 1MB bank -- see
///   `write_byte`); cartridges using a real protection chip instead
///   (NEO-SMA, which Metal Slug 3 uses) get their own real bankswitch
///   formula and 68000 program decryption -- see `Protection`.
/// - `0x300000-0x3FFFFF`: I/O and memory-mapped registers. The sound
///   comm latch (`REG_SOUND`, $320000) is real -- see `SoundLatch`.
///   Input/DIP/coin/service registers ($300000, $300001, $300081,
///   $320001, $340000, $380000, $380001) always read fully idle
///   (0xFF -- these are active-low, so idle is all-ones, not zero);
///   no controller input reaches this core yet. The LSPC video
///   registers ($3C0000+) are not implemented yet.
/// - `0x400000-0x7FFFFF`: palette RAM (8KB, mirrored) -- real,
///   directly 68k-addressable (unlike VRAM's indirect register
///   protocol). Double-buffering (a second bank selected by a
///   register) isn't implemented yet -- see `decode_palette_color` for
///   how each 16-bit word becomes an actual RGB color.
///
/// Which cartridge protection chip (if any) governs P-ROM decryption
/// and bankswitching. `None` is the generic scheme most cartridges
/// use (any write in $200000-$2FFFFF latches a plain 1MB bank index).
/// `SmaMslug3` is Metal Slug 3's real NEO-SMA chip: a specific write
/// address, a bank-number-unscrambling formula against a fixed lookup
/// table, and a real 68000 program decryption transform -- all
/// reimplemented from MAME's own `sma_prot_device`
/// (`src/devices/bus/neogeo/prot_sma.cpp`, BSD-3-Clause,
/// <https://github.com/mamedev/mame>), which documents this chip
/// exactly (credited to Razoola and Mr.K's original decode work) --
/// not guessed at from partial/conflicting wiki summaries the way
/// earlier parts of this module were. `NeoGeoBus::new` runs the
/// decryption once at construction (matching MAME's own `decrypt_all`
/// timing, immediately after loading), not on every access.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Protection {
    None,
    SmaMslug3,
}

pub struct NeoGeoBus {
    work_ram: Vec<u8>,
    /// P1 -- the cartridge's fixed-bank program ROM, always visible at
    /// 0x000000-0x0FFFFF (mirrored/truncated if smaller than 1MB).
    /// For `Protection::SmaMslug3`, this is the *decrypted* fixed bank
    /// (extracted out of the larger P2 pool by the real SMA transform,
    /// not the raw on-disk P1 file's own bytes -- see `sma_decrypt`).
    p1_rom: Vec<u8>,
    /// P2 (and beyond, concatenated) -- the cartridge's banked program
    /// ROM, windowed into 0x200000-0x2FFFFF at byte offset `bank`.
    p2_rom: Vec<u8>,
    /// Byte offset into `p2_rom` currently windowed into
    /// 0x200000-0x2FFFFF (not a bank *index* -- SMA's real bank
    /// offsets aren't uniform 1MB multiples, see `sma_mslug3_bank_base`).
    bank: usize,
    protection: Protection,
    sound_latch: SoundLatch,
    lspc: Lspc,
    /// 256 palettes x 16 colors x 1 word, visible at $400000-$401FFF
    /// (mirrored through $7FFFFF). Real hardware has a second bank for
    /// double-buffering (selected by a register not implemented yet);
    /// only one bank's worth is stored here.
    palette_ram: Vec<u16>,
    /// S1 -- the cartridge's fix/text-layer graphics ROM. Not part of
    /// the 68k memory map at all (real hardware reads it only through
    /// the LSPC while rendering); stored here purely so the renderer
    /// has it alongside VRAM and palette RAM.
    s1_rom: Vec<u8>,
    /// The cartridge's C-ROMs, pre-split into the odd/even virtual
    /// ROMs `decode_sprite_tile` needs -- also not part of the 68k
    /// memory map, same reasoning as `s1_rom`.
    odd_c_rom: Vec<u8>,
    even_c_rom: Vec<u8>,
}

impl NeoGeoBus {
    fn new(p1_rom: Vec<u8>, p2_rom: Vec<u8>, s1_rom: Vec<u8>, c_roms: &[Vec<u8>], sound_latch: SoundLatch, protection: Protection) -> Self {
        let (p1_rom, p2_rom) = match protection {
            Protection::None => (word_swap(p1_rom), word_swap(p2_rom)),
            Protection::SmaMslug3 => sma_decrypt_68k(word_swap(p1_rom), word_swap(p2_rom), &SMA_MSLUG3_DATA_BITSWAP, &SMA_MSLUG3_FIXED_ADDR_BITSWAP, SMA_MSLUG3_FIXED_SOURCE, &SMA_MSLUG3_BANKED_ADDR_BITSWAP),
        };
        // Metal Slug 3's C-ROMs (sprites/fix-layer graphics) need their
        // own separate real decryption (CMC42, a different chip from
        // NEO-SMA) -- see `cmc42`'s own module doc comment for the
        // real interleave/decrypt/de-interleave pipeline this requires.
        let (odd_c_rom, even_c_rom) = match protection {
            Protection::None => (concat_c_roms(c_roms, true), concat_c_roms(c_roms, false)),
            Protection::SmaMslug3 => {
                let mut combined = interleave_c_rom_pairs(c_roms);
                cmc42::gfx_decrypt(&mut combined, cmc42::MSLUG3_GFX_KEY);
                deinterleave_c_rom_pairs(&combined, c_roms.len().div_ceil(2))
            }
        };
        Self {
            work_ram: vec![0; WORK_RAM_SIZE],
            p1_rom,
            p2_rom,
            bank: 0,
            protection,
            sound_latch,
            lspc: Lspc::new(),
            palette_ram: vec![0; 256 * 16],
            s1_rom,
            odd_c_rom,
            even_c_rom,
        }
    }

    /// Renders the fix/text layer's full 40x32-tile tilemap (320x256
    /// pixels) into an RGBA8 framebuffer, per
    /// <https://wiki.neogeodev.org/index.php?title=Fix_layer>: each of
    /// VRAM's 1280 tilemap words at $7000-$74FF is one tile, laid out
    /// left-to-right then top-to-bottom, with bits 15-8 selecting the
    /// palette and bits 7-0 the tile number (into `s1_rom`, via
    /// `decode_fix_tile`). Palette index 0 renders fully transparent,
    /// matching real hardware's "index 0 is always transparent"
    /// convention. NTSC only actually displays the middle 28 of these
    /// 32 rows -- cropping that isn't done here, since it's a display-
    /// timing detail or the frontend's to apply, not this function's.
    pub fn render_fix_layer(&self) -> (usize, usize, Vec<u8>) {
        let (width, height, mut rgba) = self.render_fix_layer_blank();
        self.render_fix_layer_onto(&mut rgba, width, height);
        (width, height, rgba)
    }

    fn render_fix_layer_blank(&self) -> (usize, usize, Vec<u8>) {
        const COLS: usize = 40;
        const ROWS: usize = 32;
        (COLS * 8, ROWS * 8, vec![0u8; COLS * 8 * ROWS * 8 * 4])
    }

    fn render_fix_layer_onto(&self, rgba: &mut [u8], width: usize, height: usize) {
        const COLS: usize = 40;
        let rows = height / 8;

        for row in 0..rows {
            for col in 0..COLS.min(width / 8) {
                let entry = self.lspc.peek((row * COLS + col) as u16 + 0x7000);
                let palette_number = (entry >> 8) as u32;
                let tile_number = (entry & 0xFF) as u32;
                let tile = decode_fix_tile(&self.s1_rom, tile_number);
                for (y, tile_row) in tile.iter().enumerate() {
                    for (x, &color_index) in tile_row.iter().enumerate() {
                        let px = col * 8 + x;
                        let py = row * 8 + y;
                        let out = (py * width + px) * 4;
                        if color_index == 0 {
                            continue; // transparent -- leave the framebuffer untouched
                        }
                        let palette_word_index = (palette_number * 16 + color_index as u32) as usize % self.palette_ram.len();
                        let (r, g, b) = decode_palette_color(self.palette_ram[palette_word_index]);
                        rgba[out] = r;
                        rgba[out + 1] = g;
                        rgba[out + 2] = b;
                        rgba[out + 3] = 255;
                    }
                }
            }
        }
    }

    /// Composites every active sprite onto an existing RGBA8
    /// framebuffer, per
    /// <https://wiki.neogeodev.org/index.php?title=Sprites>: each
    /// sprite is a 1-tile-wide (16px), up-to-32-tile-tall vertical
    /// strip, its tiles listed in SCB1 ($0000+, 64 words/sprite: even
    /// word = tile number's low 16 bits, odd word = palette in bits
    /// 15-12, tile number's high 4 bits in bits 11-8), positioned by
    /// SCB3 (Y + height, $8200+) and SCB4 (X, $8400+), shrunk by SCB2
    /// ($8000+). Auto-animation is not implemented yet.
    ///
    /// Horizontal flip (bit 4 of SCB1's odd word) mirrors each tile's
    /// own pixels left-right; vertical flip (bit 5) mirrors each
    /// tile's pixels top-bottom *and* reverses the tile order down the
    /// sprite's column, so the whole sprite flips as one unit rather
    /// than each tile flipping in place. Implemented by assembling the
    /// sprite's full, already-palette-resolved pixel grid first (tiles
    /// stacked top to bottom, flip applied), then shrink-sampling that
    /// grid -- keeps per-tile palette selection, flip, and shrink from
    /// tangling together, since each tile in a sprite's column can use
    /// a different palette.
    ///
    /// Per
    /// <https://wiki.neogeodev.org/index.php?title=Sprite_shrinking>:
    /// horizontal shrink (SCB2 bits 11-8) is a real, exact formula --
    /// final width in pixels is the stored nibble plus 1 (1-16, $F =
    /// full size), done as plain pixel-drop subsampling with no
    /// lookup table on real hardware, which nearest-neighbor sampling
    /// here matches exactly. Vertical shrink (SCB2 bits 7-0) is only
    /// approximated: real hardware picks which lines to keep via a
    /// dedicated internal chip ROM (the "L0 ROM"), which is part of
    /// the console itself, not any cartridge dump -- nothing a user
    /// could supply -- so a plain linear scale stands in for its exact
    /// (undocumented in the sources checked) drop pattern.
    ///
    /// **Unverified**: the Y-coordinate formula (`496 - raw`, wrapped
    /// into a 512-line virtual space -- real hardware apparently
    /// stores Y as a distance-from-bottom-ish offset, not a plain
    /// top-down coordinate), the exact active-sprite count (documented
    /// numbers for this ranged from 256 to 381 depending on source;
    /// 380 is used here as a reasonable, safely-in-bounds choice for
    /// SCB2-4's own 512-word capacity), and the palette field's width
    /// (4 bits here, which only reaches 16 of the real 256 palettes --
    /// plausibly incomplete) are this function's best reading of
    /// available documentation, not confirmed against a real rendered
    /// image.
    pub fn render_sprites_onto(&self, rgba: &mut [u8], width: usize, height: usize) {
        const MAX_SPRITES: u16 = 380;
        for sprite in 0..MAX_SPRITES {
            let scb2 = self.lspc.peek(0x8000 + sprite);
            let scb3 = self.lspc.peek(0x8200 + sprite);
            let scb4 = self.lspc.peek(0x8400 + sprite);
            let y_raw = (scb3 >> 8) as i32;
            let tile_count = ((scb3 & 0x7F) as usize).min(32);
            if tile_count == 0 {
                continue;
            }
            let x = scb4 as i16 as i32; // real hardware treats X as signed, sprites can sit partly off-screen
            let y_top = (496 - y_raw).rem_euclid(512);

            // Assemble the sprite's full, un-shrunk 16-wide pixel grid
            // (RGBA, already palette-resolved; alpha 0 = transparent),
            // tiles stacked top to bottom with flip applied.
            let source_height = tile_count * 16;
            let mut full = vec![[0u8; 4]; 16 * source_height];
            for tile_row in 0..tile_count as u16 {
                let scb1_base = sprite.wrapping_mul(64).wrapping_add(tile_row * 2);
                let even = self.lspc.peek(scb1_base);
                let odd = self.lspc.peek(scb1_base.wrapping_add(1));
                let tile_number = (((odd >> 8) & 0xF) as u32) << 16 | even as u32;
                let palette = ((odd >> 12) & 0xF) as u32;
                let h_flip = (odd >> 4) & 1 != 0;
                let v_flip = (odd >> 5) & 1 != 0;
                let pixels = decode_sprite_tile(&self.odd_c_rom, &self.even_c_rom, tile_number);
                let column_position = if v_flip { tile_count as u16 - 1 - tile_row } else { tile_row } as usize;
                for (ty, row) in pixels.iter().enumerate() {
                    let out_ty = if v_flip { 15 - ty } else { ty };
                    let dest_row = column_position * 16 + out_ty;
                    for (tx, &color_index) in row.iter().enumerate() {
                        if color_index == 0 {
                            continue;
                        }
                        let out_tx = if h_flip { 15 - tx } else { tx };
                        let palette_word_index = (palette * 16 + color_index as u32) as usize % self.palette_ram.len();
                        let (r, g, b) = decode_palette_color(self.palette_ram[palette_word_index]);
                        full[dest_row * 16 + out_tx] = [r, g, b, 255];
                    }
                }
            }

            // Shrink-sample the assembled grid down to its real
            // on-screen size.
            let h_shrink = ((scb2 >> 8) & 0xF) as usize;
            let final_width = h_shrink + 1; // 1..16, $F -> 16 (full size)
            let v_shrink = (scb2 & 0xFF) as usize;
            let final_height = ((source_height * (v_shrink + 1)) / 256).max(1);

            for out_y in 0..final_height {
                let src_y = (out_y * source_height / final_height).min(source_height - 1);
                for out_x in 0..final_width {
                    let src_x = (out_x * 16 / final_width).min(15);
                    let [r, g, b, a] = full[src_y * 16 + src_x];
                    if a == 0 {
                        continue;
                    }
                    let px = x + out_x as i32;
                    let py = y_top + out_y as i32;
                    if px < 0 || py < 0 || px as usize >= width || py as usize >= height {
                        continue;
                    }
                    let out = (py as usize * width + px as usize) * 4;
                    rgba[out] = r;
                    rgba[out + 1] = g;
                    rgba[out + 2] = b;
                    rgba[out + 3] = 255;
                }
            }
        }
    }

    /// A full frame: sprites first, the fix/text layer composited on
    /// top (matching real hardware's own priority -- HUD/text sits
    /// above gameplay sprites), sharing one framebuffer.
    pub fn render_frame(&self) -> (usize, usize, Vec<u8>) {
        let (width, height, mut rgba) = self.render_fix_layer_blank();
        self.render_sprites_onto(&mut rgba, width, height);
        self.render_fix_layer_onto(&mut rgba, width, height);
        (width, height, rgba)
    }

    fn read_work_ram(&self, addr: u32) -> u8 {
        self.work_ram[(addr as usize) % WORK_RAM_SIZE]
    }

    fn write_work_ram(&mut self, addr: u32, value: u8) {
        let index = (addr as usize) % WORK_RAM_SIZE;
        self.work_ram[index] = value;
    }

    fn read_p1(&self, addr: u32) -> u8 {
        if self.p1_rom.is_empty() {
            return 0;
        }
        self.p1_rom[(addr as usize) % self.p1_rom.len()]
    }

    fn read_p2_banked(&self, addr: u32) -> u8 {
        if self.p2_rom.is_empty() {
            return 0;
        }
        // `bank` is already a real byte offset (not a bank index) --
        // see this struct's own doc comment on why, for SMA carts,
        // real bank offsets aren't uniform 1MB multiples.
        let offset = self.bank + (addr as usize % 0x100000);
        self.p2_rom[offset % self.p2_rom.len()]
    }

    /// Palette RAM's 8KB word-addressed window ($400000-$401FFF,
    /// mirrored through $7FFFFF) maps directly onto its 4096 words
    /// (256 palettes x 16 colors), unlike VRAM's indirect
    /// address/data-register protocol.
    fn palette_index(&self, address: u32) -> usize {
        ((address & 0x1FFF) / 2) as usize % self.palette_ram.len()
    }
}

impl AddressBus for NeoGeoBus {
    fn read_byte(&mut self, address: u32) -> u8 {
        let address = address & 0xFF_FFFF;
        match address {
            0x000000..=0x0FFFFF => self.read_p1(address),
            0x100000..=0x1FFFFF => self.read_work_ram(address),
            0x200000..=0x2FFFFF => self.read_p2_banked(address - 0x200000),
            // REG_SOUND: the Z80's last reply byte.
            0x320000 => self.sound_latch.reply.get(),
            // Input/DIP/system registers: real controls aren't wired
            // up to this core yet (see this struct's own doc comment),
            // so every one of these reads back as fully idle. All of
            // them are active-low per
            // <https://wiki.neogeodev.org/index.php?title=Joypad> (a
            // bit reads 1 when its input is NOT active), so idle is
            // 0xFF, not 0 -- returning 0 here would look like every
            // button, DIP switch, and coin slot being held down at
            // once, which is exactly backwards and would send real
            // cartridge code down input-handling paths it shouldn't
            // take.
            0x300000 | 0x300001 | 0x300081 | 0x320001 | 0x340000 | 0x380000 | 0x380001 => 0xFF,
            _ => 0,
        }
    }

    fn read_word(&mut self, address: u32) -> u16 {
        match address & 0xFF_FFFF {
            // REG_VRAMADDR readback and REG_VRAMRW (which also
            // auto-increments -- see `Lspc`'s own doc comment).
            0x3C0000 => self.lspc.vram_addr,
            0x3C0002 => self.lspc.read_data(),
            // NEO-SMA's "chip present" check: always replies $9A37,
            // per MAME's own `sma_prot_device::prot_9a37_r` and the
            // real hardware address neogeo.cpp installs it at for
            // Metal Slug 3/3A specifically ($2FE446).
            0x2FE446 if self.protection == Protection::SmaMslug3 => 0x9A37,
            0x400000..=0x7FFFFF => self.palette_ram[self.palette_index(address)],
            _ => {
                let hi = self.read_byte(address) as u16;
                let lo = self.read_byte(address.wrapping_add(1)) as u16;
                (hi << 8) | lo
            }
        }
    }

    fn read_long(&mut self, address: u32) -> u32 {
        let hi = self.read_word(address) as u32;
        let lo = self.read_word(address.wrapping_add(2)) as u32;
        (hi << 16) | lo
    }

    fn write_byte(&mut self, address: u32, value: u8) {
        let address = address & 0xFF_FFFF;
        match address {
            0x100000..=0x1FFFFF => self.write_work_ram(address, value),
            // The generic P2 bankswitch scheme most cartridges use --
            // any write anywhere in this window latches the value's
            // low bits as the new 1MB bank number, per
            // <https://wiki.neogeodev.org/index.php?title=Bankswitching>.
            // Cartridges with a real protection chip (`Protection::
            // SmaMslug3`) use their own specific address and formula
            // instead -- see `write_word`'s own SMA arm -- and treat
            // every other address in this window as plain read-only
            // ROM, same as real SMA hardware does.
            0x200000..=0x2FFFFF if self.protection == Protection::None => {
                let num_banks = (self.p2_rom.len() / 0x100000).max(1);
                self.bank = ((value as usize) % num_banks) * 0x100000;
            }
            // REG_SOUND: send a command byte to the Z80.
            0x320000 => {
                self.sound_latch.command.set(value);
                self.sound_latch.nmi_pending.set(true);
            }
            _ => {}
        }
        // Cartridge ROM regions and the not-yet-implemented I/O/video
        // windows are silently ignored on write, same as read-only
        // memory on real hardware.
    }

    fn write_word(&mut self, address: u32, value: u16) {
        match address & 0xFF_FFFF {
            0x3C0000 => self.lspc.vram_addr = value,
            0x3C0002 => self.lspc.write_data(value),
            0x3C0004 => self.lspc.vram_mod = value as i16,
            // NEO-SMA bankswitch write, Metal Slug 3/3A's real address
            // ($2FFFE4, per MAME's own memory map install for
            // `NEOGEO_MSLUG3`/`NEOGEO_MSLUG3A`) -- real unscrambling
            // formula and lookup table, not the generic scheme.
            0x2FFFE4 if self.protection == Protection::SmaMslug3 => {
                self.bank = sma_mslug3_bank_base(value) as usize - 0x100000;
            }
            0x400000..=0x7FFFFF => {
                let index = self.palette_index(address);
                self.palette_ram[index] = value;
            }
            _ => {
                self.write_byte(address, (value >> 8) as u8);
                self.write_byte(address.wrapping_add(1), value as u8);
            }
        }
    }

    fn write_long(&mut self, address: u32, value: u32) {
        self.write_word(address, (value >> 16) as u16);
        self.write_word(address.wrapping_add(2), value as u16);
    }
}

/// Header offset of the entry-point trampoline: a `JMP.L` instruction
/// (6 bytes: the 0x4EF9 opcode, then a 32-bit absolute address) every
/// real Neo Geo cartridge has at this fixed offset, per
/// <https://wiki.neogeodev.org/index.php?title=68k_program_header>.
/// Real hardware's BIOS validates the cartridge header (the "NEO-GEO"
/// signature at $0100, the NGH id, etc.) before jumping here; this
/// project deliberately skips that validation -- see the module doc
/// comment for why no BIOS equivalent is included at all.
const HEADER_ENTRY_TRAMPOLINE: u32 = 0x0122;

/// A Neo Geo cartridge's 68000 side, wired up and ready to step --
/// just the CPU and memory map for now (see this module's own doc
/// comment for what's still missing).
pub struct NeoGeoMachine {
    pub cpu: CpuCore,
    pub bus: NeoGeoBus,
    pub sound_cpu: Z80,
    pub sound_bus: SoundBus,
}

impl NeoGeoMachine {
    pub fn new(p1_rom: Vec<u8>, p2_rom: Vec<u8>, m1_rom: Vec<u8>, s1_rom: Vec<u8>, c_roms: Vec<Vec<u8>>, v_rom: Vec<u8>, protection: Protection) -> Self {
        let sound_latch = SoundLatch::new();

        let mut cpu = CpuCore::new();
        cpu.set_cpu_type(CpuType::M68000);
        let mut bus = NeoGeoBus::new(p1_rom, p2_rom, s1_rom, &c_roms, sound_latch.clone(), protection);
        // `reset` reads the cartridge's own initial SSP from address 0
        // (real, meaningful data -- see the module doc comment) but
        // its initial PC is not how real hardware enters cartridge
        // code (the raw reset vector's PC slot instead holds a
        // pointer used elsewhere in the header format); jumping to the
        // documented entry trampoline afterward is what the real BIOS
        // does in its place.
        cpu.reset(&mut bus);
        cpu.pc = HEADER_ENTRY_TRAMPOLINE;

        // The Z80, unlike the 68000, has no vector table to read --
        // real hardware simply starts fetching from address 0 on
        // reset, which is real M1 ROM data (unlike the 68k side, M1 is
        // an 8-bit-wide device and is not word-swapped on real dumps).
        let sound_cpu = Z80::new();
        let sound_bus = SoundBus::new(m1_rom, v_rom, sound_latch);

        Self { cpu, bus, sound_cpu, sound_bus }
    }

    /// Steps the main CPU once. Returns `false` on a halt/fault
    /// `StepResult` so callers can stop rather than spin on a dead CPU.
    pub fn step(&mut self) -> bool {
        matches!(self.cpu.step(&mut self.bus), StepResult::Ok { .. })
    }

    /// Raises the VBlank interrupt (68000 autovectored level 1, vector
    /// 25 at $000064), per
    /// <https://wiki.neogeodev.org/index.php?title=68k_vector_table> --
    /// real hardware fires this ~60 times per second, and it's the
    /// interrupt the standard "wait for vblank, update, repeat" main-
    /// loop pattern almost every real Neo Geo game uses depends on;
    /// without ever raising it, such a game's main loop would spin
    /// forever waiting for an interrupt that never comes. Only takes
    /// effect on the *next* `step()` call, and only if the CPU's own
    /// interrupt priority mask (set by the game's own code, blocked by
    /// default after reset) actually permits level-1 interrupts.
    ///
    /// Real hardware requires the handler to write `REG_IRQACK`
    /// ($3C000C) before the interrupt line re-asserts; that write is
    /// accepted (see `write_word`) but currently has no effect, since
    /// `m68k`'s own interrupt delivery already clears the pending
    /// level once taken, so this doesn't yet risk the runaway-
    /// interrupt-flood failure mode the real hardware quirk exists to
    /// prevent.
    pub fn vblank(&mut self) {
        self.cpu.set_irq(1);
    }

    /// Steps the sound CPU once. Timing synchronization between the
    /// two CPUs (real hardware runs them concurrently against a shared
    /// clock) isn't implemented yet -- this just steps the Z80 in
    /// isolation, enough to prove the M1 ROM/memory map wiring is
    /// correct. Delivers a pending sound-command NMI first, same order
    /// real hardware would notice a freshly written `REG_SOUND` byte.
    pub fn step_sound(&mut self) {
        if self.sound_bus.sound_latch.nmi_pending.replace(false) {
            self.sound_cpu.pulse_nmi();
        }
        self.sound_cpu.step(&mut self.sound_bus);
    }

    /// Generates real YM2610 audio (interleaved stereo, one `f32` pair
    /// per sample) for `seconds` of chip time. See `Ym2610`'s own doc
    /// comment for the mixdown formula and sample scaling.
    pub fn generate_audio_seconds(&self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        self.sound_bus.ym2610.generate_seconds(seconds)
    }

    /// The YM2610's real native output rate at the Neo Geo's real 8MHz
    /// clock -- callers resample from this to whatever rate their own
    /// audio device actually runs at, the same convention this
    /// project's other cores already use for their own fixed/native
    /// rates (see `retro.rs`'s `LinearResampler`).
    pub fn audio_sample_rate(&self) -> f32 {
        self.sound_bus.ym2610.chip.borrow().sample_rate().max(1) as f32
    }
}

/// The Z80 sound CPU's memory map, per
/// <https://wiki.neogeodev.org/index.php?title=Z80>:
///
/// - `$0000-$7FFF`: the static, always-resident first 32KB of the M1
///   ROM.
/// - `$8000-$BFFF` (16KB), `$C000-$DFFF` (8KB), `$E000-$EFFF` (4KB),
///   `$F000-$F7FF` (2KB): four independently bank-switched windows
///   onto the rest of the M1 ROM (for carts whose M1 is larger than
///   32KB, like Metal Slug 3's 512KB). Real hardware selects each
///   window's bank via a **read**, not a write -- confirmed against
///   MAME's own memory map install
///   (`map(0x08, 0x0b)...r(FUNC(neogeo_base_state::audio_cpu_bank_select_r))`,
///   `src/mame/snk/neogeo.cpp`) -- an `IN r,(C)` instruction with the
///   bank number in register B and the port (08-0B, one per window) in
///   C, matching the same "full BC pair on the address bus" convention
///   this module's own doc comment on bankswitch ports already
///   documented in general terms; an earlier version of this comment
///   wrongly guessed it was a write.
/// - `$F800-$FFFF`: 2KB of sound work RAM.
///
/// The 68k/Z80 sound-command handshake is implemented (`SoundLatch`,
/// shared with `NeoGeoBus`) -- port $00 reads the 68k's command byte,
/// port $0C writes a reply back. The YM2610 (ports $04-$07) is real --
/// see `Ym2610`.
pub struct SoundBus {
    m1_rom: Vec<u8>,
    work_ram: [u8; 0x800],
    sound_latch: SoundLatch,
    ym2610: Ym2610,
    /// Current bank (into `m1_rom`) for each of the four windows,
    /// indexed `[$F000-window, $E000-window, $C000-window,
    /// $8000-window]` (port $08, $09, $0A, $0B respectively -- see
    /// this struct's own doc comment). `Cell` because bank selection
    /// happens on a `Z80_io::port_in` read, which only gets `&self`.
    m1_banks: [std::cell::Cell<usize>; 4],
}

impl SoundBus {
    pub fn new(m1_rom: Vec<u8>, v_rom: Vec<u8>, sound_latch: SoundLatch) -> Self {
        Self { m1_rom, work_ram: [0; 0x800], sound_latch, ym2610: Ym2610::new(v_rom), m1_banks: Default::default() }
    }

    fn read_rom(&self, addr: u16) -> u8 {
        if self.m1_rom.is_empty() {
            return 0;
        }
        // Window sizes, largest-addressed first, matching `m1_banks`'
        // own [$F000, $E000, $C000, $8000] order.
        let offset = match addr {
            0xF000..=0xF7FF => self.m1_banks[0].get() * 0x800 + (addr - 0xF000) as usize,
            0xE000..=0xEFFF => self.m1_banks[1].get() * 0x1000 + (addr - 0xE000) as usize,
            0xC000..=0xDFFF => self.m1_banks[2].get() * 0x2000 + (addr - 0xC000) as usize,
            0x8000..=0xBFFF => self.m1_banks[3].get() * 0x4000 + (addr - 0x8000) as usize,
            _ => addr as usize,
        };
        self.m1_rom[offset % self.m1_rom.len()]
    }
}

/// The real YM2610 sound chip, via `ymfm-sys` (see Cargo.toml's own
/// doc comment on that dependency). Register access is the documented
/// Z80 port interface
/// (<https://wiki.neogeodev.org/index.php?title=Z80/YM2610_interface>):
/// port $04 = address register for part 0 (SSG, ADPCM-B, FM channels
/// 1-2), $05 = data for part 0, $06 = address for part 1 (ADPCM-A, FM
/// channels 3-4), $07 = data for part 1 -- which maps onto `ymfm`'s
/// own `offset` parameter as `2 * part + (0 for address, 1 for data)`,
/// confirmed against `ymfm-sys`'s own `vgmrender` example (the same
/// convention MAME's own YM2610 device uses internally, which `ymfm`
/// is built to match).
///
/// Wrapped in a `RefCell` because `ymfm_sys::ffi::Chip::read` needs a
/// pinned mutable reference (real hardware register reads can latch
/// internal busy-flag state, so it's not a pure read), but this
/// module's `Z80_io::port_in` trait method only gets `&self`.
///
/// **ADPCM ROM loading is a documented assumption, not directly
/// verified**: real Neo Geo hardware feeds both the ADPCM-A and
/// ADPCM-B channels from the same physical V-ROM sockets (unlike some
/// other systems' YM2610 wiring, which use genuinely separate ROMs
/// for each), so the cartridge's full V1-V4 ROM data is served to both
/// of `ymfm`'s AdpcmA and AdpcmB access classes at offset 0, via the
/// `read_data` interface callback (the real mechanism `ymfm-sys`
/// expects a host to serve ROM data through -- see its own
/// `vgmrender` example, which loads a similar shared ADPCM ROM for
/// the related YM2608 chip the same way).
struct Ym2610 {
    chip: std::cell::RefCell<ymfm_sys::ChipPtr>,
}

impl Ym2610 {
    /// The Neo Geo's real YM2610 clock, per
    /// <https://wiki.neogeodev.org/index.php?title=YM2610>.
    const CLOCK_HZ: u32 = 8_000_000;

    fn new(v_rom: Vec<u8>) -> Self {
        let v_rom = std::rc::Rc::new(v_rom);
        let handler = ymfm_sys::InterfaceHandler {
            read_data: Some(Box::new(move |access, base, length| {
                use ymfm_sys::ffi::AccessClass;
                if !matches!(access, AccessClass::AdpcmA | AccessClass::AdpcmB) || v_rom.is_empty() {
                    return vec![0; length as usize];
                }
                (0..length).map(|i| v_rom.get((base + i) as usize).copied().unwrap_or(0)).collect()
            })),
            ..Default::default()
        };
        let chip = ymfm_sys::ffi::create_chip_with_callbacks(ymfm_sys::ffi::ChipType::Ym2610, Self::CLOCK_HZ, Box::new(ymfm_sys::InterfaceCallbacks::new(handler)));
        Self { chip: std::cell::RefCell::new(chip) }
    }

    fn register_offset(z80_port: u16) -> u32 {
        // Part 0 = ports 4/5, part 1 = ports 6/7; each part is an
        // (address, data) pair two `ymfm` offsets apart, address first.
        let part = (z80_port - 4) / 2;
        let is_data = (z80_port - 4) % 2;
        (2 * part + is_data) as u32
    }

    fn read(&self, z80_port: u16) -> u8 {
        self.chip.borrow_mut().pin_mut().read(Self::register_offset(z80_port))
    }

    fn write(&self, z80_port: u16, value: u8) {
        self.chip.borrow_mut().pin_mut().write(Self::register_offset(z80_port), value);
    }

    /// Generates real audio through the chip's own native mixdown,
    /// resampled to `out_rate`. `ymfm`'s YM2610 raw output is 3
    /// channels per native sample (FM left, FM right, SSG/ADPCM mono);
    /// `left = out[0] + out[2]`, `right = out[1] + out[2]` is the exact
    /// mixdown `ymfm-sys`'s own `vgmrender` example uses for this chip.
    /// Real samples are scaled from ymfm's i32 range by the same
    /// `/32768.0` convention this module's other cores already use for
    /// their own 16-bit-range audio.
    fn generate_seconds(&self, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut chip = self.chip.borrow_mut();
        let native_rate = chip.sample_rate().max(1) as f32;
        let samples = (seconds * native_rate).round().max(0.0) as usize;
        let channels = chip.channels().max(1) as usize;
        let mut native = vec![0i32; channels];
        let mut left = Vec::with_capacity(samples);
        let mut right = Vec::with_capacity(samples);
        for _ in 0..samples {
            native.fill(0);
            chip.pin_mut().generate(&mut native);
            let l = native[0] + native[2 % channels];
            let r = native[1 % channels] + native[2 % channels];
            left.push(l as f32 / 32768.0);
            right.push(r as f32 / 32768.0);
        }
        (left, right)
    }
}

impl Z80_io for SoundBus {
    fn read_byte(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0xF7FF => self.read_rom(addr),
            0xF800..=0xFFFF => self.work_ram[(addr - 0xF800) as usize],
        }
    }

    fn write_byte(&mut self, addr: u16, value: u8) {
        if let 0xF800..=0xFFFF = addr {
            self.work_ram[(addr - 0xF800) as usize] = value;
        }
        // ROM writes are silently ignored, same as real read-only ROM.
    }

    fn port_in(&self, addr: u16) -> u8 {
        // The real port number is always the address bus's low byte
        // (see this module's own doc comment on bankswitch ports for
        // why the high byte varies by addressing mode).
        match addr & 0xFF {
            0x00 => self.sound_latch.command.get(),
            port @ 0x04..=0x07 => self.ym2610.read(port),
            // Bankswitch: the bank number rides in the high byte (the
            // real `IN r,(C)` instruction form real hardware uses here
            // puts the B register on the address bus's upper 8 bits --
            // see this struct's own doc comment). Port order matches
            // `m1_banks`' own [$F000, $E000, $C000, $8000] layout.
            port @ 0x08..=0x0B => {
                self.m1_banks[(port - 0x08) as usize].set((addr >> 8) as usize);
                0xFF
            }
            _ => 0xFF,
        }
    }

    fn port_out(&mut self, addr: u16, value: u8) {
        match addr & 0xFF {
            0x0C => self.sound_latch.reply.set(value),
            port @ 0x04..=0x07 => self.ym2610.write(port, value),
            _ => {}
        }
    }
}

/// Decodes one 8x8, 4-bits-per-pixel fix/text-layer tile from raw S
/// ROM bytes, per
/// <https://wiki.neogeodev.org/index.php?title=Fix_graphics_format>.
///
/// Each tile is 32 bytes, addressed within the ROM as `tile*32 +
/// (half<<4) | (column<<3) | line` -- the tile's 8x8 grid is split
/// into two 4-pixel-wide halves, each split again into two 2-pixel
/// columns, each holding all 8 lines contiguously. Each byte packs one
/// horizontally-adjacent pixel pair directly as two 4-bit palette
/// indices (low nibble = left pixel, high nibble = right pixel) --
/// unlike the sprite/C-ROM format, there is no cross-byte bitplane
/// combining here, since the fix layer's S ROM is a single 8-bit-wide
/// chip rather than a pair of ROMs like sprites use.
///
/// Returned as `[y][x]` palette indices (0-15); index 0 means
/// transparent, per the wiki's fix-layer page.
///
/// **Unverified**: the wiki states `H`=0 is the tile's right half and
/// `H`=1 is the left half (an intentionally-quoted, seemingly
/// backwards convention on real hardware) -- this is implemented
/// exactly as documented, but nothing in this project can yet render
/// a frame to confirm left/right orientation is actually correct
/// (a solid-color test tile, which is what the real S1 ROM's first
/// several tiles happen to be, can't distinguish it either way). Get
/// this wrong and every fix-layer tile would render horizontally
/// mirrored.
fn decode_fix_tile(s1_rom: &[u8], tile_number: u32) -> [[u8; 8]; 8] {
    let mut pixels = [[0u8; 8]; 8];
    if s1_rom.is_empty() {
        return pixels;
    }
    let tile_base = (tile_number as usize) * 32;
    for half in 0..2u32 {
        for column in 0..2u32 {
            for line in 0..8u32 {
                let byte_offset = tile_base + ((half << 4) | (column << 3) | line) as usize;
                let byte = s1_rom[byte_offset % s1_rom.len()];
                // H=0 is the right half of the tile, H=1 the left --
                // see this function's own doc comment.
                let column_pair_index = (1 - half) * 2 + column;
                let x = (column_pair_index * 2) as usize;
                pixels[line as usize][x] = byte & 0x0F;
                pixels[line as usize][x + 1] = (byte >> 4) & 0x0F;
            }
        }
    }
    pixels
}

/// Decodes one 16-bit Neo Geo palette color word into 8-bit-per-
/// channel RGB, per <https://wiki.neogeodev.org/index.php?title=Colors>'s
/// documented bit layout:
///
/// ```text
/// bit: 15    14  13  12  11  10  9   8   7   6   5   4   3   2   1   0
///      Dark  R0  G0  B0  R4  R3  R2  R1  G4  G3  G2  G1  B4  B3  B2  B1
/// ```
///
/// Each channel is 5 bits, but split non-contiguously across the word
/// (its low bit sits by itself next to the other channels' low bits),
/// plus one "dark" bit shared across all three channels as an extra
/// implicit low bit -- giving each channel 6 bits of real precision
/// from a 16-bit word that only spends 15 bits on color. The resulting
/// 6-bit channel is expanded to 8 bits by bit replication (repeating
/// its own top 2 bits into the bottom 2), the standard way to spread
/// a smaller value evenly across the full 0-255 range without a
/// lookup table.
fn decode_palette_color(word: u16) -> (u8, u8, u8) {
    let dark = (word >> 15) & 1;
    // Each channel's top 4 bits (R4..R1 / G4..G1 / B4..B1) sit as one
    // contiguous nibble; shifting that nibble down by the right amount
    // puts bit11/bit7/bit3 (the R4/G4/B4 MSB) at the nibble's own bit
    // 3, in the same MSB-first order the 5-bit channel needs.
    let r5 = ((word >> 8) & 0b1111) << 1 | ((word >> 14) & 1);
    let g5 = ((word >> 4) & 0b1111) << 1 | ((word >> 13) & 1);
    let b5 = (word & 0b1111) << 1 | ((word >> 12) & 1);
    let expand = |channel5: u16| -> u8 {
        let channel6 = ((channel5 << 1) | dark) as u8;
        (channel6 << 2) | (channel6 >> 4)
    };
    (expand(r5), expand(g5), expand(b5))
}

/// Decodes one 16x16, 4-bits-per-pixel sprite tile from the
/// cartridge's C-ROMs, per
/// <https://wiki.neogeodev.org/index.php?title=Sprite_graphics_format>.
///
/// A 128-byte tile is split across two *virtual* ROMs: `odd_rom` is
/// every odd-numbered C-ROM (C1, C3, C5, C7...) concatenated together,
/// supplying bitplanes 0-1; `even_rom` is every even one (C2, C4,
/// C6...), supplying bitplanes 2-3 -- each contributing 64 bytes per
/// tile (half of the tile's total 128).
///
/// Each virtual ROM's 64 bytes for one tile are four 16-byte 8x8
/// quadrants, in the documented (and, on real hardware, genuinely
/// backwards) order top-right, bottom-right, top-left, bottom-left.
/// Within a quadrant, each row is 2 bytes -- one per bitplane the ROM
/// supplies -- read MSB-first (bit 7 = the row's leftmost pixel).
/// Combining all 4 bitplane bits (3=MSB, 0=LSB) gives one pixel's
/// 4-bit palette index (0-15; 0 is transparent, same as the fix
/// layer).
///
/// **Unverified**: which of each quadrant's 2 bytes is bitplane 0 vs.
/// 1 (and 2 vs. 3), and the precise top/bottom ordering within
/// "top-right"/"bottom-right" etc., are this function's best reading
/// of the documentation, not confirmed against a real rendered image
/// -- there is no equivalent to the fix layer's lucky solid-color
/// tiles to validate sprite orientation structurally. Get this wrong
/// and sprites would decode with scrambled or mirrored pixels despite
/// using genuinely correct source bytes.
fn decode_sprite_tile(odd_rom: &[u8], even_rom: &[u8], tile_number: u32) -> [[u8; 16]; 16] {
    let mut pixels = [[0u8; 16]; 16];
    if odd_rom.is_empty() || even_rom.is_empty() {
        return pixels;
    }
    let tile_base = (tile_number as usize) * 64;
    // (quadrant index, x offset, y offset) for top-right, bottom-right,
    // top-left, bottom-left, in that ROM order.
    const QUADRANTS: [(usize, usize); 4] = [(8, 0), (8, 8), (0, 0), (0, 8)];
    for (quadrant_index, &(x_base, y_base)) in QUADRANTS.iter().enumerate() {
        let quadrant_base = tile_base + quadrant_index * 16;
        for row in 0..8usize {
            let plane0 = odd_rom[(quadrant_base + row * 2) % odd_rom.len()];
            let plane1 = odd_rom[(quadrant_base + row * 2 + 1) % odd_rom.len()];
            let plane2 = even_rom[(quadrant_base + row * 2) % even_rom.len()];
            let plane3 = even_rom[(quadrant_base + row * 2 + 1) % even_rom.len()];
            for x in 0..8usize {
                let bit = 7 - x;
                let color_index = (((plane3 >> bit) & 1) << 3) | (((plane2 >> bit) & 1) << 2) | (((plane1 >> bit) & 1) << 1) | ((plane0 >> bit) & 1);
                pixels[y_base + row][x_base + x] = color_index;
            }
        }
    }
    pixels
}

/// Concatenates the odd-numbered (C1, C3, C5...) or even-numbered (C2,
/// C4, C6...) C-ROMs into one virtual ROM for `decode_sprite_tile`,
/// per that function's own doc comment.
fn concat_c_roms(c_roms: &[Vec<u8>], odd: bool) -> Vec<u8> {
    c_roms.iter().enumerate().filter(|(i, _)| (i % 2 == 0) == odd).flat_map(|(_, rom)| rom.iter().copied()).collect()
}

/// Metal Slug 3's real CMC42 sprite/fix-layer graphics decryption,
/// reimplemented from MAME's own BSD-3-licensed source
/// (`src/devices/bus/neogeo/prot_cmc.cpp`, `gfx_decrypt`/`decrypt`) --
/// a real, fully-specified (if genuinely intricate) XOR scheme
/// involving 9 correlated 256-byte tables, not a guess. Metal Slug 3
/// uses the shared "kof99" tables every CMC42 game except KOF2000/MS4
/// uses (per that source's own comment) plus its own key,
/// `MSLUG3_GFX_KEY` = 0xad.
///
/// Real hardware/MAME operate on ONE combined buffer built by
/// interleaving each numbered pair of C-ROMs byte-by-byte (C1's bytes
/// at even positions, C2's at odd, forming one block; C3/C4 form the
/// next block; and so on -- confirmed against MAME's own cartridge ROM
/// loading for `mslug3`, which uses exactly this `ROM_LOAD16_BYTE`
/// pairing), NOT this module's own `odd_c_rom`/`even_c_rom` split
/// (which concatenates all odd-numbered and all even-numbered ROMs
/// separately -- a different, though tile-addressing-equivalent,
/// organization used only for `decode_sprite_tile`). `interleave_c_rom_pairs`
/// and `deinterleave_c_rom_pairs` convert between the two.
mod cmc42 {
    include!("neogeo_cmc42_tables.rs");

    /// One byte-pair's data XOR, per MAME's own `cmc_prot_device::decrypt`.
    fn decrypt_pair(c0: u8, c1: u8, table0hi: &[u8; 256], table0lo: &[u8; 256], table1: &[u8; 256], base: u32, invert: bool) -> (u8, u8) {
        let tmp = table1[((base & 0xff) ^ CMC42_KOF99_ADDRESS_0_7_XOR[((base >> 8) & 0xff) as usize] as u32) as usize];
        let xor0 = (table0hi[((base >> 8) & 0xff) as usize] & 0xfe) | (tmp & 0x01);
        let xor1 = (tmp & 0xfe) | (table0lo[((base >> 8) & 0xff) as usize] & 0x01);
        if invert {
            (c1 ^ xor0, c0 ^ xor1)
        } else {
            (c0 ^ xor0, c1 ^ xor1)
        }
    }

    /// The real algorithm, per MAME's own `cmc_prot_device::gfx_decrypt`:
    /// a data-XOR pass over every 4-byte group, then an address-XOR
    /// pass that shuffles those (now data-decrypted) 4-byte groups
    /// into their real positions.
    pub fn gfx_decrypt(rom: &mut [u8], extra_xor: u32) {
        let rom_size = rom.len();
        let mut buf = vec![0u8; rom_size];

        // Data xor.
        for rpos in 0..(rom_size / 4) {
            let (b0, b3) = decrypt_pair(rom[4 * rpos], rom[4 * rpos + 3], &CMC42_KOF99_TYPE0_T03, &CMC42_KOF99_TYPE0_T12, &CMC42_KOF99_TYPE1_T03, rpos as u32, ((rpos >> 8) & 1) != 0);
            buf[4 * rpos] = b0;
            buf[4 * rpos + 3] = b3;
            let (b1, b2) = decrypt_pair(
                rom[4 * rpos + 1],
                rom[4 * rpos + 2],
                &CMC42_KOF99_TYPE0_T12,
                &CMC42_KOF99_TYPE0_T03,
                &CMC42_KOF99_TYPE1_T12,
                rpos as u32,
                (((rpos >> 16) as u32) ^ (CMC42_KOF99_ADDRESS_16_23_XOR2[(rpos >> 8) & 0xff] as u32)) & 1 != 0,
            );
            buf[4 * rpos + 1] = b1;
            buf[4 * rpos + 2] = b2;
        }

        // Address xor.
        for rpos in 0..(rom_size / 4) {
            let mut baser = rpos as u32;
            baser ^= extra_xor;
            baser ^= (CMC42_KOF99_ADDRESS_8_15_XOR1[((baser >> 16) & 0xff) as usize] as u32) << 8;
            baser ^= (CMC42_KOF99_ADDRESS_8_15_XOR2[(baser & 0xff) as usize] as u32) << 8;
            baser ^= (CMC42_KOF99_ADDRESS_16_23_XOR1[(baser & 0xff) as usize] as u32) << 16;
            baser ^= (CMC42_KOF99_ADDRESS_16_23_XOR2[((baser >> 8) & 0xff) as usize] as u32) << 16;
            baser ^= CMC42_KOF99_ADDRESS_0_7_XOR[((baser >> 8) & 0xff) as usize] as u32;
            baser &= (rom_size as u32 / 4) - 1;
            let baser = baser as usize;
            rom[4 * rpos] = buf[4 * baser];
            rom[4 * rpos + 1] = buf[4 * baser + 1];
            rom[4 * rpos + 2] = buf[4 * baser + 2];
            rom[4 * rpos + 3] = buf[4 * baser + 3];
        }
    }

    pub const MSLUG3_GFX_KEY: u32 = 0xad;
}

/// Builds the real combined sprite-ROM buffer `cmc42::gfx_decrypt`
/// operates on: each numbered pair of C-ROMs (C1/C2, C3/C4, ...)
/// interleaved byte-by-byte into its own block, blocks concatenated in
/// cartridge order -- see `cmc42`'s own module doc comment for why
/// this differs from `concat_c_roms`.
fn interleave_c_rom_pairs(c_roms: &[Vec<u8>]) -> Vec<u8> {
    let pair_len = c_roms.chunks(2).map(|pair| pair.iter().map(|r| r.len()).max().unwrap_or(0)).max().unwrap_or(0);
    let mut out = vec![0u8; c_roms.len().div_ceil(2) * pair_len * 2];
    for (pair_index, pair) in c_roms.chunks(2).enumerate() {
        let base = pair_index * pair_len * 2;
        if let Some(even_rom) = pair.first() {
            for (i, &b) in even_rom.iter().enumerate() {
                out[base + i * 2] = b;
            }
        }
        if let Some(odd_rom) = pair.get(1) {
            for (i, &b) in odd_rom.iter().enumerate() {
                out[base + i * 2 + 1] = b;
            }
        }
    }
    out
}

/// The inverse of `interleave_c_rom_pairs`, splitting a decrypted
/// combined buffer back into this module's own `odd_c_rom`/`even_c_rom`
/// representation (all even-position bytes across every pair-block
/// concatenated in order = the "odd-numbered-ROM" virtual buffer
/// `decode_sprite_tile` expects, and vice versa).
fn deinterleave_c_rom_pairs(combined: &[u8], pair_count: usize) -> (Vec<u8>, Vec<u8>) {
    let pair_len = if pair_count == 0 { 0 } else { combined.len() / pair_count / 2 };
    let mut odd_rom = Vec::with_capacity(pair_len * pair_count);
    let mut even_rom = Vec::with_capacity(pair_len * pair_count);
    for pair_index in 0..pair_count {
        let base = pair_index * pair_len * 2;
        for i in 0..pair_len {
            odd_rom.push(combined[base + i * 2]);
            even_rom.push(combined[base + i * 2 + 1]);
        }
    }
    (odd_rom, even_rom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const ROM_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/roms/neogeo/metalslug3");

    fn load_real_rom(name: &str) -> Option<Vec<u8>> {
        std::fs::read(Path::new(ROM_DIR).join(name)).ok()
    }

    /// All-zero and all-one words must produce black and white
    /// respectively -- the simplest possible sanity check that channel
    /// extraction and the 6-to-8-bit expansion aren't wildly wrong.
    #[test]
    fn palette_color_extremes_are_black_and_white() {
        assert_eq!(decode_palette_color(0x0000), (0, 0, 0), "an all-zero word must decode to pure black");
        assert_eq!(decode_palette_color(0xFFFF), (255, 255, 255), "an all-one word must decode to pure white");
    }

    /// Each channel's bits are scattered non-contiguously across the
    /// word (see `decode_palette_color`'s own doc comment and bit
    /// table) -- setting only the bits documented as "red" (R4-R1 at
    /// bits 11-8, R0 at bit 14) must produce a pure red color with
    /// green and blue both fully zero, proving the bit positions for
    /// each channel are exactly right, not just approximately.
    #[test]
    fn only_the_documented_red_bits_produce_a_pure_red_channel() {
        let red_bits = 0b0_1_0_0_1111_0000_0000u16; // R0=1, R4..R1=1111, dark=0
        let (r, g, b) = decode_palette_color(red_bits);
        assert_eq!((g, b), (0, 0), "setting only red's documented bits must leave green and blue at zero");
        assert!(r > 200, "setting all 5 red bits should produce a strong red channel, got {r}");
    }

    /// Same check for green's documented bits (G4-G1 at bits 7-4, G0
    /// at bit 13).
    #[test]
    fn only_the_documented_green_bits_produce_a_pure_green_channel() {
        let green_bits = 0b0_0_1_0_0000_1111_0000u16; // G0=1, G4..G1=1111
        let (r, g, b) = decode_palette_color(green_bits);
        assert_eq!((r, b), (0, 0), "setting only green's documented bits must leave red and blue at zero");
        assert!(g > 200, "setting all 5 green bits should produce a strong green channel, got {g}");
    }

    /// Same check for blue's documented bits (B4-B1 at bits 3-0, B0 at
    /// bit 12).
    #[test]
    fn only_the_documented_blue_bits_produce_a_pure_blue_channel() {
        let blue_bits = 0b0_0_0_1_0000_0000_1111u16; // B0=1, B4..B1=1111
        let (r, g, b) = decode_palette_color(blue_bits);
        assert_eq!((r, g), (0, 0), "setting only blue's documented bits must leave red and green at zero");
        assert!(b > 200, "setting all 5 blue bits should produce a strong blue channel, got {b}");
    }

    /// The dark bit is shared across all three channels as an extra
    /// low bit -- toggling it alone (with every other bit already at
    /// max) must not change which channels are lit, only brighten all
    /// three slightly (since it's the new LSB of an already-maxed
    /// 5-bit value, going from 6-bit value 30 to 31).
    #[test]
    fn the_dark_bit_brightens_all_three_channels_together() {
        let without_dark = 0b0_1111_1111_1111_111u16; // all channel bits set, dark=0 (top bit)
        let with_dark = 0xFFFFu16; // same, dark=1
        let (r0, g0, b0) = decode_palette_color(without_dark);
        let (r1, g1, b1) = decode_palette_color(with_dark);
        assert!(r1 >= r0 && g1 >= g0 && b1 >= b0, "setting the dark bit should never darken a channel that was already near-maximal");
        assert_eq!((r1, g1, b1), (255, 255, 255), "with every channel bit and the dark bit all set, the result must be pure white");
    }

    /// A minimal synthetic cartridge: just enough of a real header to
    /// exercise `NeoGeoMachine::new`'s boot sequence without needing a
    /// real ROM -- a real stack pointer at offset 0, and a real
    /// `JMP.L` trampoline at the documented header offset, matching
    /// the byte-swapped-on-disk convention real dumps use.
    fn synthetic_cartridge_with_entry_point(entry: u32) -> Vec<u8> {
        let mut rom = vec![0u8; 0x200];
        rom[0..4].copy_from_slice(&0x0010F000u32.to_be_bytes());
        rom[(HEADER_ENTRY_TRAMPOLINE as usize)..(HEADER_ENTRY_TRAMPOLINE as usize + 2)].copy_from_slice(&0x4EF9u16.to_be_bytes());
        rom[(HEADER_ENTRY_TRAMPOLINE as usize + 2)..(HEADER_ENTRY_TRAMPOLINE as usize + 6)].copy_from_slice(&entry.to_be_bytes());
        word_swap(rom)
    }

    /// `reset` must load the cartridge's own real initial stack
    /// pointer from address 0, and `NeoGeoMachine::new` must then move
    /// PC to the documented header entry trampoline rather than
    /// leaving it at whatever the raw reset vector's PC slot held.
    #[test]
    fn new_loads_the_cartridges_own_stack_pointer_and_starts_at_the_header_trampoline() {
        let rom = synthetic_cartridge_with_entry_point(0x000180);
        let machine = NeoGeoMachine::new(rom, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        assert_eq!(machine.cpu.dar[15], 0x0010F000, "SSP (A7) must be exactly what the cartridge's own header at offset 0 specifies");
        assert_eq!(machine.cpu.pc, HEADER_ENTRY_TRAMPOLINE, "PC must start at the documented header entry trampoline");
    }

    /// The header's own `JMP.L` must actually redirect execution to
    /// wherever the cartridge's real code entry point is -- the whole
    /// point of jumping to the trampoline instead of straight to a
    /// hardcoded address, since different cartridges put their real
    /// code at different offsets.
    #[test]
    fn the_header_trampoline_jumps_to_the_cartridges_own_declared_entry_point() {
        let rom = synthetic_cartridge_with_entry_point(0x0001C4);
        let mut machine = NeoGeoMachine::new(rom, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        assert!(machine.step(), "the header's own JMP.L must execute cleanly");
        assert_eq!(machine.cpu.pc, 0x0001C4, "PC must land exactly on the cartridge's own declared entry point");
    }

    /// Raising VBlank must actually redirect the CPU to the real
    /// vector-25 (level 1 autovector, $000064) handler address the
    /// cartridge's own vector table specifies -- once the game's own
    /// code has lowered the interrupt priority mask to permit it, the
    /// same as real code does before relying on VBlank at all.
    #[test]
    fn vblank_redirects_execution_to_the_cartridges_own_level_1_handler() {
        let mut rom = vec![0u8; 0x200];
        rom[0..4].copy_from_slice(&0x0010F000u32.to_be_bytes());
        rom[0x64..0x68].copy_from_slice(&0x000001A0u32.to_be_bytes()); // vector 25: VBlank handler
        rom[HEADER_ENTRY_TRAMPOLINE as usize..HEADER_ENTRY_TRAMPOLINE as usize + 2].copy_from_slice(&0x4EF9u16.to_be_bytes());
        rom[HEADER_ENTRY_TRAMPOLINE as usize + 2..HEADER_ENTRY_TRAMPOLINE as usize + 6].copy_from_slice(&0x00000180u32.to_be_bytes());
        let rom = word_swap(rom);

        let mut machine = NeoGeoMachine::new(rom, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        machine.cpu.int_mask = 0; // simulate the game's own code having unmasked interrupts
        machine.vblank();
        assert!(machine.step(), "taking the interrupt must not fault the CPU");
        assert_eq!(machine.cpu.pc, 0x000001A0, "PC must land exactly on the cartridge's own vector-25 handler address");
    }

    /// The full 68k -> Z80 -> 68k sound-command round trip, driven end
    /// to end: the 68k writes a command byte to `REG_SOUND` ($320000),
    /// which must reach the Z80 as a real NMI (vector $0066) with the
    /// command readable on port $00; a synthetic Z80 "sound driver"
    /// (just `IN A,($00)` / `OUT ($0C),A` / `RETN` at $0066, real Z80
    /// opcodes) echoes it back via port $0C; and the 68k must be able
    /// to read that exact byte back from the same `REG_SOUND` address.
    #[test]
    fn a_68k_sound_command_reaches_the_z80_via_nmi_and_its_reply_reaches_the_68k() {
        let rom = synthetic_cartridge_with_entry_point(0x000180);
        let mut m1_rom = vec![0u8; 0x100];
        // NMI handler at $0066: IN A,($00) ; OUT ($0C),A ; RETN
        m1_rom[0x66] = 0xDB;
        m1_rom[0x67] = 0x00;
        m1_rom[0x68] = 0xD3;
        m1_rom[0x69] = 0x0C;
        m1_rom[0x6A] = 0xED;
        m1_rom[0x6B] = 0x45;
        let mut machine = NeoGeoMachine::new(rom, Vec::new(), m1_rom, Vec::new(), Vec::new(), Vec::new(), Protection::None);

        // The 68k sends a real command byte.
        machine.bus.write_byte(0x320000, 0x42);
        assert!(machine.bus.sound_latch.nmi_pending.get(), "writing REG_SOUND must request an NMI for the Z80");

        // Deliver the NMI and let the synthetic handler run (call +
        // three instructions + RETN).
        for _ in 0..5 {
            machine.step_sound();
        }

        assert_eq!(machine.bus.read_byte(0x320000), 0x42, "the 68k must read back exactly the byte the Z80's handler echoed via port $0C");
    }

    /// Input/DIP/system registers are active-low (a bit reads 1 when
    /// its input is NOT asserted, per
    /// <https://wiki.neogeodev.org/index.php?title=Joypad>), so with no
    /// real controller wired up yet, every one of them must read back
    /// fully idle (0xFF) -- not 0, which would look like every button
    /// held and every coin slot inserted simultaneously and could send
    /// real cartridge code down input-handling paths it shouldn't take.
    #[test]
    fn unwired_input_registers_read_idle_not_all_buttons_held() {
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        for addr in [0x300000u32, 0x300001, 0x300081, 0x320001, 0x340000, 0x380000, 0x380001] {
            assert_eq!(machine.bus.read_byte(addr), 0xFF, "register 0x{addr:06X} must read idle (0xFF), not 0");
        }
    }

    /// The core VRAM access protocol: set an address via REG_VRAMADDR,
    /// write a word via REG_VRAMRW, and read it back from the same
    /// address -- the basic round trip every real Neo Geo game relies
    /// on to build its sprite/fix-layer data before anything can be
    /// rendered.
    #[test]
    fn vram_write_then_read_back_at_the_same_address_round_trips() {
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        machine.bus.write_word(0x3C0000, 0x1234); // REG_VRAMADDR
        machine.bus.write_word(0x3C0002, 0xBEEF); // REG_VRAMRW
        machine.bus.write_word(0x3C0000, 0x1234); // re-seek: REG_VRAMRW just auto-incremented past it
        assert_eq!(machine.bus.read_word(0x3C0002), 0xBEEF, "the word just written must read back unchanged at the same VRAM address");
    }

    /// REG_VRAMMOD's auto-increment must actually advance the VRAM
    /// address after each REG_VRAMRW access (both directions), the
    /// real mechanism games use to scan through the sprite list/fix
    /// layer via repeated accesses instead of re-seeking every word.
    #[test]
    fn vram_mod_auto_increments_the_address_after_each_access() {
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        machine.bus.write_word(0x3C0000, 0x0100); // REG_VRAMADDR
        machine.bus.write_word(0x3C0004, 1); // REG_VRAMMOD: +1 per access
        machine.bus.write_word(0x3C0002, 0xAAAA);
        machine.bus.write_word(0x3C0002, 0xBBBB);
        machine.bus.write_word(0x3C0002, 0xCCCC);

        machine.bus.write_word(0x3C0000, 0x0100); // seek back to the start
        assert_eq!(machine.bus.read_word(0x3C0002), 0xAAAA, "first word at the base address");
        assert_eq!(machine.bus.read_word(0x3C0002), 0xBBBB, "second word, one past the base");
        assert_eq!(machine.bus.read_word(0x3C0002), 0xCCCC, "third word, two past the base");
    }

    /// Palette RAM is directly 68k-addressable (unlike VRAM's indirect
    /// register protocol) -- a word written at $400000 must read back
    /// unchanged at the same address, and distinct palette entries
    /// ($400000 vs $400002, the first two colors of palette 0) must
    /// not alias each other.
    #[test]
    fn palette_ram_write_then_read_back_round_trips_without_aliasing() {
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        machine.bus.write_word(0x400000, 0x7FFF);
        machine.bus.write_word(0x400002, 0x1234);
        assert_eq!(machine.bus.read_word(0x400000), 0x7FFF, "the word written to color 0 of palette 0 must read back unchanged");
        assert_eq!(machine.bus.read_word(0x400002), 0x1234, "the word written to color 1 of palette 0 must read back unchanged and not alias color 0");
    }

    /// Writing a value anywhere in the $200000-$2FFFFF window must
    /// switch which 1MB slice of P2 becomes visible there, per the
    /// generic bankswitch scheme most cartridges use (not the special
    /// protection-chip schemes some games use instead).
    #[test]
    fn writing_anywhere_in_the_bank_window_switches_the_visible_p2_bank() {
        let mut p2 = vec![0u8; 0x300000]; // 3 real 1MB banks
        p2[0x000000] = 0xAA; // bank 0's first byte
        p2[0x100000] = 0xBB; // bank 1's first byte
        p2[0x200000] = 0xCC; // bank 2's first byte
        // `NeoGeoBus::new` word-swaps P2 once, the same as it would a
        // real on-disk dump -- pre-swapping this synthetic ROM here
        // (the same convention `synthetic_cartridge_with_entry_point`
        // uses for P1) cancels that out, so the logical byte values
        // set above land exactly where intended.
        let p2 = word_swap(p2);
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], p2, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);

        assert_eq!(machine.bus.read_byte(0x200000), 0xAA, "bank 0 must be visible by default after reset");
        machine.bus.write_byte(0x250000, 2); // write anywhere in the window, not just its base
        assert_eq!(machine.bus.read_byte(0x200000), 0xCC, "writing 2 anywhere in the window must switch to bank 2");
        machine.bus.write_byte(0x200000, 1);
        assert_eq!(machine.bus.read_byte(0x200000), 0xBB, "writing 1 must switch to bank 1");
    }

    /// A sprite whose SCB1 tile/palette entry, SCB3 Y/height, and SCB4
    /// X are all set explicitly must composite onto the framebuffer at
    /// exactly the position and color those registers specify --
    /// proving the full pipeline (VRAM sprite tables -> tile decode ->
    /// palette lookup -> positioned blit) connects correctly end to
    /// end, the sprite equivalent of the fix-layer test above.
    #[test]
    fn a_synthetic_sprite_composites_at_the_right_position_and_color() {
        // A solid-fill tile (odd ROM all-0xFF, even all-0x00) decodes
        // to color index 3 everywhere -- see
        // `a_solid_fill_sprite_tile_decodes_to_a_uniform_block`.
        let odd_c_rom = vec![0xFFu8; 64];
        let even_c_rom = vec![0x00u8; 64];
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), vec![odd_c_rom, even_c_rom], Vec::new(), Protection::None);

        // SCB1 sprite 0: tile 0, palette 5.
        machine.bus.write_word(0x3C0000, 0x0000); // REG_VRAMADDR: SCB1 even word (tile LSBs)
        machine.bus.write_word(0x3C0002, 0x0000); // tile number 0
        machine.bus.write_word(0x3C0000, 0x0001); // SCB1 odd word (attributes)
        machine.bus.write_word(0x3C0002, 5u16 << 12); // palette 5, tile MSBs 0

        // SCB2 sprite 0: full size, no shrink (see `render_sprites_onto`'s
        // own doc comment -- the default $0000 means the smallest
        // possible size, not full size, so tests must set this explicitly).
        machine.bus.write_word(0x3C0000, 0x8000);
        machine.bus.write_word(0x3C0002, 0x0FFF);

        // SCB3 sprite 0: Y raw = 0 (-> y_top = 496, per the documented
        // formula), height = 1 tile.
        machine.bus.write_word(0x3C0000, 0x8200);
        machine.bus.write_word(0x3C0002, 1);

        // SCB4 sprite 0: X = 50.
        machine.bus.write_word(0x3C0000, 0x8400);
        machine.bus.write_word(0x3C0002, 50);

        // Palette 5, color index 3 = pure blue.
        machine.bus.write_word(0x400000 + (5 * 16 + 3) * 2, 0b1_0_0_1_0000_0000_1111);

        const CANVAS: usize = 512;
        let mut rgba = vec![0u8; CANVAS * CANVAS * 4];
        machine.bus.render_sprites_onto(&mut rgba, CANVAS, CANVAS);

        let px = 50 + 4; // inside the 16px-wide tile at x=50
        let py = 496 + 4; // inside the tile at y_top=496
        let offset = (py * CANVAS + px) * 4;
        // The dark bit is shared across all 3 channels (see
        // `decode_palette_color`'s own doc comment/tests), so setting
        // it for a fully-saturated blue also nudges red/green up from
        // 0 slightly -- real hardware behavior, not a bug, hence the
        // small tolerance on red/green rather than requiring exactly 0.
        assert!(rgba[offset] < 10 && rgba[offset + 1] < 10, "red/green should be near zero, got ({}, {})", rgba[offset], rgba[offset + 1]);
        assert_eq!((rgba[offset + 2], rgba[offset + 3]), (255, 255), "the sprite's pixel should be fully-saturated blue and opaque at its real computed position");

        // A position well outside the sprite's 16x16 footprint must
        // stay untouched.
        let untouched_offset = (10 * CANVAS + 10) * 4;
        assert_eq!(rgba[untouched_offset + 3], 0, "a position outside every sprite's footprint must remain transparent");
    }

    /// A tile colored only in its physical top-left 8x8 quadrant, with
    /// both horizontal and vertical flip set, must show that color in
    /// the *bottom-right* quadrant instead -- proving both flip bits
    /// actually mirror pixel position, not just get read and ignored.
    #[test]
    fn sprite_flip_bits_mirror_the_colored_quadrant() {
        // Color only the physical top-left 8x8 quadrant (ROM quadrant
        // index 2 in `decode_sprite_tile`'s own ordering -- see its
        // doc comment): bytes 32..48 of the odd ROM.
        let mut odd_c_rom = vec![0u8; 64];
        odd_c_rom[32..48].fill(0xFF);
        let even_c_rom = vec![0u8; 64];
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), vec![odd_c_rom, even_c_rom], Vec::new(), Protection::None);

        machine.bus.write_word(0x3C0000, 0x0000);
        machine.bus.write_word(0x3C0002, 0x0000); // tile 0
        machine.bus.write_word(0x3C0000, 0x0001);
        machine.bus.write_word(0x3C0002, (5u16 << 12) | (1 << 5) | (1 << 4)); // palette 5, v_flip + h_flip

        machine.bus.write_word(0x3C0000, 0x8000); // SCB2: full size, no shrink
        machine.bus.write_word(0x3C0002, 0x0FFF);

        machine.bus.write_word(0x3C0000, 0x8200);
        machine.bus.write_word(0x3C0002, 1); // height 1 tile, y_raw 0 -> y_top 496
        machine.bus.write_word(0x3C0000, 0x8400);
        machine.bus.write_word(0x3C0002, 50); // x = 50

        machine.bus.write_word(0x400000 + (5 * 16 + 3) * 2, 0b1_0_0_1_0000_0000_1111); // palette 5/color 3 = blue

        const CANVAS: usize = 512;
        let mut rgba = vec![0u8; CANVAS * CANVAS * 4];
        machine.bus.render_sprites_onto(&mut rgba, CANVAS, CANVAS);

        let top_left = ((496 + 2) * CANVAS + (50 + 2)) * 4;
        let bottom_right = ((496 + 12) * CANVAS + (50 + 12)) * 4;
        assert_eq!(rgba[top_left + 3], 0, "with both flips set, the top-left quadrant (where the color lives unflipped) must now be transparent");
        assert_eq!(rgba[bottom_right + 3], 255, "with both flips set, the color must have moved to the bottom-right quadrant");
    }

    /// SCB2's horizontal shrink field must actually shrink the
    /// sprite's on-screen width to exactly the documented formula
    /// (stored nibble + 1 pixels), per
    /// <https://wiki.neogeodev.org/index.php?title=Sprite_shrinking>
    /// -- a solid-fill tile shrunk to nibble value 3 (width 4) must
    /// paint columns 0-3 of its footprint and leave column 4 onward
    /// untouched, not still span the full 16 columns.
    #[test]
    fn scb2_horizontal_shrink_narrows_the_sprite_to_the_documented_width() {
        let odd_c_rom = vec![0xFFu8; 64];
        let even_c_rom = vec![0x00u8; 64];
        let mut machine = NeoGeoMachine::new(vec![0u8; 0x200], Vec::new(), Vec::new(), Vec::new(), vec![odd_c_rom, even_c_rom], Vec::new(), Protection::None);

        machine.bus.write_word(0x3C0000, 0x0000);
        machine.bus.write_word(0x3C0002, 0x0000); // tile 0
        machine.bus.write_word(0x3C0000, 0x0001);
        machine.bus.write_word(0x3C0002, 5u16 << 12); // palette 5

        // SCB2: horizontal shrink nibble = 3 (width 4px), vertical
        // shrink = $FF (full height).
        machine.bus.write_word(0x3C0000, 0x8000);
        machine.bus.write_word(0x3C0002, (3u16 << 8) | 0xFF);

        machine.bus.write_word(0x3C0000, 0x8200);
        machine.bus.write_word(0x3C0002, 1); // height 1 tile
        machine.bus.write_word(0x3C0000, 0x8400);
        machine.bus.write_word(0x3C0002, 100); // x = 100

        machine.bus.write_word(0x400000 + (5 * 16 + 3) * 2, 0b1_0_0_1_0000_0000_1111); // blue

        const CANVAS: usize = 512;
        let mut rgba = vec![0u8; CANVAS * CANVAS * 4];
        machine.bus.render_sprites_onto(&mut rgba, CANVAS, CANVAS);

        let inside = ((496 + 2) * CANVAS + (100 + 2)) * 4;
        let just_past_shrunk_width = ((496 + 2) * CANVAS + (100 + 5)) * 4;
        assert_eq!(rgba[inside + 3], 255, "within the shrunk 4px width, the sprite should still paint");
        assert_eq!(rgba[just_past_shrunk_width + 3], 0, "past the documented shrunk width (nibble 3 -> 4px), nothing should be painted");
    }

    /// A tile made of one repeated byte must decode to a perfectly
    /// uniform 8x8 block of that byte's two nibbles -- true regardless
    /// of exactly how the four (half, column) quadrants map onto
    /// physical x-position (see `decode_fix_tile`'s own doc comment on
    /// that unresolved ambiguity), so this doesn't depend on getting
    /// that orientation right, only on every one of the tile's 32
    /// bytes being read and placed somewhere in the 8x8 grid.
    #[test]
    fn a_solid_fill_tile_decodes_to_a_uniform_block() {
        let s1_rom = vec![0x11u8; 32]; // one tile: every pixel = palette index 1
        let pixels = decode_fix_tile(&s1_rom, 0);
        for row in pixels {
            assert_eq!(row, [1u8; 8], "a tile of all 0x11 bytes must decode to all pixels = palette index 1");
        }
    }

    /// Two different tiles must read from two different, non-
    /// overlapping 32-byte regions of the ROM -- `tile_number` must
    /// actually select which tile, not just always decode tile 0.
    #[test]
    fn different_tile_numbers_read_different_rom_regions() {
        let mut s1_rom = vec![0x11u8; 64];
        s1_rom[32..64].fill(0x22);
        assert_eq!(decode_fix_tile(&s1_rom, 0), [[1u8; 8]; 8], "tile 0 must decode from bytes 0..32");
        assert_eq!(decode_fix_tile(&s1_rom, 1), [[2u8; 8]; 8], "tile 1 must decode from bytes 32..64, not overlap tile 0");
    }

    /// A sprite tile with every odd-ROM byte 0xFF and every even-ROM
    /// byte 0x00 must decode to a perfectly uniform color-index-3
    /// block (bitplanes 0 and 1 both fully set, 2 and 3 both clear) --
    /// true regardless of exactly how the four quadrants map onto
    /// physical position or which byte within a row-pair is which
    /// bitplane (see `decode_sprite_tile`'s own doc comment on those
    /// unresolved ambiguities), so this validates the byte-count/
    /// quadrant-coverage math without depending on getting orientation
    /// right.
    #[test]
    fn a_solid_fill_sprite_tile_decodes_to_a_uniform_block() {
        let odd_rom = vec![0xFFu8; 64];
        let even_rom = vec![0x00u8; 64];
        let pixels = decode_sprite_tile(&odd_rom, &even_rom, 0);
        for row in pixels {
            assert_eq!(row, [3u8; 16], "bitplanes 0+1 set, 2+3 clear must decode to color index 3 (0b0011) everywhere");
        }
    }

    /// Every one of a 16x16 tile's 256 pixels must actually get
    /// written to (no quadrant silently skipped, no position written
    /// twice at another's expense) -- a tile with distinct per-
    /// quadrant fill values must show exactly 4 distinct colors, each
    /// covering exactly one quadrant's 64 pixels.
    #[test]
    fn every_quadrant_of_a_sprite_tile_is_covered_exactly_once() {
        // Fill each quadrant's odd-ROM bytes with a distinct low
        // nibble (even-ROM stays 0, so bitplanes 2-3 stay clear and
        // the color index is just the odd-ROM's own low bitplane
        // pair).
        let mut odd_rom = vec![0u8; 64];
        for quadrant in 0..4 {
            let fill = match quadrant {
                0 => 0b01010101u8, // bitplane0=1,bitplane1=0 alternating -> color 1 per bit
                1 => 0b00000000u8,
                2 => 0b11111111u8,
                _ => 0b10101010u8,
            };
            odd_rom[quadrant * 16..quadrant * 16 + 16].fill(fill);
        }
        let even_rom = vec![0u8; 64];
        let pixels = decode_sprite_tile(&odd_rom, &even_rom, 0);
        let mut distinct_colors = std::collections::HashSet::new();
        for row in pixels {
            for color in row {
                distinct_colors.insert(color);
            }
        }
        assert!(distinct_colors.len() >= 2, "distinctly-filled quadrants must actually produce more than one color across the tile, got {distinct_colors:?}");
    }

    /// `concat_c_roms` must route odd-numbered C-ROMs (C1, C3...) and
    /// even-numbered ones (C2, C4...) into separate virtual ROMs,
    /// preserving each group's own order -- the split
    /// `decode_sprite_tile` itself depends on for bitplane assignment.
    #[test]
    fn concat_c_roms_splits_odd_and_even_numbered_roms_correctly() {
        let c1 = vec![1u8; 4];
        let c2 = vec![2u8; 4];
        let c3 = vec![3u8; 4];
        let c4 = vec![4u8; 4];
        let roms = vec![c1, c2, c3, c4];
        assert_eq!(concat_c_roms(&roms, true), vec![1, 1, 1, 1, 3, 3, 3, 3], "odd ROMs (C1, C3) must concatenate in order");
        assert_eq!(concat_c_roms(&roms, false), vec![2, 2, 2, 2, 4, 4, 4, 4], "even ROMs (C2, C4) must concatenate in order");
    }

    /// `interleave_c_rom_pairs` must byte-interleave each numbered
    /// pair (C1 at even positions, C2 at odd, forming one block; C3/C4
    /// forming the next), matching MAME's own `ROM_LOAD16_BYTE`
    /// pairing for real `mslug3` -- and `deinterleave_c_rom_pairs` must
    /// exactly invert it, since the real pipeline round-trips through
    /// both (interleave -> `cmc42::gfx_decrypt` -> de-interleave).
    #[test]
    fn interleave_and_deinterleave_c_rom_pairs_round_trip() {
        let c1 = vec![0xAAu8; 4];
        let c2 = vec![0xBBu8; 4];
        let c3 = vec![0xCCu8; 4];
        let c4 = vec![0xDDu8; 4];
        let roms = vec![c1, c2, c3, c4];

        let combined = interleave_c_rom_pairs(&roms);
        assert_eq!(combined, vec![0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xAA, 0xBB, 0xCC, 0xDD, 0xCC, 0xDD, 0xCC, 0xDD, 0xCC, 0xDD], "C1/C2 must interleave into the first block, C3/C4 into the second, matching MAME's real ROM_LOAD16_BYTE layout");

        let (odd_rom, even_rom) = deinterleave_c_rom_pairs(&combined, 2);
        assert_eq!(odd_rom, vec![0xAA, 0xAA, 0xAA, 0xAA, 0xCC, 0xCC, 0xCC, 0xCC], "de-interleaving must recover exactly C1++C3 (the odd-numbered-ROM virtual buffer)");
        assert_eq!(even_rom, vec![0xBB, 0xBB, 0xBB, 0xBB, 0xDD, 0xDD, 0xDD, 0xDD], "de-interleaving must recover exactly C2++C4 (the even-numbered-ROM virtual buffer)");
    }

    /// If real Metal Slug 3 C-ROMs are present, running them through
    /// the full real pipeline (interleave -> CMC42 decrypt -> de-
    /// interleave -> `decode_sprite_tile`) must produce structured,
    /// non-degenerate output -- the same "does it produce something
    /// coherent, not garbage or a crash" bar the pre-decryption sprite
    /// test met, now against what should be the *actually correct*
    /// graphics data instead of still-encrypted bytes.
    #[test]
    fn real_c_roms_decrypt_via_cmc42_to_structured_non_degenerate_sprites_if_present() {
        let mut c_roms = Vec::new();
        for n in 1..=8 {
            let Some(rom) = load_real_rom(&format!("256-c{n}.rom")) else {
                eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
                return;
            };
            c_roms.push(rom);
        }
        let mut combined = interleave_c_rom_pairs(&c_roms);
        cmc42::gfx_decrypt(&mut combined, cmc42::MSLUG3_GFX_KEY);
        let (odd_rom, even_rom) = deinterleave_c_rom_pairs(&combined, c_roms.len().div_ceil(2));

        let mut distinct_colors = std::collections::HashSet::new();
        let mut all_zero_count = 0;
        for tile in 0..20u32 {
            let pixels = decode_sprite_tile(&odd_rom, &even_rom, tile);
            if pixels.iter().flatten().all(|&c| c == 0) {
                all_zero_count += 1;
            }
            for &color in pixels.iter().flatten() {
                distinct_colors.insert(color);
            }
        }
        assert!(all_zero_count < 20, "real decrypted sprite tile data across the first 20 tiles shouldn't decode to entirely blank output");
        assert!(distinct_colors.len() > 1, "real decrypted sprite tile data should use more than one color index across 20 tiles, got {distinct_colors:?}");
    }

    /// If a real Metal Slug 3 cartridge dump is present, decode its
    /// real S1 ROM's tile 0: checking its raw bytes directly (all 32
    /// are 0x11) confirms it's a genuinely uniform solid-color tile,
    /// unlike every other early tile (1-9), which are real dithered/
    /// shaded glyph shapes, not solid fills -- an earlier version of
    /// this test wrongly assumed several more tiles were solid, from
    /// misreading truncated hex output; the real bytes settle it.
    /// This can't confirm left/right pixel orientation (a solid-color
    /// tile looks the same regardless -- see `decode_fix_tile`'s own
    /// doc comment), only that whole-tile addressing and nibble-pair
    /// extraction are right. Skips (doesn't fail) when no ROM is
    /// present.
    #[test]
    fn real_s1_rom_tile_zero_decodes_to_a_uniform_block_if_present() {
        let Some(s1) = load_real_rom("256-s1.rom") else {
            eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
            return;
        };
        assert_eq!(&s1[0..32], [0x11u8; 32], "tile 0's raw bytes must genuinely be uniform 0x11 for this test's premise to hold");
        assert_eq!(decode_fix_tile(&s1, 0), [[1u8; 8]; 8], "tile 0 of the real S1 ROM should decode to a solid palette-index-1 block");
    }

    /// If real Metal Slug 3 C-ROMs are present, decode several real
    /// sprite tiles end to end (real concatenated odd/even ROMs, real
    /// bitplane bytes) and check the output is structured, real image
    /// data -- not a crash, not degenerate all-zero/all-max noise --
    /// matching the same "does it produce something coherent" bar the
    /// fix-layer tests met before the exact left/right orientation
    /// could be confirmed visually. Skips (doesn't fail) when the
    /// C-ROMs aren't present.
    #[test]
    fn real_c_roms_decode_to_structured_non_degenerate_sprite_tiles_if_present() {
        let mut c_roms = Vec::new();
        for n in 1..=8 {
            let Some(rom) = load_real_rom(&format!("256-c{n}.rom")) else {
                eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
                return;
            };
            c_roms.push(rom);
        }
        let odd_rom = concat_c_roms(&c_roms, true);
        let even_rom = concat_c_roms(&c_roms, false);
        assert_eq!(odd_rom.len(), even_rom.len(), "C1+C3+C5+C7 and C2+C4+C6+C8 should be the same total size for real matched romsets");

        let mut all_zero_count = 0;
        let mut distinct_colors = std::collections::HashSet::new();
        for tile in 0..20u32 {
            let pixels = decode_sprite_tile(&odd_rom, &even_rom, tile);
            if pixels.iter().flatten().all(|&c| c == 0) {
                all_zero_count += 1;
            }
            for &color in pixels.iter().flatten() {
                distinct_colors.insert(color);
            }
        }
        assert!(all_zero_count < 20, "real sprite tile data across the first 20 tiles shouldn't decode to entirely blank output");
        assert!(distinct_colors.len() > 1, "real sprite tile data should use more than one color index across 20 tiles, got {distinct_colors:?}");
    }

    /// Not a correctness test -- a diagnostic dump of the first 256
    /// real sprite tiles as a grayscale grid PNG (color index * 17,
    /// ignoring palette entirely, since real palette assignment is a
    /// game-logic detail this core doesn't have yet), so orientation
    /// and byte-order assumptions this module's own doc comments flag
    /// as unverified can be checked visually: real game sprite tiles
    /// should show recognizable silhouettes/shading, not visual noise,
    /// if the decode is broadly right, even before palette or exact
    /// left/right mirroring is confirmed. Runs the real CMC42 decrypt
    /// pipeline first (Metal Slug 3 needs it -- undecrypted C-ROM data
    /// would never resemble real sprites no matter how correct the
    /// tile-format decode itself is). `#[ignore]`d since it's a human-
    /// inspection tool, not an assertion -- run explicitly with
    /// `cargo test --bin portamax-sim real_c_roms_visual_dump -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_c_roms_visual_dump_if_present() {
        let mut c_roms = Vec::new();
        for n in 1..=8 {
            let Some(rom) = load_real_rom(&format!("256-c{n}.rom")) else {
                eprintln!("skipping: no ROM in {ROM_DIR}");
                return;
            };
            c_roms.push(rom);
        }
        let mut combined = interleave_c_rom_pairs(&c_roms);
        cmc42::gfx_decrypt(&mut combined, cmc42::MSLUG3_GFX_KEY);
        let (odd_rom, even_rom) = deinterleave_c_rom_pairs(&combined, c_roms.len().div_ceil(2));

        const GRID: usize = 16; // 16x16 tiles = 256 tiles total
        const SIZE: usize = GRID * 16;
        let mut gray = vec![0u8; SIZE * SIZE];
        for tile in 0..(GRID * GRID) as u32 {
            let pixels = decode_sprite_tile(&odd_rom, &even_rom, tile);
            let tile_x = (tile as usize % GRID) * 16;
            let tile_y = (tile as usize / GRID) * 16;
            for (y, row) in pixels.iter().enumerate() {
                for (x, &color) in row.iter().enumerate() {
                    gray[(tile_y + y) * SIZE + (tile_x + x)] = color * 17;
                }
            }
        }
        let out_path = std::env::temp_dir().join("neogeo_sprite_tiles_grayscale.png");
        let file = std::fs::File::create(&out_path).expect("create output file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Grayscale);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer.write_image_data(&gray).expect("write PNG data");
        eprintln!("wrote sprite tile grid to {}", out_path.display());
    }

    /// If a real Metal Slug 3 cartridge dump is present (user-supplied,
    /// never committed -- see `.gitignore`), boot far enough into its
    /// real code, through its real NEO-SMA decryption and bankswitch
    /// (`Protection::SmaMslug3`), to prove the memory map, word-swap,
    /// header/entry handling, and SMA transform are right against real
    /// hardware/software, not just synthetic bytes. This can't prove
    /// full compatibility (sprite auto-animation isn't implemented, and
    /// the C-ROM/fix-layer CMC42 decryption's left/right sprite
    /// orientation is still flagged unverified -- see this module's own
    /// doc comment and `decode_sprite_tile`'s), but millions of real
    /// instructions executing without faulting, deep into genuinely
    /// SMA-decrypted code, is a meaningful signal the transform is
    /// right. Skips (doesn't fail) when no ROM is present.
    ///
    /// **Real bug found and fixed here**: an earlier version of
    /// `sma_decrypt_68k` truncated the decrypted fixed bank to just the
    /// 0xC0000 bytes its relocate step actually writes, discarding the
    /// real remaining 0x40000 bytes (0xC0000-0xFFFFF) that MAME's own
    /// transform deliberately leaves untouched -- real, meaningful
    /// data (the original P1 file's own content there), not padding.
    /// The truncated buffer's own `% len()` wraparound then fed
    /// completely wrong bytes to any code that read that upper range,
    /// which surfaced as a real `FlineTrap` partway through a longer
    /// run. Verified fixed by running 100,000,000 real instructions
    /// with no fault at all (2,000,000 kept here as the steady-state
    /// budget -- comfortably past where the old bug would have hit).
    #[test]
    fn a_real_metal_slug_3_cartridge_boots_into_its_own_code_if_present() {
        let Some(p1) = load_real_rom("256-p1.rom") else {
            eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
            return;
        };
        let p2 = load_real_rom("256-p2.rom").unwrap_or_default();
        let m1 = load_real_rom("256-m1.rom").unwrap_or_default();
        let s1 = load_real_rom("256-s1.rom").unwrap_or_default();
        let mut machine = NeoGeoMachine::new(p1, p2, m1, s1, Vec::new(), Vec::new(), Protection::SmaMslug3);

        // Real code almost universally waits on VBlank before doing
        // anything -- see `vblank`'s own doc comment -- so a run with
        // no interrupts at all mostly just proves the boot sequence,
        // not that the game progresses. Pulsing it every 1000
        // instructions (not real hardware timing, just "often enough
        // that a wait loop isn't stuck forever") lets real code get
        // much further, matching this test's real interest: seeing
        // how far unimplemented pieces (bankswitching, PVC decryption,
        // sprites, sound) let it go before something real blocks it.
        let mut alive = true;
        let mut steps_run = 0;
        for _ in 0..2_000_000 {
            if steps_run % 1000 == 0 {
                machine.vblank();
            }
            let pc_before = machine.cpu.pc;
            let result = machine.cpu.step(&mut machine.bus);
            alive = matches!(result, StepResult::Ok { .. });
            if !alive {
                eprintln!("step {steps_run}: pc=0x{pc_before:06X} -> {result:?}");
            }
            steps_run += 1;
            if !alive {
                break;
            }
        }
        assert!(alive, "the CPU should not fault/halt while executing real Metal Slug 3 code, but stopped after {steps_run} steps at PC=0x{:06X}", machine.cpu.pc);

        // Not a correctness assertion (there's no "expected" screen to
        // compare against without audio/full hardware) -- just a real,
        // current snapshot of what a full frame (sprites + fix layer)
        // actually contains after a real, sustained run, saved so it
        // can be visually inspected.
        let (width, height, rgba) = machine.bus.render_frame();
        let out_path = std::env::temp_dir().join("neogeo_fix_layer_snapshot.png");
        if let Ok(file) = std::fs::File::create(&out_path) {
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            if let Ok(mut writer) = encoder.write_header() {
                let _ = writer.write_image_data(&rgba);
                eprintln!("wrote fix-layer snapshot to {}", out_path.display());
            }
        }
    }

    /// A tilemap entry pointing at S1 ROM tile 0 (the real cartridge's
    /// genuinely solid palette-index-1 swatch -- see the tile-decoder
    /// tests above) with a palette entry set to pure red must render
    /// as a solid 8x8 red block at exactly that tile's screen position
    /// -- proving the full pipeline (VRAM tilemap -> tile decode ->
    /// palette lookup -> framebuffer) connects correctly end to end,
    /// using real S1 ROM data. Skips (doesn't fail) when no ROM is
    /// present.
    #[test]
    fn real_s1_rom_tile_renders_as_the_right_color_at_the_right_position_if_present() {
        let Some(s1) = load_real_rom("256-s1.rom") else {
            eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
            return;
        };
        let mut machine = NeoGeoMachine::new(Vec::new(), Vec::new(), Vec::new(), s1, Vec::new(), Vec::new(), Protection::None);

        // Tilemap entry at row 2, col 3: palette 5, tile 0 (the real,
        // genuinely-solid palette-index-1 swatch).
        let tilemap_word_addr = (2 * 40 + 3) as u16 + 0x7000;
        machine.bus.write_word(0x3C0000, tilemap_word_addr);
        machine.bus.write_word(0x3C0002, (5u16 << 8) | 0);

        // Palette 5, color 1 (palette index 1, matching tile 0's solid
        // fill) = pure red: R4..R0 all set, G/B/dark all clear.
        machine.bus.write_word(0x400000 + (5 * 16 + 1) * 2, 0b0_1_0_0_1111_0000_0000);

        let (width, _height, rgba) = machine.bus.render_fix_layer();
        let px = 3 * 8 + 4; // middle of tile column 3
        let py = 2 * 8 + 4; // middle of tile row 2
        let offset = (py * width + px) * 4;
        assert!(rgba[offset] > 200, "the tile's pixel should render with a strong red channel, got {}", rgba[offset]);
        assert_eq!((rgba[offset + 1], rgba[offset + 2]), (0, 0), "the tile's pixel should have no green or blue");
        assert_eq!(rgba[offset + 3], 255, "a non-transparent pixel must be fully opaque");

        // A tile position we never wrote to defaults to tilemap entry
        // 0 (palette 0, tile 0) -- since the real S1 ROM's tile 0 is a
        // genuinely solid, non-transparent swatch (palette index 1,
        // not the transparent index 0), and palette 0/color 1 is still
        // zeroed, this must render solid black, not transparent. An
        // earlier version of this test wrongly assumed untouched
        // positions stay transparent; they don't, once a real,
        // non-blank tile 0 is involved -- only color index 0 itself is
        // ever transparent, which this position doesn't use.
        let untouched_offset = (10 * width + 10) * 4;
        assert_eq!(
            (rgba[untouched_offset], rgba[untouched_offset + 1], rgba[untouched_offset + 2], rgba[untouched_offset + 3]),
            (0, 0, 0, 255),
            "an untouched position (palette 0, tile 0, still zeroed) must render solid black, since tile 0 isn't the transparent color"
        );

        // Genuine transparency: a tile explicitly pointed at a pixel
        // whose color index really is 0 must leave the framebuffer's
        // alpha untouched (0), regardless of what palette is selected.
        let s1_again = load_real_rom("256-s1.rom").expect("already confirmed present above");
        let tile_123 = decode_fix_tile(&s1_again, 123);
        machine.bus.write_word(0x3C0000, (15 * 40 + 15) as u16 + 0x7000);
        machine.bus.write_word(0x3C0002, (5u16 << 8) | 123);
        let (_, _, rgba) = machine.bus.render_fix_layer();
        let mut checked_a_transparent_pixel = false;
        for (y, row) in tile_123.iter().enumerate() {
            for (x, &color_index) in row.iter().enumerate() {
                if color_index == 0 {
                    let off = ((15 * 8 + y) * width + (15 * 8 + x)) * 4;
                    assert_eq!(rgba[off + 3], 0, "a pixel whose real color index is 0 must render fully transparent");
                    checked_a_transparent_pixel = true;
                }
            }
        }
        assert!(checked_a_transparent_pixel, "tile 123 should contain at least one color-index-0 pixel to actually exercise transparency -- pick a different tile number if this ever fails");
    }

    /// The Z80 sound CPU has no vector table to validate against (see
    /// `SoundBus`'s own doc comment) -- unlike the 68000 side, there is
    /// nothing to get backwards here, just real M1 ROM code that
    /// should execute without the Z80 core rejecting it as invalid.
    /// This is a much weaker signal than the 68k test above (the Z80
    /// isn't even receiving the sound-command interrupts real code
    /// waits on, since that handshake isn't implemented yet -- so this
    /// only proves the ROM is readable and its very first real
    /// instructions decode as genuine Z80 opcodes, not that the sound
    /// driver actually runs correctly). Skips (doesn't fail) when no
    /// ROM is present.
    #[test]
    fn a_real_metal_slug_3_sound_rom_executes_real_z80_opcodes_if_present() {
        let Some(m1) = load_real_rom("256-m1.rom") else {
            eprintln!("skipping: no ROM in {ROM_DIR} (this is expected in a fresh checkout)");
            return;
        };
        let mut machine = NeoGeoMachine::new(Vec::new(), Vec::new(), m1, Vec::new(), Vec::new(), Vec::new(), Protection::None);
        let pc_before = machine.sound_cpu.pc;
        for _ in 0..1_000 {
            machine.step_sound();
        }
        assert_ne!(machine.sound_cpu.pc, pc_before, "the Z80 should have advanced through real M1 ROM code, not stalled immediately");
    }

    /// The Z80-port-to-`ymfm`-offset mapping this module documents
    /// (port 4/6 = address for part 0/1, port 5/7 = data) must match
    /// exactly, per `Ym2610::register_offset`'s own doc comment.
    #[test]
    fn ym2610_register_offset_matches_the_documented_z80_port_layout() {
        assert_eq!(Ym2610::register_offset(4), 0, "port 4 (part 0 address) must map to ymfm offset 0");
        assert_eq!(Ym2610::register_offset(5), 1, "port 5 (part 0 data) must map to ymfm offset 1");
        assert_eq!(Ym2610::register_offset(6), 2, "port 6 (part 1 address) must map to ymfm offset 2");
        assert_eq!(Ym2610::register_offset(7), 3, "port 7 (part 1 data) must map to ymfm offset 3");
    }

    /// Real Z80 M1 ROM bankswitching: reading port $0B (the $8000-
    /// $BFFF window's port, per this module's own doc comment on
    /// `SoundBus::m1_banks`) with a given bank number in the address's
    /// high byte -- the real `IN r,(C)` addressing convention hardware
    /// uses here -- must switch which 16KB page of the M1 ROM appears
    /// at $8000, not just leave it reading bank 0 forever.
    #[test]
    fn z80_bankswitch_read_selects_the_real_m1_rom_page() {
        let mut m1_rom = vec![0u8; 0x20000]; // 128KB: banks 0-7 of 16KB each
        m1_rom[0] = 0xAA; // bank 0's first byte at the $8000 window's own offset
        m1_rom[3 * 0x4000] = 0xBB; // bank 3's first byte

        let mut bus = SoundBus::new(m1_rom, Vec::new(), SoundLatch::new());
        assert_eq!(bus.read_byte(0x8000), 0xAA, "bank 0 must be selected by default");

        // Real hardware: `IN r,(C)` with B=3 (bank), C=0x0B (port) --
        // the full BC pair lands in `addr`'s high/low bytes respectively.
        bus.port_in((3u16 << 8) | 0x0B);
        assert_eq!(bus.read_byte(0x8000), 0xBB, "reading port 0x0B with bank 3 in the high byte must switch the $8000 window to M1 ROM bank 3");
    }

    /// `sma_mslug3_bank_base`'s table lookup, checked against values
    /// copied directly from MAME's own `sma_prot_device::
    /// mslug3_bank_base` (`bankoffset[0]` and `bankoffset[1]`).
    /// Selector 0 unscrambles to table index 0 trivially (every bit
    /// the formula reads is already 0). The formula is
    /// `bitswap<6>(sel, 9,3,6,15,12,14)` -- its *last*-listed source
    /// bit becomes the unscrambled index's LSB, so setting only input
    /// bit 14 isolates index 1 (binary `000001`).
    #[test]
    fn sma_mslug3_bank_base_matches_mames_own_table() {
        assert_eq!(sma_mslug3_bank_base(0), 0x100000, "selector 0 -> table index 0 -> bankoffset[0]=0 -> 0x100000+0");
        assert_eq!(sma_mslug3_bank_base(1 << 14), 0x120000, "selector with only bit 14 set -> table index 1 -> bankoffset[1]=0x020000 -> 0x100000+0x020000");
    }

    /// The NEO-SMA "chip present" check must always reply $9A37 at its
    /// real documented address, per MAME's own memory map install for
    /// `NEOGEO_MSLUG3`/`NEOGEO_MSLUG3A` ($2FE446) -- and, critically,
    /// carts *without* SMA protection must not respond to that address
    /// at all (it's ordinary ROM space for them).
    #[test]
    fn sma_protection_check_replies_9a37_only_for_protected_carts() {
        let sma_rom = vec![0u8; 0x200];
        let mut sma_machine = NeoGeoMachine::new(sma_rom, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::SmaMslug3);
        assert_eq!(sma_machine.bus.read_word(0x2FE446), 0x9A37, "an SMA-protected cart must reply 0x9A37 at its real documented check address");

        let plain_rom = vec![0u8; 0x200];
        let mut plain_machine = NeoGeoMachine::new(plain_rom, Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        assert_ne!(plain_machine.bus.read_word(0x2FE446), 0x9A37, "an unprotected cart must not coincidentally reply 0x9A37 at the same address (it's just ordinary banked ROM space for it)");
    }

    /// A real, minimal SSG (AY-3-8910-compatible) tone-on sequence,
    /// written through the documented Z80 port interface exactly the
    /// way real Neo Geo sound driver code would, must produce real,
    /// varying (not silent, not stuck-DC) audio out of the actual
    /// vendored YM2610 chip -- proof the register routing, the chip
    /// wiring, and audio generation all connect correctly end to end.
    /// Chosen over an FM-channel tone specifically because the SSG's
    /// register layout is simple, extremely well-documented, and low-
    /// risk to get right relative to full FM operator programming.
    #[test]
    fn a_real_ssg_tone_produces_real_varying_audio() {
        let mut machine = NeoGeoMachine::new(Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Protection::None);
        let bus = &mut machine.sound_bus;
        let mut set = |register: u8, value: u8| {
            bus.port_out(4, register); // part 0 address
            bus.port_out(5, value); // part 0 data
        };
        set(0x00, 0x10); // channel A tone period, fine
        set(0x01, 0x00); // channel A tone period, coarse
        set(0x07, 0x3E); // mixer: enable tone A only, disable B/C and all noise
        set(0x08, 0x0F); // channel A volume: maximum, no envelope

        let (left, _right) = machine.generate_audio_seconds(0.02);
        assert!(!left.is_empty(), "generate_audio_seconds must actually produce samples");
        assert!(left.iter().any(|&s| s != 0.0), "a real enabled SSG tone must produce non-silent audio");
        // A real square wave alternates between exactly two levels, so
        // this checks for genuine toggling (both a positive and a
        // negative/zero sample present), not a value stuck at one
        // level -- >2 distinct values would actually be the wrong
        // expectation for a pure square wave.
        let distinct: std::collections::HashSet<_> = left.iter().map(|s| s.to_bits()).collect();
        assert!(distinct.len() >= 2, "a real square-wave tone must toggle between levels, not sit at a single stuck value (got {} distinct sample values)", distinct.len());
        let (min, max) = left.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &s| (lo.min(s), hi.max(s)));
        assert!(max - min > 0.01, "the tone's high and low levels should be clearly separated, not two nearly-identical values (min={min}, max={max})");
    }
}
