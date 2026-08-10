//! Decoding an image whose header a stranger wrote.
//!
//! This is the choke point named in `docs/security.md`, so the panicking forms
//! are denied here rather than trusted to review.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::limits::{Exceeded, Limits};
use crate::ocr::error::{Result, ScanError};
use std::path::Path;

pub type GrayImage = image::GrayImage;

/// Decode a picture this crate has no decoder for.
///
/// **This is the third seam, and it is shaped like the other two.**
/// [`crate::ocr::OcrBackend`] is declared here and implemented in
/// `readany-ocr`, which holds ONNX. [`crate::pdf::render::Rasterise`] is
/// declared here and implemented in `readany-pdfium`, in Android's
/// `PdfRenderer` and in iOS PDFKit. This is declared here and implemented by
/// whatever the platform already ships.
///
/// ## Why this one had to exist
///
/// **Every iPhone photograph is HEIC by default**, and HEIC is the container
/// the corpus arrived in. The pure-Rust decoder this crate uses reads JPEG,
/// PNG, TIFF, BMP and WebP and will never read HEIC, so a receipt scanner
/// without this seam cannot open the majority of its own input.
///
/// The obvious fix is `libheif`. It is **LGPL**, and static linking on a phone
/// makes that a real question rather than a formality — this project has
/// already been through it with MuPDF. So no decoder is linked here at all:
///
/// | Where | Decoder | Licence question |
/// |---|---|---|
/// | iOS | ImageIO, in the platform | none |
/// | Android | `BitmapFactory`, API 28 and above | none |
/// | Desktop, macOS | ImageIO through `sips`, in `readany-ocr` | none |
/// | Server, Linux | `libheif`, or a named refusal | to settle before it ships |
///
/// The two platforms that matter dodge it entirely, and a **named refusal is a
/// valid answer** — [`ScanError::NeedsPlatformDecoder`] exists so a caller can
/// tell "we cannot open this container here" from "this file is damaged".
///
/// ## Why a trait rather than a feature flag
///
/// A trait with no implementor compiles anywhere, which is the point: `readany`
/// still builds for `wasm32-unknown-unknown` with this in it, because nothing
/// here names a platform.
///
/// ## Two methods, and nothing else
///
/// A decoder must be able to say whether it can open these bytes, and to open
/// them. Anything more would make the seam one platform's shape.
///
/// [`DecodeImage::handles`] is separate from [`DecodeImage::decode`] because
/// the answer is a **runtime** property, not a compile-time one: the same
/// Android build decodes HEIC on API 28 and cannot on API 26. Without it, "this
/// phone is too old" and "this photograph is corrupt" arrive as the same error.
pub trait DecodeImage {
    /// Can this decoder open these bytes?
    ///
    /// Read the bytes, not a file name. Implementations should answer from the
    /// container signature and from what the running platform can actually do,
    /// and must not answer yes speculatively — a yes here is what suppresses
    /// the named refusal.
    fn handles(&self, bytes: &[u8]) -> bool;

    /// Decode to 8-bit grey.
    ///
    /// Grey, not colour, for the same reason [`crate::pdf::render::PageImage`]
    /// is grey: OCR discards colour immediately and a 12 megapixel photograph
    /// is 12 MB grey against 36 MB in RGB. Convert inside the decoder, where
    /// the platform can usually do it during the decode.
    ///
    /// The caller re-checks the result against the pixel ceiling, so an
    /// implementation does not have to — but returning an image far past it
    /// wastes the allocation this crate exists to bound.
    fn decode(&self, bytes: &[u8]) -> Result<GrayImage>;
}

/// The HEIF family, recognised from the ISO base media `ftyp` box.
///
/// Returns the container's common name, or `None`. This is a byte signature and
/// not platform knowledge, so it belongs here rather than in any decoder: the
/// refusal has to be able to name the format on a build that has no decoder for
/// it at all.
///
/// The layout is `[4-byte box size][ftyp][4-byte major brand]`. Only the brand
/// is read; the compatible-brand list that follows adds nothing a refusal
/// needs.
pub fn heif_brand(bytes: &[u8]) -> Option<&'static str> {
    if bytes.get(4..8)? != b"ftyp" {
        return None;
    }
    match bytes.get(8..12)? {
        // Stills and sequences, HEVC-coded. What an iPhone writes.
        b"heic" | b"heix" | b"heim" | b"heis" => Some("HEIC"),
        b"hevc" | b"hevx" | b"hevm" | b"hevs" => Some("HEIC"),
        // AV1 in the same container. What newer Android cameras write.
        b"avif" | b"avis" => Some("AVIF"),
        // The generic image brands. A HEIC written by a non-Apple encoder
        // frequently carries `mif1` as its major brand.
        b"mif1" | b"msf1" | b"mif2" => Some("HEIF"),
        _ => None,
    }
}

/// Decode any supported image format from bytes and convert to 8-bit grey.
/// The format is read from the bytes, not from a file name.
///
/// Uses the documented [`Limits`]; see [`decode_bytes_within`] for why this is
/// the only place the pixel ceiling can usefully be enforced.
pub fn decode_bytes(bytes: &[u8]) -> Result<GrayImage> {
    decode_bytes_within(bytes, &Limits::default())
}

