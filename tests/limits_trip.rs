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
