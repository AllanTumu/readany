//! Refusing an archive before it is unpacked.
//!
//! Word, Excel, PowerPoint, OpenDocument and EPUB are all zip files, so every
//! Office format shares one attack surface: a small archive that expands to
//! gigabytes.
//!
//! Measured against this library before this module existed: a **4.6 MB
//! `.xlsx`**, structurally valid and sniffing correctly as a spreadsheet,
//! expanded to **1.04 GB** and was read to completion in 3.0 seconds using
//! 1.075 GB of resident memory. Memory tracked the decompressed size almost
//! exactly, so at the same ratio a 50 MB upload — the largest we accept —
//! reaches roughly 11 GB, and deflate permits ratios four times higher again.
//!
//! ## The header is written by the attacker
//!
//! A zip's central directory declares each entry's uncompressed size. That
//! number is a claim, not a measurement, and it can be a lie in either
//! direction. So it is used for one thing only: a **cheap early refusal** when
//! the file admits to being too large. It is never used to conclude that a
//! file is small enough.
//!
//! The real cap is counted **as the bytes are produced**, by decompressing
//! into a sink that stops at the limit — see [`check`]. That costs a bounded
//! amount of work: the guard never produces more than the cap before refusing.
//!
//! ## Why this lives here and not in the reader
//!
//! The decompression that matters happens inside `anydoc`, which this crate
//! consumes and does not own. The cap therefore runs as a **pre-flight**: the
//! archive is validated here and only then handed on. The consequence is that
//! a permitted archive is decompressed twice, once to check and once to read,
//! and the check half is bounded by the cap. That is the price of enforcing a
//! limit at a boundary that is not where the work happens, and it is worth
//! stating rather than discovering.
//!
//! This module exists to be pointed at hostile input, so the panicking forms
//! are denied rather than trusted to review. See `docs/security.md`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use crate::error::{ReadError, Result};
use crate::limits::{Exceeded, Limits};
use std::io::Read;

/// The first bytes of a zip local file header.
const ZIP_MAGIC: [u8; 2] = *b"PK";

/// Is this plausibly a zip archive?
pub fn looks_like_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes.starts_with(&ZIP_MAGIC)
}

/// Refuse an archive that would decompress past the limits.
///
/// Checks, in increasing order of cost:
///
/// 1. the number of entries, read from the directory;
/// 2. the declared uncompressed total, as an early refusal only;
/// 3. the **actual** decompressed size, counted as it is produced and stopped
///    at the cap;
/// 4. nested archives, recursively, to [`Limits::archive_depth`].
///
/// Returns `Ok(())` for anything that is not a zip at all: this is a guard on
/// one format, not a gate on every file.
pub fn check(bytes: &[u8], limits: &Limits) -> Result<()> {
    if !looks_like_zip(bytes) {
        return Ok(());
    }
    check_at_depth(bytes, limits, 0)
}

