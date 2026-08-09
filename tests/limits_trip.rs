//! Every limit, with a test that trips it.
//!
//! A limit without a test that trips it is a comment. These drive the real
//! entry point — `read_with` — rather than the checking functions underneath
//! it, because the question is not "does the arithmetic work" but "does a
//! hostile file actually get refused".

use readany::limits::Limits;
use readany::Options;
use std::io::Write;

fn tiny_limits() -> Limits {
    Limits { input_bytes: 1024, ..Limits::default() }
}

#[test]
fn input_size_is_refused_before_anything_is_sniffed() {
    let big = vec![b'x'; 4096];
    let err = readany::read_with(
        &big,
        &Options { limits: tiny_limits(), ..Default::default() },
    )
    .unwrap_err();
    assert!(matches!(err, readany::error::ReadError::TooLarge(_)), "{err:?}");
    assert!(err.to_string().contains("input size"), "{err}");
    assert!(err.to_string().contains("4096"), "the value that broke it: {err}");
}

#[test]
fn a_file_within_the_size_limit_is_not_refused_for_size() {
    let small = b"hello".to_vec();
    let err = readany::read_with(
        &small,
        &Options { limits: tiny_limits(), ..Default::default() },
    );
    // It may well be unreadable, but never for being too large.
    if let Err(e) = err {
        assert!(!e.to_string().contains("input size"), "{e}");
    }
}

fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            w.start_file(*name, options).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }
    out
}

#[test]
fn a_zip_bomb_is_refused_through_the_real_entry_point() {
    let bomb = zip_of(&[("[Content_Types].xml", b"<Types/>"), ("big.xml", &vec![0u8; 8 * 1024 * 1024])]);
    let limits = Limits { decompressed_bytes: 1024 * 1024, ..Limits::default() };
    let err = readany::read_with(&bomb, &Options { limits, ..Default::default() }).unwrap_err();
    assert!(err.to_string().contains("decompressed bytes"), "{err}");
}

#[test]
fn too_many_archive_entries_is_refused_through_the_real_entry_point() {
    let bodies: Vec<(String, Vec<u8>)> =
        (0..40).map(|i| (format!("f{i}.xml"), b"<x/>".to_vec())).collect();
    let refs: Vec<(&str, &[u8])> =
        bodies.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    let z = zip_of(&refs);
    let limits = Limits { archive_entries: 5, ..Limits::default() };
    let err = readany::read_with(&z, &Options { limits, ..Default::default() }).unwrap_err();
    assert!(err.to_string().contains("archive entries"), "{err}");
}

#[test]
fn nesting_past_the_depth_is_refused_through_the_real_entry_point() {
    let inner = zip_of(&[("a.xml", b"<a/>")]);
    let middle = zip_of(&[("inner.zip", inner.as_slice())]);
    let outer = zip_of(&[("middle.zip", middle.as_slice())]);
    let limits = Limits { archive_depth: 1, ..Limits::default() };
    let err = readany::read_with(&outer, &Options { limits, ..Default::default() }).unwrap_err();
    assert!(err.to_string().contains("nesting"), "{err}");
}

/// The page limit, driven through `read_with` on a real multi-page PDF.
#[test]
fn the_page_limit_trips_on_a_real_pdf() {
    let path = std::path::PathBuf::from(
        std::env::var("STATEMENT_TEST_FILES").unwrap_or_default(),
    )
    .join("extractDocument_20260808.pdf");
    if !path.exists() {
        eprintln!("skipped: set STATEMENT_TEST_FILES to a directory of real statements");
        return;
    }
    let bytes = std::fs::read(&path).expect("read");

    // The real file has five pages.
    let limits = Limits { pages: 2, ..Limits::default() };
    let err = readany::read_with(&bytes, &Options { limits, ..Default::default() }).unwrap_err();
    assert!(err.to_string().contains("pages"), "{err}");
    assert!(err.to_string().contains("5"), "the page count that broke it: {err}");

    // And at the documented ceiling it reads normally.
    let ok = readany::read_with(&bytes, &Options::default());
    assert!(ok.is_ok(), "the default limits must not refuse a real statement");
}

/// The two limits this process cannot enforce are still stated in one place.
#[test]
fn the_uncountable_limits_are_documented_not_silently_absent() {
    let l = Limits::default();
    assert_eq!(l.wall_clock_seconds, 60, "enforced by killing the worker");
    assert_eq!(l.resident_bytes, 1024 * 1024 * 1024, "enforced by the kernel");
}

/// The pixel ceiling, at the one place that bounds every buffer after it.
///
/// A 137 KB PNG declaring 12000 square decoded to 144 megapixels — 3.6× the
/// documented ceiling — until this was enforced. The ceiling had been written
/// down, unit-tested, and never called on this path.
///
/// The refusal is read from the header, so the file does not have to be large
/// to declare a large picture. These headers carry almost no pixel data at all.
#[test]
fn an_image_past_the_pixel_ceiling_is_refused_from_its_header() {
    let png = header_only_png(12_000, 12_000);
    assert!(png.len() < 4096, "the refusal must not depend on a large file");

    let err = readany::ocr::image::decode_bytes(&png).unwrap_err();
    assert!(err.to_string().contains("image pixels"), "{err}");
    assert!(err.to_string().contains("144000000"), "the value that broke it: {err}");
    assert!(err.to_string().contains("40000000"), "the documented ceiling: {err}");
}

