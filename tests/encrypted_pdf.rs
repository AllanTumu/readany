//! What this library does with an encrypted PDF, pinned.
//!
//! Two facts, and only one of them was ever written down anywhere.
//!
//! **A PDF encrypted with an empty user password reads completely, today, with
//! no option set.** That is the ordinary protected bank statement: an owner
//! password restricts printing and copying, the user password is empty, and
//! every reader opens it silently. It comes free from `pdf-inspector`, which
//! defaults the password to `""`. Nothing in this repository said so, and an
//! undocumented behaviour is one that can be removed by accident — so the test
//! below asserts the markdown is *identical* to the same document unencrypted,
//! not merely non-empty.
//!
//! **A PDF with a real user password is refused by name.** It used to be
//! refused as `ReadError::Pdf("PDF is encrypted")`, whose message named an
//! `Options::pdf_password` that has never existed, while
//! `ReadError::PasswordRequired` sat in a public enum and was never constructed
//! anywhere. A caller who matched on it to prompt for a password never saw it
//! fire, and a caller who read the message went looking for a field that was
//! not there.
//!
//! # Why the fixtures are built here
//!
//! No encrypted PDF may be committed to this repository: the only encrypted
//! PDFs to hand are one real person's bank statements. So the fixture is
//! generated — RC4-40, the V1/R2 standard security handler, which is the
//! oldest and simplest scheme in the specification and the one a fixture can
//! implement in a page of code. `qpdf`, `pikepdf` and Ghostscript are all
//! absent from this machine and none of them may become a test dependency of a
//! crate that has to build in a bare CI container.
//!
//! Every value printed in the fixture documents is invented.

use md5::{Digest, Md5};

// ---------------------------------------------------------------------------
// The standard security handler, revision 2. ISO 32000-1, algorithms 2 to 5.
// ---------------------------------------------------------------------------

/// The 32-byte padding string every password is padded or truncated to.
/// ISO 32000-1 table 20; it is a constant of the format, not a choice.
const PAD: [u8; 32] = [
    0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01, 0x08,
    0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53, 0x69, 0x7A,
];

/// Revision 2 uses a 40-bit file key. Five bytes, fixed by the revision.
const KEY_LEN: usize = 5;

fn pad_password(pw: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    let b = pw.as_bytes();
    let n = b.len().min(32);
    out[..n].copy_from_slice(&b[..n]);
    out[n..].copy_from_slice(&PAD[..32 - n]);
    out
}

fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = [0; 256];
    for (i, v) in s.iter_mut().enumerate() {
        *v = i as u8;
    }
    let mut j = 0u8;
    for i in 0..256 {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }
    let (mut i, mut j) = (0u8, 0u8);
    data.iter()
        .map(|&b| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(s[i as usize]);
            s.swap(i as usize, j as usize);
            b ^ s[(s[i as usize].wrapping_add(s[j as usize])) as usize]
        })
        .collect()
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut h = Md5::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// Algorithm 3: the `/O` entry, revision 2.
fn owner_entry(owner_pw: &str, user_pw: &str) -> [u8; 32] {
    let padded = pad_password(if owner_pw.is_empty() { user_pw } else { owner_pw });
    let digest = md5(&[&padded]);
    let mut o = [0u8; 32];
    o.copy_from_slice(&rc4(&digest[..KEY_LEN], &pad_password(user_pw)));
    o
}

/// Algorithm 2: the file encryption key, revision 2.
fn file_key(user_pw: &str, o: &[u8; 32], p: i32, id0: &[u8]) -> [u8; KEY_LEN] {
    let digest = md5(&[&pad_password(user_pw), o, &p.to_le_bytes(), id0]);
    let mut k = [0u8; KEY_LEN];
    k.copy_from_slice(&digest[..KEY_LEN]);
    k
}

/// Algorithm 4: the `/U` entry, revision 2.
fn user_entry(key: &[u8; KEY_LEN]) -> [u8; 32] {
    let mut u = [0u8; 32];
    u.copy_from_slice(&rc4(key, &PAD));
    u
}