fn check_at_depth(bytes: &[u8], limits: &Limits, depth: u32) -> Result<()> {
    if depth > limits.archive_depth {
        return Err(exceeded(Exceeded::new(
            "archive nesting depth",
            limits.archive_depth as u64,
            depth as u64,
        )));
    }

    let reader = std::io::Cursor::new(bytes);
    let mut zip = match zip::ZipArchive::new(reader) {
        Ok(z) => z,
        // Not a readable zip. Refusing here would reject files the reader
        // behind us may still handle, and this module's job is to stop bombs,
        // not to decide what is readable.
        Err(_) => return Ok(()),
    };

    let entries = zip.len() as u64;
    if entries > limits.archive_entries {
        return Err(exceeded(Exceeded::new(
            "archive entries",
            limits.archive_entries,
            entries,
        )));
    }

    // The declared total, as an early refusal. A liar under-declares, so this
    // catches only the honest and the careless — which is exactly why it is
    // not the enforcement.
    let mut declared: u64 = 0;
    for i in 0..zip.len() {
        let Ok(entry) = zip.by_index_raw(i) else { continue };
        declared = declared.saturating_add(entry.size());
    }
    if declared > limits.decompressed_bytes {
        return Err(exceeded(Exceeded::new(
            "declared decompressed bytes",
            limits.decompressed_bytes,
            declared,
        )));
    }

    // The measurement. Every entry is decompressed into a counter that stops
    // at the cap, so the guard's own cost is bounded by the cap however large
    // the archive claims, or fails to claim, to be.
    let mut produced: u64 = 0;
    for i in 0..zip.len() {
        let mut entry = match zip.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let remaining = limits.decompressed_bytes.saturating_sub(produced);
        // One byte past the budget is enough to know the budget is gone.
        let mut sink = Counting { written: 0, cap: remaining.saturating_add(1) };
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let n = match entry.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(_) => break,
            };
            if !sink.write(n as u64) {
                return Err(exceeded(Exceeded::unbounded(
                    "decompressed bytes",
                    limits.decompressed_bytes,
                )));
            }
        }
        produced = produced.saturating_add(sink.written);
        if produced > limits.decompressed_bytes {
            return Err(exceeded(Exceeded::unbounded(
                "decompressed bytes",
                limits.decompressed_bytes,
            )));
        }
    }

    // Archives inside archives. Recursion is **not** stopped at the permitted
    // depth: it goes one level further, because an archive at depth `limit+1`
    // can only be refused by being looked at. The check at the top of this
    // function is what stops it, so the recursion is bounded by `limit + 1`
    // however deeply the file nests.
    //
    // Reading a nested entry costs a second decompression of that entry alone,
    // which the cap above has already bounded.
    {
        for i in 0..zip.len() {
            let mut entry = match zip.by_index(i) {
                Ok(e) => e,
                Err(_) => continue,
            };
            if entry.is_dir() || entry.size() > limits.decompressed_bytes {
                continue;
            }
            let mut head = [0u8; 4];
            if entry.read_exact(&mut head).is_err() || !looks_like_zip(&head) {
                continue;
            }
            let mut inner = head.to_vec();
            let budget = limits.decompressed_bytes.min(entry.size()) as usize;
            if entry.take(budget as u64).read_to_end(&mut inner).is_err() {
                continue;
            }
            check_at_depth(&inner, limits, depth.saturating_add(1))?;
        }
    }

    Ok(())
}

/// A sink that counts and refuses to hold anything.
///
/// The bytes are never kept. Counting them is the whole point — a guard that
/// buffered what it was measuring would be the bomb it is trying to stop.
struct Counting {
    written: u64,
    cap: u64,
}

impl Counting {
    /// Returns false once the cap is passed.
    fn write(&mut self, n: u64) -> bool {
        self.written = self.written.saturating_add(n);
        self.written <= self.cap
    }
}

fn exceeded(e: Exceeded) -> ReadError {
    ReadError::TooLarge(e)
}

#[cfg(test)]
mod tests {
    // See the note on the same allow in `route`: denying these in tests buys
    // noise, not safety.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;
    use std::io::Write;

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
            let options: zip::write::FileOptions<'_, ()> =
                zip::write::FileOptions::default()
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
    fn an_ordinary_archive_passes() {
        let z = zip_of(&[("a.xml", b"<a/>"), ("b.xml", b"<b/>")]);
        assert!(check(&z, &Limits::default()).is_ok());
    }

    #[test]
    fn something_that_is_not_a_zip_is_not_this_module_s_business() {
        assert!(check(b"%PDF-1.4", &Limits::default()).is_ok());
        assert!(check(b"", &Limits::default()).is_ok());
        assert!(check(b"PK", &Limits::default()).is_ok());
    }

    /// The measured vulnerability, as a test.
    #[test]
    fn a_bomb_is_refused_by_measurement_not_by_its_own_declaration() {
        // 8 MB of zeros compresses to a few kilobytes.
        let bomb = zip_of(&[("sheet.xml", &vec![0u8; 8 * 1024 * 1024])]);
        assert!(bomb.len() < 64 * 1024, "the bomb must actually be small");

        let tight = Limits { decompressed_bytes: 1024 * 1024, ..Limits::default() };
        let err = check(&bomb, &tight).unwrap_err();
        assert!(matches!(err, ReadError::TooLarge(_)), "got {err:?}");
        assert!(err.to_string().contains("decompressed bytes"));

        // And the same archive is fine when the limit allows it.
        assert!(check(&bomb, &Limits::default()).is_ok());
    }

