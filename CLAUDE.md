# readany

OCR for scans and photographs. CLI binary: `readany`.

## Build and test

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings   # must be clean
cargo test --release
```

All three must pass before committing.

## Architecture

```
src/
  lib.rs               – public API: read(), read_with(), Document, Options, Origin
  error.rs             – ReadError, wrapping all three engines
  route.rs             – inspect(): Office | Pdf | Image | Unknown, before any work
  ocr/
    mod.rs             – OcrBackend trait
    engine.rs          – ScanOptions, Engine, prepare()
    error.rs           – ScanError
    types.rs           – Quad, TextBox, TextLine, ScanResult
    image/
      decode.rs          – bytes to grey, format from content
    binarize.rs        – Otsu level, ink mask with polarity correction
    deskew.rs          – projection-profile skew, bilinear rotation
    orient.rs          – 0/90/180/270 by projection spikiness
  detect/mod.rs        – Detector trait
  recognize/
    mod.rs             – Recognizer trait
    ctc.rs             – greedy CTC decode, charset parsing
  layout/reading_order.rs – columns, then lines, then order
  markdown/mod.rs      – headings by height ratio, paragraphs by gap
  models/mod.rs        – cache dir, SHA-256 verification
  bin/anyscan.rs       – CLI
```

## Key design decisions

- **Columns are split before lines are grouped.** Grouping first merges left and right columns at the same height. There is a test for this.
- **Backends are traits, not concrete types.** Keeps the neural runtime out of the core crate so it builds for WebAssembly.
- **Weights are never bundled.** crates.io caps at 10 MB. Fetch, verify by SHA-256, cache in `ANYSCAN_HOME`.
- **Ink polarity is corrected.** If more than half the page reads as ink, the mask is inverted.
- **Blank is CTC class 0**, matching PaddleOCR dictionaries.

## Conventions

- Clippy: use `is_none_or` and `is_some_and`; iterate with `enumerate` rather than indexing in loops.
- Every public type gets a doc comment saying what a caller does with it.
- Tests are named as sentences describing the behaviour, not the function.

## The rule that defines this library

A partial read must never look like a complete one. `anydoc` returns exit 0 on a
mixed PDF and silently omits the scanned page; that is the behaviour readany
exists to fix. Every page appears in `Document::pages` with an `Origin`, and an
unreadable page is named in `unresolved_pages`, never dropped. `--strict` and
exit code 3 exist for the same reason. Do not add a code path that discards a
page quietly.

## Never resample a glyph twice

Detection runs on the straightened page; crops are sampled from `Prepared::original`
through `image::Correction`. Straightening and then cropping from the straightened
copy costs about 7 points of character accuracy on a tilted page — measured, not
guessed. `crop_corrected` is the only path that should be used to feed the
recogniser.
