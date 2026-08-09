//! Model cache.
//!
//! Weights are never bundled in the crate. crates.io caps a package at 10 MB
//! and we need several language heads, so models are fetched on first use,
//! verified by SHA-256, and cached in a user directory. The cache location can
//! be overridden with `ANYSCAN_HOME`, and a machine with no network can be
//! preseeded by copying files into it.

use crate::ocr::error::{Result, ScanError};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The models this library is tested against. Hashes were computed from the
/// files actually downloaded and run, not copied from a README.
pub const PP_OCRV4_DET: ModelSpec = ModelSpec {
    name: "ch_PP-OCRv4_det",
    file: "ch_PP-OCRv4_det_infer.onnx",
    sha256: "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
    url: "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_det_infer.onnx",
};

pub const PP_OCRV4_REC: ModelSpec = ModelSpec {
    name: "ch_PP-OCRv4_rec",
    file: "ch_PP-OCRv4_rec_infer.onnx",
    sha256: "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b",
    url: "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv4/ch_PP-OCRv4_rec_infer.onnx",
};

pub const PP_OCRV3_REC_EN: ModelSpec = ModelSpec {
    name: "en_PP-OCRv3_rec",
    file: "en_PP-OCRv3_rec_infer.onnx",
    sha256: "ef7abd8bd3629ae57ea2c28b425c1bd258a871b93fd2fe7c433946ade9b5d9ea",
    url: "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv3/en_PP-OCRv3_rec_infer.onnx",
};

/// A model file we know how to verify.
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub name: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    pub url: &'static str,
}

/// Where models live. `ANYSCAN_HOME`, else the platform cache directory.
pub fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ANYSCAN_HOME") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("HOME")
                .map(|h| PathBuf::from(h).join(".cache"))
                .unwrap_or_else(|_| PathBuf::from(".cache"))
        });
    base.join("anyscan")
}

pub fn model_path(spec: &ModelSpec) -> PathBuf {
    cache_dir().join(spec.file)
}

/// Hex SHA-256 of a file.
pub fn file_digest(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(digest(&bytes))
}

pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Return the path to a verified local model, or explain what is missing.
/// Downloading is left to the caller so the core crate stays free of a HTTP
/// stack and still builds for WebAssembly.
pub fn resolve(spec: &ModelSpec) -> Result<PathBuf> {
    let path = model_path(spec);
    if !path.exists() {
        return Err(ScanError::ModelMissing {
            name: spec.name.to_string(),
            path,
        });
    }
    let found = file_digest(&path)?;
    if found != spec.sha256 {
        return Err(ScanError::ModelCorrupt {
            name: spec.name.to_string(),
            expected: spec.sha256.to_string(),
            found,
        });
    }
    Ok(path)
}

#[cfg(test)]
mod tests {

    /// `ANYSCAN_HOME` is process-global, so the three tests that set it cannot
    /// run at the same time. Without this they pass or fail depending on the
    /// order the harness happens to pick — which is how this surfaced: adding
    /// unrelated tests elsewhere in the crate changed the order and one of
    /// these started failing.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use super::*;
    use std::io::Write;

    #[test]
    fn the_shipped_specs_have_real_hashes() {
        for spec in [PP_OCRV4_DET, PP_OCRV4_REC, PP_OCRV3_REC_EN] {
            assert_eq!(spec.sha256.len(), 64, "{} has a stub hash", spec.name);
            assert!(spec.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(spec.url.starts_with("https://"));
        }
    }

    #[test]
    fn digest_is_stable_and_correct() {
        // Known SHA-256 of the empty string.
        assert_eq!(
            digest(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn cache_dir_respects_the_override() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("ANYSCAN_HOME", "/tmp/anyscan-test-home");
        assert_eq!(cache_dir(), PathBuf::from("/tmp/anyscan-test-home"));
        std::env::remove_var("ANYSCAN_HOME");
    }

    #[test]
    fn a_missing_model_says_where_it_should_be() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ANYSCAN_HOME", dir.path());
        let spec = ModelSpec {
            name: "det-tiny",
            file: "det-tiny.onnx",
            sha256: "00",
            url: "https://example.invalid/det-tiny.onnx",
        };
        match resolve(&spec) {
            Err(ScanError::ModelMissing { name, .. }) => assert_eq!(name, "det-tiny"),
            other => panic!("expected ModelMissing, got {other:?}"),
        }
        std::env::remove_var("ANYSCAN_HOME");
    }

    #[test]
    fn a_tampered_model_is_rejected() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ANYSCAN_HOME", dir.path());
        let mut f = std::fs::File::create(dir.path().join("rec-tiny.onnx")).unwrap();
        f.write_all(b"not really a model").unwrap();
        let spec = ModelSpec {
            name: "rec-tiny",
            file: "rec-tiny.onnx",
            sha256: "deadbeef",
            url: "https://example.invalid/rec-tiny.onnx",
        };
        match resolve(&spec) {
            Err(ScanError::ModelCorrupt { expected, .. }) => assert_eq!(expected, "deadbeef"),
            other => panic!("expected ModelCorrupt, got {other:?}"),
        }
        std::env::remove_var("ANYSCAN_HOME");
    }
}
