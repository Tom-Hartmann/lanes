//! Lanes core engine.
//!
//! Everything that touches capture, audio, encoding, muxing and the replay
//! buffer lives here, independent of Tauri so it can be unit-tested and driven
//! from a CLI. Hot paths run on dedicated OS threads; this crate never needs an
//! async runtime.

pub mod audio;
pub mod av;
pub mod capture;
pub mod clock;
pub mod config;
pub mod convert;
pub mod encode;
pub mod engine;
pub mod error;
pub mod games;
pub mod gpu;
pub mod media;
pub mod mux;
pub mod replay;
pub mod sysinfo;
pub mod util;

pub use clock::Timestamp;
pub use error::{Error, Result};
