use crate::ocr::error::{Result, ScanError};
use std::path::Path;

pub type GrayImage = image::GrayImage;

/// Decode any supported image format from bytes and convert to 8-bit grey.
/// The format is read from the bytes, not from a file name.
pub fn decode_bytes(bytes: &[u8]) -> Result<GrayImage> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ScanError::Decode(e.to_string()))?;

    if reader.format().is_none() {
        return Err(ScanError::Unsupported(
            "unrecognised image content; anyscan reads JPEG, PNG, TIFF, BMP and WebP".into(),
        ));
    }

    let img = reader
        .decode()
        .map_err(|e| ScanError::Decode(e.to_string()))?;
    Ok(img.to_luma8())
}

pub fn decode_path(path: impl AsRef<Path>) -> Result<GrayImage> {
    let bytes = std::fs::read(path.as_ref())?;
    decode_bytes(&bytes)
}
