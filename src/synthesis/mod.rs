//! The Portamax synthesis platform: a serializable DSP graph ("patch")
//! compiled into a real-time engine, shared by every app that builds
//! sounds from it -- Oracle (patches written by a language model) and
//! Atlas (curated presets with macros and morphing).
//!
//! - `patch`: the JSON format (versioned; see `patch::migrate`) -- nodes,
//!   expressions, params with metadata, macros, morph states.
//! - `blocks`: the DSP node library. `blocks::SPECS` is the single source
//!   of truth: the compiler validates against it, and Oracle's AI prompt
//!   is generated from it, so a new block is usable everywhere at once.
//! - `expr`: small safe expressions -- how nodes connect and modulate each
//!   other (FM/PM/AM/ring, modulators modulating modulators, macros).
//! - `engine`: `compile` (off the audio thread: validation, allocation)
//!   and `Engine::process` (audio thread: no allocation, no locks, no
//!   panicking paths), with a voice manager for instruments.
//! - `evolve`: randomize / mutate / breed on normalized parameters.
//!
//! See docs/SYNTH_PLATFORM.md for the architecture.

pub mod blocks;
pub mod engine;
pub mod evolve;
pub mod expr;
pub mod patch;
