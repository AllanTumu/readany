use crate::ocr::ScanError;

/// One error type for the whole library, whichever engine produced it.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported input: {0}")]
    Unsupported(String),

    #[error("document conversion failed: {0}")]
    Document(String),

    #[error("PDF could not be read: {0}")]
    Pdf(String),

    /// The PDF is encrypted, and the empty password did not open it.
    ///
    /// **The empty-password case never reaches here, and that is the case that
    /// matters.** A great many bank statements are encrypted with no user
    /// password at all — only an owner password, which restricts printing and
    /// copying rather than opening. Those read exactly as their unencrypted
    /// twin does, through the ordinary path, with no option set and nothing
    /// said. Measured: an RC4-40 file with an empty user password produces
    /// markdown byte-identical to the same document unencrypted. See
    /// `tests/encrypted_pdf.rs`.
    ///
    /// This variant is what is left: a document that wants a password nobody
    /// supplied. It is returned rather than guessed at — trying a date of
    /// birth or an account number would be worse than failing.
    ///
    /// It also covers the rarer case of encryption this build cannot open at
    /// all, such as a public-key handler, because `pdf-inspector` reports both
    /// as one error and inventing a distinction we cannot observe would be
    /// worse than naming the common one.
    ///
    /// **There is no `Options::pdf_password`.** This message used to name one,
    /// which was the whole defect: a caller who read the error and went looking
    /// found nothing. See [`crate::Options`] for what can and cannot be done
    /// about an encrypted PDF today, and why.
    #[error("PDF is encrypted and the empty password did not open it")]
    PasswordRequired,

    /// A password was supplied for an encrypted PDF and it did not open it.
    ///
    /// Distinct from [`ReadError::PasswordRequired`] because the two are
    /// different facts and a caller acts differently on them: one asks a person
    /// for a password, the other tells them the one they gave was wrong. The
    /// same distinction this crate already draws between a page that was not
    /// read and a page that was not printed.
    ///
    /// Only a [`crate::pdf::render::Rasterise`] implementation can produce
    /// this, because rasterising is the only path in this project that accepts
    /// a password at all.
    ///
    /// **The password is not in this error, and must never be.** It carries no
    /// data for exactly that reason.
    #[error("the password supplied did not open this PDF")]
    PasswordRejected,

    /// The file crossed a limit meant to keep a hostile document from taking
    /// the service down. Distinct from every other error because it is not a
    /// judgement about the document at all — it is a statement about what we
    /// are willing to spend on one.
    #[error("refused: {0}")]
    TooLarge(crate::limits::Exceeded),

    #[error("OCR failed: {0}")]
    Ocr(#[from] ScanError),

    /// The file needs OCR and no backend was supplied. This is a distinct
    /// error rather than an empty result, so a caller can never mistake
    /// "nothing was read" for "there was nothing to read".
    #[error("{pages} page(s) need OCR and no OCR backend is configured")]
    OcrRequired { pages: usize },
}

impl From<anydoc::ConvertError> for ReadError {
    fn from(e: anydoc::ConvertError) -> Self {
        ReadError::Document(e.to_string())
    }
}

impl From<pdf_inspector::PdfError> for ReadError {
    fn from(e: pdf_inspector::PdfError) -> Self {
        match e {
            // `pdf-inspector` already tried the empty password before saying
            // this, so reaching here means the document genuinely wants one.
            // It used to become `Pdf("PDF is encrypted")` — a named refusal, so
            // it failed safe, but the wrong name: a caller matching on
            // `PasswordRequired` to prompt for a password never saw it fire,
            // and the variant sat unconstructed in a public enum.
            pdf_inspector::PdfError::Encrypted => ReadError::PasswordRequired,
            other => ReadError::Pdf(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, ReadError>;
