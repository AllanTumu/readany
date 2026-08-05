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
        ReadError::Pdf(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ReadError>;