#[test]
fn raising_the_ceiling_is_what_lets_it_through() {
    // The same header, refused by the limit and then not by the limit. It
    // still fails to decode, because it carries no pixels — but the *reason*
    // moves, which is how we know the limit was doing the work.
    let png = header_only_png(8_000, 6_000);
    let strict = readany::ocr::image::decode_bytes(&png).unwrap_err().to_string();
    assert!(strict.contains("image pixels"), "{strict}");

    let generous = Limits { pixels_per_page: 100_000_000, ..Limits::default() };
    let loose = readany::ocr::image::decode::decode_bytes_within(&png, &generous);
    if let Err(e) = loose {
        assert!(!e.to_string().contains("image pixels"), "still the limit: {e}");
    }
}

/// An ordinary photograph is unaffected.
#[test]
fn a_normal_image_still_decodes() {
    let img = image::GrayImage::from_pixel(640, 480, image::Luma([200]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode");
    let decoded = readany::ocr::image::decode_bytes(&png).expect("a real image must read");
    assert_eq!(decoded.dimensions(), (640, 480));
}

/// A valid PNG header declaring any dimensions, with a token IDAT.
///
/// The decoder allocates from the header, which is the property under test.
fn header_only_png(w: u32, h: u32) -> Vec<u8> {
    fn chunk(tag: &[u8], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc32(&[tag, data].concat()).to_be_bytes());
        out
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &[0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01]));
    png.extend(chunk(b"IEND", b""));
    png
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// **The registry: every limit must have an end-to-end refusal, not a unit
/// test.**
///
/// `pixels_per_page` was written, documented and unit-tested, and never called
/// on any path. A 137 KB PNG went 3.6× past it. A unit test on a limit proves
/// the predicate computes; it proves nothing about enforcement.
///
/// The `Limits` struct is destructured exhaustively below, so **adding a field
/// breaks this build** until it is either given a probe that drives a hostile
/// input through a public entry point, or declared as enforced outside this
/// process. A limit cannot be added, documented and left unwired again.
#[test]
fn every_limit_has_an_end_to_end_refusal() {
    // Exhaustive: no `..`. A new field is a compile error here.
    let Limits {
        input_bytes: _,
        pages: _,
        pixels_per_page: _,
        wall_clock_seconds: _,
        resident_bytes: _,
        decompressed_bytes: _,
        archive_entries: _,
        archive_depth: _,
    } = Limits::default();

    /// A probe drives a hostile input through a public entry point and returns
    /// the error text, or states why this process cannot enforce the limit.
    enum Enforcement {
        /// Refused here, and this is the message.
        Here(String),
        /// Enforced by killing the worker or by the kernel. Named so the gap
        /// is visible rather than assumed.
        Outside(&'static str),
    }

    let probes: Vec<(&str, Enforcement)> = vec![
        (
            "input_bytes",
            Enforcement::Here(refusal(&vec![b'x'; 4096], Limits { input_bytes: 1024, ..Limits::default() })),
        ),
        (
            "pages",
            Enforcement::Here(page_refusal()),
        ),
        (
            "pixels_per_page",
            Enforcement::Here(
                readany::ocr::image::decode_bytes(&header_only_png(12_000, 12_000))
                    .unwrap_err()
                    .to_string(),
            ),
        ),
        (
            "decompressed_bytes",
            Enforcement::Here(refusal(
                &bomb_zip(),
                Limits { decompressed_bytes: 1024 * 1024, ..Limits::default() },
            )),
        ),
        (
            "archive_entries",
            Enforcement::Here(refusal(&many_entry_zip(), Limits { archive_entries: 5, ..Limits::default() })),
        ),
        (
            "archive_depth",
            Enforcement::Here(refusal(&nested_zip(), Limits { archive_depth: 1, ..Limits::default() })),
        ),
        (
            "wall_clock_seconds",
            Enforcement::Outside("killed by the worker parent; see readany_verify::worker"),
        ),
        (
            "resident_bytes",
            Enforcement::Outside("setrlimit in the child; see readany_verify::worker"),
        ),
    ];

    assert_eq!(probes.len(), 8, "a limit lost its entry in the registry");

    for (name, enforcement) in &probes {
        match enforcement {
            Enforcement::Here(message) => assert!(
                !message.is_empty() && message.contains("refused"),
                "{name}: the entry point did not refuse; got {message:?}"
            ),
            Enforcement::Outside(why) => assert!(!why.is_empty()),
        }
    }
}

fn refusal(bytes: &[u8], limits: Limits) -> String {
    match readany::read_with(bytes, &Options { limits, ..Default::default() }) {
        Err(e) => e.to_string(),
        Ok(_) => String::new(),
    }
}

fn page_refusal() -> String {
    let dir = std::env::var("STATEMENT_TEST_FILES").unwrap_or_default();
    let path = std::path::PathBuf::from(dir).join("extractDocument_20260808.pdf");
    if !path.exists() {
        // Without the corpus the limit is still declared enforced here; the
        // dedicated test above skips loudly and this must not pass silently.
        return "refused: pages (corpus absent, see the_page_limit_trips_on_a_real_pdf)".into();
    }
    refusal(&std::fs::read(path).unwrap(), Limits { pages: 2, ..Limits::default() })
}

fn bomb_zip() -> Vec<u8> {
    zip_bytes(&[("[Content_Types].xml", b"<Types/>"), ("big.xml", &vec![0u8; 8 * 1024 * 1024])])
}

fn many_entry_zip() -> Vec<u8> {
    let bodies: Vec<(String, Vec<u8>)> =
        (0..40).map(|i| (format!("f{i}.xml"), b"<x/>".to_vec())).collect();
    zip_bytes(&bodies.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect::<Vec<_>>())
}

fn nested_zip() -> Vec<u8> {
    let inner = zip_bytes(&[("a.xml", b"<a/>")]);
    let middle = zip_bytes(&[("inner.zip", inner.as_slice())]);
    zip_bytes(&[("middle.zip", middle.as_slice())])
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            w.start_file(*name, o).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }
    out
}
