use crate::limits::{Exceeded, Limits};
use crate::ocr::error::{Result, ScanError};
use std::path::Path;

pub type GrayImage = image::GrayImage;

/// Decode any supported image format from bytes and convert to 8-bit grey.
/// The format is read from the bytes, not from a file name.
///
/// Uses the documented [`Limits`]; see [`decode_bytes_within`] for why this is
/// the only place the pixel ceiling can usefully be enforced.
pub fn decode_bytes(bytes: &[u8]) -> Result<GrayImage> {
    decode_bytes_within(bytes, &Limits::default())
}

/// As [`decode_bytes`], with the ceilings supplied.
///
/// ## The choke point
///
/// **Every buffer in the OCR pipeline is sized from a decoded image's own
/// dimensions**, and those dimensions come from a header a stranger wrote. The
/// summed-area table in `flatten`, the projection buffers in `deskew`, the
/// cell grid in `frame`, the ink mask in `binarize`, the input tensor in
/// `readany-ocr` — all of them multiply width by height.
///
/// Auditing every one of those multiplications is necessary and not
/// sufficient, because new ones get written. Bounding the dimensions *here*,
/// before any of them run, bounds all of them at once and bounds the ones
/// nobody has written yet.
///
/// Measured before this check existed: a **137 KB PNG** declaring 12000 square
/// decoded to **144 megapixels** — 3.6× the documented ceiling — in 0.04 s
/// using 295 MB. The ceiling had been written down, unit-tested and never
/// called on this path.
///
/// The dimensions are read from the header **before** decoding, so an
/// oversized image costs a header parse rather than an allocation.
pub fn decode_bytes_within(bytes: &[u8], limits: &Limits) -> Result<GrayImage> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ScanError::Decode(e.to_string()))?;

    if reader.format().is_none() {
        return Err(ScanError::Unsupported(
            "unrecognised image content; anyscan reads JPEG, PNG, TIFF, BMP and WebP".into(),
        ));
    }

    // The header, not the pixels. `into_dimensions` consumes the reader, so
    // the decoding reader is built again from the same in-memory bytes.
    let format = reader.format();
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| ScanError::Decode(e.to_string()))?;
    if !limits.pixels_within(width as u64, height as u64, 1) {
        return Err(ScanError::TooLarge(Exceeded::new(
            "image pixels",
            limits.pixels_per_page,
            (width as u64).saturating_mul(height as u64),
        )));
    }

    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    reader.set_format(format.expect("a format was found above"));
    // Belt to the dimension check's braces: the decoder refuses to allocate
    // past the ceiling even if a header under-reports what it will produce.
    let mut image_limits = image::Limits::default();
    image_limits.max_alloc = Some(limits.pixels_per_page.saturating_mul(4));
    reader.limits(image_limits);

    let img = reader
        .decode()
        .map_err(|e| ScanError::Decode(e.to_string()))?;
    Ok(img.to_luma8())
}

pub fn decode_path(path: impl AsRef<Path>) -> Result<GrayImage> {
    let bytes = std::fs::read(path.as_ref())?;
    decode_bytes(&bytes)
}
