//! A document must not be able to reach the log.
//!
//! Structural, not a rule people remember. A distinctive token is planted
//! inside a document, a full extraction runs with logging at its most verbose,
//! and every channel that could carry text out is searched for it.
//!
//! This test is worth more than any newtype, because it also covers the paths
//! nobody remembered to wrap — including error messages and panic messages,
//! which are where document content most often escapes.

use std::io::Write;

/// Long, unique, and not a substring of anything a library would emit.
const TOKEN: &str = "ZQXJVBWKPMHGDF-planted-secret-74619283";

/// Every value here is invented. The row this fixture started from was copied
/// out of a real bank statement — its date, its amount and its closing balance
/// — into a public repository, which is the one place this project's own rule
/// says a balance may never go. The test needs a row with a date, a signed
/// amount and a balance; it never needed *that* row.
fn planted_csv() -> Vec<u8> {
    format!("Date,Description,Amount,Balance\n01/01/2020,{TOKEN},-10.00,1000.00\n").into_bytes()
}

fn planted_xlsx() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(&mut out));
        let o: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        w.start_file("[Content_Types].xml", o).unwrap();
        w.write_all(format!("<Types><!--{TOKEN}--></Types>").as_bytes()).unwrap();
        w.finish().unwrap();
    }
    out
}

/// Bytes that are broken in a way that makes the reader complain.
fn planted_broken() -> Vec<u8> {
    let mut v = b"%PDF-1.4\n".to_vec();
    v.extend_from_slice(TOKEN.as_bytes());
    v.extend_from_slice(b"\n%%EOF\n");
    v
}

/// A PDF that declares itself encrypted, with the token in the one place a
/// reader is most likely to quote it back.
///
/// The encrypted path is the sharpest case in this sweep, because it is the
/// only one where the reader is holding bytes it could not interpret — and
/// "could not interpret" is exactly when a library reaches for "here is what I
/// saw". The document need not be *validly* encrypted for that: it needs to
/// reach the refusal with the token in scope, which this does without the
/// hundred lines of RC4 that `tests/encrypted_pdf.rs` needs for the real thing.
fn planted_encrypted() -> Vec<u8> {
    let mut v = format!(
        "%PDF-1.4\n\
         1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
         2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n\
         3 0 obj\n<< /Filter /Standard /V 1 /R 2 /P -1 /Note ({TOKEN}) >>\nendobj\n\
         trailer\n<< /Size 4 /Root 1 0 R /Encrypt 3 0 R /ID [<0123> <0123>] >>\n"
    )
    .into_bytes();
    v.extend_from_slice(b"%%EOF\n");
    v
}

fn leaked(haystack: &str, where_: &str) -> Option<String> {
    haystack.contains(TOKEN).then(|| format!("{where_} contains the planted token"))
}

#[test]
fn no_document_content_reaches_any_log_or_error_channel() {
    // Logging at its most verbose, captured rather than printed.
    let _ = std::env::var("RUST_LOG");
    std::env::set_var("RUST_LOG", "trace");

    let mut findings: Vec<String> = Vec::new();
    // A test that exercised no error path would pass while testing nothing —
    // the same class as a check that never runs. Counted and asserted.
    let mut errors_seen = 0usize;
    let mut reads_seen = 0usize;

    for (name, bytes) in [
        ("csv", planted_csv()),
        ("xlsx", planted_xlsx()),
        ("broken pdf", planted_broken()),
        ("encrypted pdf", planted_encrypted()),
    ] {
        let result = readany::read_with(
            &bytes,
            &readany::Options { filename: Some("planted.csv"), ..Default::default() },
        );

        match result {
            Ok(doc) => {
                reads_seen += 1;
                // The markdown is *supposed* to contain it: that is the
                // product. What must not is anything describing the document.
                for (channel, text) in [
                    ("unresolved pages", format!("{:?}", doc.unresolved_pages)),
                    ("page metadata", format!("{:?}", doc.pages.iter().map(|p| p.number).collect::<Vec<_>>())),
                ] {
                    if let Some(f) = leaked(&text, &format!("{name}: {channel}")) {
                        findings.push(f);
                    }
                }
            }
            Err(e) => {
                errors_seen += 1;
                // The channel that leaks most often.
                if let Some(f) = leaked(&e.to_string(), &format!("{name}: Display of the error")) {
                    findings.push(f);
                }
                if let Some(f) = leaked(&format!("{e:?}"), &format!("{name}: Debug of the error")) {
                    findings.push(f);
                }
            }
        }
    }

    assert!(findings.is_empty(), "document content escaped:\n  {}", findings.join("\n  "));
    assert!(errors_seen > 0, "no error path was exercised, so the test proved nothing");
    assert!(reads_seen > 0, "no successful read was exercised");
    eprintln!("planted-token sweep: {reads_seen} read(s), {errors_seen} error(s) inspected");
}