/// As [`decode_bytes`], offering a platform decoder for containers this crate
/// cannot open on its own.
///
/// The built-in decoder is tried first and the platform one only where it said
/// it did not recognise the format at all. That ordering is deliberate: a JPEG
/// must decode identically on every surface, and only the formats this crate
/// genuinely cannot read are allowed to depend on where they are being read.
///
/// A damaged JPEG is **not** handed on. It failed as a JPEG, and asking a
/// second decoder to have a go would turn one clear error into two vague ones.
pub fn decode_bytes_using(
    bytes: &[u8],
    limits: &Limits,
    decoder: Option<&dyn DecodeImage>,
) -> Result<GrayImage> {
    let unrecognised = match decode_bytes_within(bytes, limits) {
        Ok(img) => return Ok(img),
        Err(e @ ScanError::Unsupported(_)) => e,
        Err(other) => return Err(other),
    };

    let Some(decoder) = decoder.filter(|d| d.handles(bytes)) else {
        return Err(match heif_brand(bytes) {
            Some(format) => ScanError::NeedsPlatformDecoder {
                format,
                // Named per target, because a refusal suggesting a remedy the
                // reader cannot follow is barely better than a silent one. A
                // browser has no HEIC decoder to lend and cannot be given one,
                // so telling it about ImageIO is telling it about somebody
                // else's machine — the same defect as `error.rs` naming an
                // `Options::pdf_password` field that did not exist.
                #[cfg(target_arch = "wasm32")]
                why: "this build links no decoder for it, and a browser has \
                      none to lend — WebAssembly cannot decode HEIC today, so \
                      convert the photograph to JPEG or PNG before sending it",
                #[cfg(not(target_arch = "wasm32"))]
                why: "this build links no decoder for it — iOS decodes it with \
                      ImageIO and Android with BitmapFactory, through \
                      readany::ocr::image::DecodeImage",
            },
            None => unrecognised,
        });
    };

    let img = decoder.decode(bytes)?;
    // The ceiling again, on pixels this crate did not decode and did not size.
    // `decode_bytes_within` explains why this is the only place it can be
    // enforced usefully; a platform decoder walks straight past that check, so
    // it is repeated here rather than assumed.
    let (width, height) = img.dimensions();
    if !limits.pixels_within(width as u64, height as u64, 1) {
        return Err(ScanError::TooLarge(Exceeded::new(
            "image pixels",
            limits.pixels_per_page,
            (width as u64).saturating_mul(height as u64),
        )));
    }
    Ok(img)
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

    // Bound once, by value: `ImageFormat` is `Copy`, so the format survives the
    // reader being consumed below and there is nothing left to re-check later.
    let Some(format) = reader.format() else {
        return Err(ScanError::Unsupported(
            "unrecognised image content; anyscan reads JPEG, PNG, TIFF, BMP and WebP".into(),
        ));
    };

    // The header, not the pixels. `into_dimensions` consumes the reader, so
    // the decoding reader is built again from the same in-memory bytes.
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
    reader.set_format(format);
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

#[cfg(test)]
mod tests {
    // `unwrap` in a test is an assertion. See the note on the same allow in
    // `route`.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;

    /// The first twelve bytes of the ISO base media header, and nothing else.
    ///
    /// Written here rather than taken from the corpus on purpose: the corpus is
    /// one real person's receipts and must not enter this repository. A brand
    /// box is a signature, not a photograph, and the signature is the whole of
    /// what `heif_brand` reads.
    fn ftyp(brand: &[u8; 4]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 0, 24];
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(brand);
        bytes.extend_from_slice(b"\0\0\0\0mif1heic");
        bytes
    }

    fn jpeg() -> Vec<u8> {
        let img = GrayImage::from_pixel(16, 16, image::Luma([200u8]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageLuma8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        bytes
    }

    #[test]
    fn the_brands_an_iphone_writes_are_recognised() {
        // What Apple's encoder stamps, and what a non-Apple encoder writing the
        // same pictures stamps instead.
        for brand in [b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx"] {
            assert_eq!(
                heif_brand(&ftyp(brand)),
                Some("HEIC"),
                "{} was not recognised",
                String::from_utf8_lossy(brand)
            );
        }
        assert_eq!(heif_brand(&ftyp(b"mif1")), Some("HEIF"));
        assert_eq!(heif_brand(&ftyp(b"avif")), Some("AVIF"));
    }

    /// The signature is four bytes at offset eight, and a container that is not
    /// a picture wears the same `ftyp` box. Reading only "does it say ftyp"
    /// would claim every MP4 on the phone.
    #[test]
    fn a_video_container_is_not_claimed_as_a_photograph() {
        assert_eq!(heif_brand(&ftyp(b"isom")), None, "an MP4 is not a photo");
        assert_eq!(heif_brand(&ftyp(b"qt  ")), None, "a MOV is not a photo");
        assert_eq!(heif_brand(&jpeg()), None);
        assert_eq!(heif_brand(b"short"), None);
        assert_eq!(heif_brand(&[]), None);
    }

    /// With no decoder, HEIC must be refused **by name**. "Unsupported input"
    /// sends a reader looking for a format nobody recognised; this file is a
    /// format everybody recognises and this build cannot open.
    #[test]
    fn heic_with_no_decoder_is_refused_by_name() {
        let err = decode_bytes_using(&ftyp(b"heic"), &Limits::default(), None)
            .expect_err("no decoder can open this");
        assert!(
            matches!(err, ScanError::NeedsPlatformDecoder { format: "HEIC", .. }),
            "got {err:?}"
        );
        let message = err.to_string();
        assert!(message.contains("HEIC"), "must name the format: {message}");
        assert!(
            message.contains("ImageIO") && message.contains("BitmapFactory"),
            "must say what would open it: {message}"
        );
    }

    /// A decoder that returns a fixed image, standing in for the platform.
    struct StubDecoder(u32, u32);
    impl DecodeImage for StubDecoder {
        fn handles(&self, bytes: &[u8]) -> bool {
            heif_brand(bytes).is_some()
        }
        fn decode(&self, _bytes: &[u8]) -> Result<GrayImage> {
            Ok(GrayImage::from_pixel(self.0, self.1, image::Luma([128u8])))
        }
    }

    #[test]
    fn heic_with_a_decoder_is_read_through_the_seam() {
        let img = decode_bytes_using(
            &ftyp(b"heic"),
            &Limits::default(),
            Some(&StubDecoder(40, 30)),
        )
        .expect("the platform decoder opens it");
        assert_eq!(img.dimensions(), (40, 30));
    }

    /// A JPEG is decoded by this crate on every surface, and a **broken** JPEG
    /// is a broken JPEG rather than something to offer round. Handing it on
    /// would turn one clear error into two vague ones, and would let the same
    /// file read differently on two platforms.
    #[test]
    fn a_damaged_jpeg_is_not_offered_to_the_platform_decoder() {
        let mut damaged = jpeg();
        let n = damaged.len();
        damaged.truncate(n / 2);
        // A decoder that would claim anything, to prove it never gets asked.
        struct ClaimsEverything;
        impl DecodeImage for ClaimsEverything {
            fn handles(&self, _bytes: &[u8]) -> bool {
                true
            }
            fn decode(&self, _bytes: &[u8]) -> Result<GrayImage> {
                Ok(GrayImage::from_pixel(1, 1, image::Luma([0u8])))
            }
        }
        let err = decode_bytes_using(&damaged, &Limits::default(), Some(&ClaimsEverything))
            .expect_err("a truncated JPEG must fail as a JPEG");
        assert!(matches!(err, ScanError::Decode(_)), "got {err:?}");
    }

    /// The pixel ceiling is the whole reason this module is the choke point,
    /// and a platform decoder walks straight past the header check that
    /// enforces it. So it is enforced again on what the decoder returned.
    #[test]
    fn a_platform_decoder_is_still_bounded_by_the_pixel_ceiling() {
        let tight = Limits {
            pixels_per_page: 1_000,
            ..Limits::default()
        };
        let err = decode_bytes_using(&ftyp(b"heic"), &tight, Some(&StubDecoder(2000, 2000)))
            .expect_err("4 megapixels is past a 1,000 pixel ceiling");
        assert!(matches!(err, ScanError::TooLarge(_)), "got {err:?}");
        // And the same decoder inside the ceiling is allowed, or the test only
        // proves that something failed.
        decode_bytes_using(&ftyp(b"heic"), &tight, Some(&StubDecoder(20, 20)))
            .expect("400 pixels is within it");
    }

    /// The refusal names a remedy *this* target can follow.
    ///
    /// Two targets, two answers, because a refusal pointing at somebody else's
    /// platform is the shape `error.rs` already shipped once: "supply one with
    /// `Options::pdf_password`", naming a field that did not exist. A browser
    /// cannot be handed ImageIO.
    ///
    /// Asserted on substance rather than on the sentence, so rewording does not
    /// fail it — but swapping which target gets which remedy does. Verified by
    /// swapping the two `cfg`s and watching this go red.
    #[test]
    fn the_heic_refusal_names_a_remedy_this_target_can_follow() {
        // A minimal ISO base-media header carrying an `heic` brand.
        // `heif_brand` reads the `ftyp` box and nothing else, so this routes.
        let mut heic = vec![0u8, 0, 0, 24];
        heic.extend_from_slice(b"ftypheic");
        heic.extend_from_slice(&[0u8; 12]);

        let err = decode_bytes_using(&heic, &crate::limits::Limits::default(), None)
            .expect_err("HEIC without a decoder must refuse");
        let why = format!("{err}");

        if cfg!(target_arch = "wasm32") {
            assert!(why.contains("JPEG"), "a browser needs a remedy it can follow: {why}");
            assert!(!why.contains("ImageIO"), "a browser cannot use ImageIO: {why}");
        } else {
            assert!(
                why.contains("ImageIO") || why.contains("BitmapFactory"),
                "a platform build should name its decoders: {why}"
            );
        }
    }

}