    /// **The test that decides whether this module works.**
    ///
    /// An attacker writes the header, so a declared size is a claim and not a
    /// measurement. Here every declared uncompressed size — in the central
    /// directory *and* in the local headers — is rewritten to a small lie, so
    /// the cheap check passes and only counting the produced bytes can catch
    /// it.
    ///
    /// Measured on the real 4.6 MB / 1.04 GB spreadsheet: refused in 0.05 s
    /// using 11.5 MB of resident memory, because the bytes are counted and
    /// discarded rather than held.
    #[test]
    fn a_bomb_that_lies_about_its_size_is_still_caught() {
        let mut z = zip_of(&[("sheet.xml", &vec![0u8; 8 * 1024 * 1024])]);

        // Rewrite the uncompressed size to 4096 everywhere it is stated.
        let lie = 4096u32.to_le_bytes();
        let mut patched = 0;
        for (magic, offset) in [
            (b"PK\x01\x02".as_slice(), 24usize),
            (b"PK\x03\x04".as_slice(), 22usize),
        ] {
            let mut i = 0;
            while let Some(found) = find(&z, magic, i) {
                if found + offset + 4 <= z.len() {
                    z[found + offset..found + offset + 4].copy_from_slice(&lie);
                    patched += 1;
                }
                i = found + 4;
            }
        }
        assert!(patched >= 2, "the lie must be written into both headers");

        let tight = Limits { decompressed_bytes: 1024 * 1024, ..Limits::default() };
        let err = check(&z, &tight).unwrap_err();
        assert!(err.to_string().contains("decompressed bytes"), "{err}");
        // And it is the *measured* refusal, not the declared one: the declared
        // total is now a few kilobytes and would have sailed through.
        assert!(
            !err.to_string().contains("declared"),
            "caught by the attacker's own number rather than by measurement: {err}"
        );
    }

    fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
        haystack
            .get(from..)?
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|p| p + from)
    }

    #[test]
    fn too_many_entries_is_refused_before_anything_is_decompressed() {
        let bodies: Vec<(String, Vec<u8>)> =
            (0..50).map(|i| (format!("f{i}.xml"), b"<x/>".to_vec())).collect();
        let refs: Vec<(&str, &[u8])> =
            bodies.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
        let z = zip_of(&refs);

        let tight = Limits { archive_entries: 10, ..Limits::default() };
        let err = check(&z, &tight).unwrap_err();
        assert!(err.to_string().contains("archive entries"), "{err}");
    }

    #[test]
    fn an_archive_nested_past_the_depth_is_refused() {
        let inner = zip_of(&[("payload.xml", &vec![0u8; 4 * 1024 * 1024])]);
        let middle = zip_of(&[("inner.zip", inner.as_slice())]);
        let outer = zip_of(&[("middle.zip", middle.as_slice())]);

        // The payload is well inside the byte cap; only the nesting is at issue.
        let shallow = Limits { archive_depth: 1, ..Limits::default() };
        let err = check(&outer, &shallow).unwrap_err();
        assert!(err.to_string().contains("nesting"), "{err}");

        // At the documented depth of 2 it is permitted.
        assert!(check(&outer, &Limits::default()).is_ok());
    }

    /// A bomb hidden one level down is still a bomb.
    #[test]
    fn a_nested_bomb_is_counted_too() {
        let inner = zip_of(&[("sheet.xml", &vec![0u8; 8 * 1024 * 1024])]);
        let outer = zip_of(&[("inner.zip", inner.as_slice())]);
        let tight = Limits { decompressed_bytes: 1024 * 1024, ..Limits::default() };
        let err = check(&outer, &tight).unwrap_err();
        assert!(err.to_string().contains("decompressed bytes"), "{err}");
    }

    #[test]
    fn the_guard_never_holds_what_it_measures() {
        // A counter that stops one byte past its budget.
        let mut c = Counting { written: 0, cap: 4 };
        assert!(c.write(3));
        assert!(c.write(1));
        assert!(!c.write(1));
        assert_eq!(c.written, 5);
    }
}