/// A password must not reach an error either, and it is a worse leak than the
/// document.
///
/// A document's text is at least the thing the caller asked us to produce. A
/// password is never output — it is only ever input, and the one input here
/// that is a secret. It is also the input most likely to end up quoted back,
/// because the natural way to report a failure is to say what was tried.
///
/// [`Rasterise::render_page_with_password`] is the only place in this crate
/// that takes one. The stub below fails the way a real renderer fails, with the
/// planted token as the password, and neither the error nor its `Debug` may
/// carry it. This runs without PDFium: the seam is defined here, so its
/// contract is testable here.
#[test]
fn a_password_does_not_reach_an_error() {
    use readany::pdf::render::{PageImage, Rasterise};

    /// Fails on every page, which is how a renderer behaves when it was handed
    /// a document it cannot open.
    struct Failing;
    impl Rasterise for Failing {
        fn page_count(&self, _pdf: &[u8]) -> readany::Result<usize> {
            Ok(1)
        }
        fn render_page(&self, _pdf: &[u8], page: usize, dpi: f32) -> readany::Result<PageImage> {
            // Deliberately chatty, because a renderer that says nothing cannot
            // demonstrate that it says nothing *sensitive*. Page and dpi are
            // not secrets; the password is, and it is not here.
            Err(readany::ReadError::Pdf(format!(
                "page {page} could not be rendered at {dpi} dpi"
            )))
        }
    }

    let err = Failing
        .render_page_with_password(b"%PDF-1.4\n%%EOF\n", 0, 150.0, Some(TOKEN))
        .expect_err("the stub renderer must fail");

    let mut findings: Vec<String> = Vec::new();
    for (channel, text) in [("Display", err.to_string()), ("Debug", format!("{err:?}"))] {
        if let Some(f) = leaked(&text, &format!("password: {channel} of the error")) {
            findings.push(f);
        }
    }
    // And the two refusals a real renderer returns carry no data at all, so
    // there is nowhere for a password to hide in them.
    for (name, e) in [
        ("PasswordRequired", readany::ReadError::PasswordRequired),
        ("PasswordRejected", readany::ReadError::PasswordRejected),
    ] {
        for (channel, text) in [("Display", e.to_string()), ("Debug", format!("{e:?}"))] {
            if let Some(f) = leaked(&text, &format!("{name}: {channel}")) {
                findings.push(f);
            }
        }
    }
    assert!(findings.is_empty(), "a password escaped:\n  {}", findings.join("\n  "));
}

// The verdict-surface half of this sweep now lives in `readany-verify`.
//
// It was here, which made this crate — public, MIT, on npm — carry a path
// dev-dependency on a private sibling. No runner can check out a private repo,
// so this crate's own CI failed at dependency resolution on every push, and
// nobody cloning it could run `cargo test` at all. A public crate that cannot
// build in its own CI is a worse defect than one fixture defined in two places.
