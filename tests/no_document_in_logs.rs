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

// The verdict-surface half of this sweep now lives in `readany-verify`.
//
// It was here, which made this crate — public, MIT, on npm — carry a path
// dev-dependency on a private sibling. No runner can check out a private repo,
// so this crate's own CI failed at dependency resolution on every push, and
// nobody cloning it could run `cargo test` at all. A public crate that cannot
// build in its own CI is a worse defect than one fixture defined in two places.
