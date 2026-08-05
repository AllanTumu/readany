use std::path::PathBuf;

/// Every way anyscan can fail.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unsupported input: {0}")]
    Unsupported(String),

    #[error("image could not be decoded: {0}")]
    Decode(String),

    #[error("no text was found in the image")]
    NoText,

    #[error("model {name} is not available locally; run with auto-download or preseed {path}")]
    ModelMissing { name: String, path: PathBuf },

    #[error("model {name} failed its checksum: expected {expected}, found {found}")]
    ModelCorrupt {
        name: String,
        expected: String,
        found: String,
    },

    #[error("inference failed: {0}")]
    Inference(String),
}

pub type Result<T> = std::result::Result<T, ScanError>;