/// Algorithm 1: the key one object's data is encrypted with.
fn object_key(key: &[u8; KEY_LEN], obj: u32, gen: u16) -> Vec<u8> {
    let o = obj.to_le_bytes();
    let g = gen.to_le_bytes();
    // First `KEY_LEN + 5` bytes, capped at 16 — here always 10.
    md5(&[key, &o[..3], &g[..2]])[..KEY_LEN + 5].to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

// ---------------------------------------------------------------------------
// The fixture documents.
// ---------------------------------------------------------------------------

/// The line the fixture prints. Invented, and deliberately distinctive so a
/// test can tell "read it" from "read something".
const LINE_ONE: &str = "GENERATED FIXTURE STATEMENT";
const LINE_TWO: &str = "Opening balance 4,821.90";

/// Build the one-page fixture.
///
/// `user_pw` distinguishes three genuinely different documents, and the
/// distinction is the point of the whole file:
///
/// - `None` — not encrypted at all.
/// - `Some("")` — encrypted, empty user password. **The common bank statement.**
/// - `Some("hunter2")` — encrypted, a password a reader must be given.
fn fixture(user_pw: Option<&str>, owner_pw: &str) -> Vec<u8> {
    let content = format!(
        "BT /F1 18 Tf 72 720 Td ({LINE_ONE}) Tj ET\n\
         BT /F1 12 Tf 72 690 Td ({LINE_TWO}) Tj ET"
    );

    // Fixed, because the file key derives from it: a fixture whose bytes
    // differed between runs could not pin anything.
    let id0: [u8; 16] = [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0xFE, 0xDC, 0xBA, 0x98, 0x76, 0x54, 0x32,
        0x10,
    ];
    // Permissions. -1 is every bit set: this fixture restricts nothing, because
    // what it is testing is opening the file, not what may be done afterwards.
    let p: i32 = -1;

    let enc = user_pw.map(|pw| {
        let o = owner_entry(owner_pw, pw);
        let key = file_key(pw, &o, p, &id0);
        (o, user_entry(&key), key)
    });

    // Object 5 is the content stream; RC4 is a stream cipher, so `/Length` is
    // the same either way.
    let stream: Vec<u8> = match &enc {
        Some((_, _, key)) => rc4(&object_key(key, 5, 0), content.as_bytes()),
        None => content.into_bytes(),
    };

    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = vec![0; 6];
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let mut obj = |out: &mut Vec<u8>, n: usize, body: &[u8]| {
        offsets[n - 1] = out.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    obj(&mut out, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, 2, b"<< /Type /Pages /Kids [ 3 0 R ] /Count 1 >>");
    obj(
        &mut out,
        3,
        b"<< /Type /Page /Parent 2 0 R /MediaBox [ 0 0 595 842 ] \
          /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>",
    );
    obj(
        &mut out,
        4,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    let mut body: Vec<u8> = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
    body.extend_from_slice(&stream);
    body.extend_from_slice(b"\nendstream");
    obj(&mut out, 5, &body);

    // The encryption dictionary is itself never encrypted.
    if let Some((o, u, _)) = &enc {
        obj(
            &mut out,
            6,
            format!(
                "<< /Filter /Standard /V 1 /R 2 /O <{}> /U <{}> /P {p} >>",
                hex(o),
                hex(u)
            )
            .as_bytes(),
        );
    }

    let count = if enc.is_some() { 6 } else { 5 };
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", count + 1).as_bytes());
    for off in offsets.iter().take(count) {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    let encrypt_ref = if enc.is_some() { "/Encrypt 6 0 R " } else { "" };
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {encrypt_ref}/ID [<{}> <{}>] >>\n\
             startxref\n{xref_at}\n%%EOF\n",
            count + 1,
            hex(&id0),
            hex(&id0)
        )
        .as_bytes(),
    );
    out
}

// ---------------------------------------------------------------------------
// The fixture has to be a real encrypted PDF, or nothing below means anything.
// ---------------------------------------------------------------------------

/// A green run that encrypted nothing would look exactly like a green run that
/// encrypted everything. So: the bytes must differ from the plain twin, must
/// declare an encryption dictionary, and must not carry the plaintext.
#[test]
fn the_fixture_really_is_encrypted() {
    let plain = fixture(None, "");
    let sealed = fixture(Some("hunter2"), "ownersecret");

    assert!(
        plain.windows(LINE_ONE.len()).any(|w| w == LINE_ONE.as_bytes()),
        "the unencrypted twin should carry the line in the clear"
    );
    assert!(
        !sealed.windows(LINE_ONE.len()).any(|w| w == LINE_ONE.as_bytes()),
        "the encrypted fixture still contains its own plaintext, so it is not encrypted"
    );
    assert!(
        sealed.windows(8).any(|w| w == b"/Encrypt"),
        "no encryption dictionary in the trailer"
    );
    assert!(
        sealed.windows(9).any(|w| w == b"/Standard"),
        "not the standard security handler"
    );
}

// ---------------------------------------------------------------------------
// The behaviour that already worked and that nothing wrote down.
// ---------------------------------------------------------------------------

/// The common protected bank statement, and the reason this whole area matters
/// less than it looks: it already works.
///
/// Identical markdown, not merely non-empty markdown. "It read something" would
/// pass on a file that lost half its content.
#[test]
fn an_empty_user_password_reads_exactly_like_the_unencrypted_twin() {
    let plain = readany::read(&fixture(None, "")).expect("plain fixture");
    let sealed = readany::read(&fixture(Some(""), "")).expect("empty-user-password fixture");

    assert!(
        plain.markdown.contains(LINE_ONE),
        "the fixture itself did not read; nothing else here means anything"
    );
    assert_eq!(
        plain.markdown, sealed.markdown,
        "an encrypted PDF with an empty user password must read identically"
    );
    assert_eq!(plain.pages.len(), sealed.pages.len());
    assert!(sealed.is_complete());
    assert!(sealed.unresolved_pages.is_empty());
}

/// An owner password restricts printing and copying, not opening. This is what
/// a bank actually sets, and it must not change the read either.
#[test]
fn an_owner_password_alone_does_not_stop_the_read() {
    let plain = readany::read(&fixture(None, "")).expect("plain fixture");
    let sealed =
        readany::read(&fixture(Some(""), "ownersecret")).expect("owner-password fixture");
    assert_eq!(plain.markdown, sealed.markdown);
    assert!(sealed.markdown.contains(LINE_TWO));
}

/// Inspection has to agree with reading. A plan that says "1 page, no OCR
/// needed" for a file the reader then refuses would be worse than either.
#[test]
fn inspect_agrees_with_read_on_an_empty_user_password() {
    let plan = readany::inspect(&fixture(Some(""), "ownersecret")).expect("inspect");
    match plan.route {
        readany::Route::Pdf(p) => {
            assert_eq!(p.page_count, 1);
            assert!(p.pages_needing_ocr.is_empty());
        }
        other => panic!("routed to {other:?}, not to the PDF engine"),
    }
    assert!(!plan.needs_ocr);
}

// ---------------------------------------------------------------------------
// The behaviour that was named wrongly.
// ---------------------------------------------------------------------------

/// A real user password is refused, by the variant that describes it.
#[test]
fn a_real_user_password_is_refused_as_password_required() {
    let err = readany::read(&fixture(Some("hunter2"), "ownersecret"))
        .expect_err("a PDF with a user password must not read");
    assert!(
        matches!(err, readany::ReadError::PasswordRequired),
        "expected PasswordRequired, got {err:?}"
    );
}

/// The message may not name a remedy that does not exist.
///
/// This is the defect the whole file is here for: the error said "supply one
/// with `Options::pdf_password`", and there is no such field. A caller who
/// believed it went looking for something that was never there.
#[test]
fn the_refusal_names_no_option_that_does_not_exist() {
    let err = readany::read(&fixture(Some("hunter2"), "ownersecret")).unwrap_err();
    let message = err.to_string();
    assert!(
        !message.contains("pdf_password"),
        "the message names a field that does not exist: {message}"
    );
    assert!(
        !message.contains("Options::"),
        "the message points at an option; there is none: {message}"
    );
    // And it still says what happened, rather than going quiet to avoid lying.
    assert!(
        message.contains("encrypted"),
        "the message no longer says what was wrong: {message}"
    );
}

/// Strict mode changes nothing here, and that is the point.
///
/// A refusal is not a partial read. `strict` turns a partial read into an
/// error; an encrypted document was never partial — it was never opened.
#[test]
fn an_encrypted_pdf_is_refused_whether_or_not_strict_is_set() {
    for strict in [false, true] {
        let err = readany::read_with(
            &fixture(Some("hunter2"), ""),
            &readany::Options { strict, ..Default::default() },
        )
        .unwrap_err();
        assert!(
            matches!(err, readany::ReadError::PasswordRequired),
            "strict={strict} produced {err:?}"
        );
    }
}

/// Nothing of the document escapes through the refusal.
///
/// The encrypted path is the one place a reader holds bytes it could not
/// interpret, and "could not interpret" is exactly when a library is most
/// tempted to print what it saw. The planted line is invented; see
/// `tests/no_document_in_logs.rs` for the general sweep.
#[test]
fn the_refusal_carries_nothing_from_the_document() {
    let err = readany::read(&fixture(Some("hunter2"), "ownersecret")).unwrap_err();
    for (channel, text) in [
        ("Display", err.to_string()),
        ("Debug", format!("{err:?}")),
    ] {
        assert!(
            !text.contains(LINE_ONE) && !text.contains(LINE_TWO),
            "{channel} of the error carries the document's own text: {text}"
        );
        // And not the password either, which the reader never had but which a
        // future implementation would be handed right here.
        assert!(
            !text.contains("hunter2") && !text.contains("ownersecret"),
            "{channel} of the error carries a password: {text}"
        );
    }
}
