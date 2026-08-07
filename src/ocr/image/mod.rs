//! Image loading and the geometric corrections that must happen before
//! recognition. Everything here is plain Rust on top of the `image` crate,
//! so it cross-compiles and builds for WebAssembly without native libraries.

pub mod binarize;
pub mod correction;
pub mod decode;
pub mod deskew;
pub mod frame;
pub mod orient;

pub use correction::Correction;
pub use decode::{decode_bytes, decode_path, GrayImage};
