# readany

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Read any document into clean Markdown, through one call. Office files, PDFs, scans and photographs.

```bash
npm install readany        # JavaScript and TypeScript, via WebAssembly
cargo add readany          # Rust
```

```rust
let doc = readany::read(&bytes)?;
println!("{}", doc.markdown);
println!("{}", doc.receipt());   // "4 page(s): 3 extracted, 1 recognised, 0 unresolved, in 41ms"
```

## Why it exists

Three engines already solve three parts of this problem, and nobody joins them up.

| Engine | Reads | Fails on |
|---|---|---|
| [anydoc](https://github.com/firecrawl/anydoc) | 14 office formats | pages with no text layer |
| [pdf-inspector](https://github.com/firecrawl/pdf-inspector) | text-based PDFs | pages with no text layer |
| the `ocr` module here | scans and photographs | nothing else |

readany routes each file — and each **page** — to whichever engine can read it.

## The case this was built for

A PDF where some pages were typed and some were scanned. It is the most common real-world document and the one everything else gets wrong.

I built a four-page PDF: three typed pages and one scanned page. Measured, not claimed:

```
$ npx @firecrawl/anydoc mixed.pdf
# ... 3,849 bytes of Markdown ...
$ echo $?
0
```

**anydoc returns text, exits 0, and never mentions that page 3 exists.** No warning. No marker. A caller cannot tell a whole document from a partial one.

```
$ readany mixed.pdf --pages
# ... the same 3 pages ...
readany: 1 page(s) need OCR and were not read: [3]
$ echo $?
3
```

readany reads the three typed pages the cheap way, names the page it could not read, and exits 3 so a pipeline can act on it.

That is the whole design principle: **a partial read must never be able to look like a complete one.**

## Look before you spend

Routing is separate from reading, so a caller can ask what a file will cost before paying for it.

```
$ readany contract.pdf --inspect
mixed PDF, 4 pages, 1 need OCR
```

Measured on this corpus: **1 to 15 ms** to inspect, whatever the file.

```rust
let plan = readany::inspect(&bytes)?;
if plan.ocr_page_count > 10 { /* send it somewhere else */ }
```

## Reading

```
readany report.docx                 # Markdown to stdout
readany scan.pdf --pages            # with <!-- Page N --> markers
readany file.xlsx -o out.md         # to a file, print a receipt
readany photo.jpg --json            # machine-readable report
readany contract.pdf --strict       # fail rather than return a partial document
```

Exit codes: `0` whole file read, `1` unreadable, `2` usage error, **`3` read in part**.

## Measured

| File | Route | Time |
|---|---|---|
| `.docx` | office | 13 ms |
| `.xlsx` | office | 3 ms |
| 4-page text PDF | text PDF | 19 ms |
| 4-page mixed PDF | 3 extracted, 1 flagged | 17 ms |
| photograph, decode + straighten | OCR | 42–53 ms |
| photograph, full OCR (8 lines) | OCR | 656 ms |

Skew of exactly 7.0 degrees was measured as 7.10. A page turned on its side was reported as rotation 90.

## Reading an actual receipt

Measured with a PP-OCRv4 backend behind the traits. The output, verbatim, from a photographed shop receipt on CPU:

```
$ cargo run --release --features onnx --example read_scan -- \
      receipt.jpg det.onnx rec.onnx ppocr_keys_v1.txt

models loaded in 198ms, 6624 characters
read 8 lines in 656ms, rotation 0, skew -0.10, mean confidence 0.99

QUICKMART SUPERMARKET
Kampala Rd, Kampela
Mitk 2L 8,500
Bread 4,000
Sugar 1kg 6,200
TOTAL 18,700
CASH 20,000
CHANGE 1,300
```

Eight lines out of eight. Two character errors in about 120 characters: `Kampela` for Kampala and `Mitk` for Milk. **Every figure is correct**, which is what a receipt is for.

The same receipt turned on its side reads identically — orientation is detected as 90 and corrected before recognition.

### Two honest findings from this run

**Confidence measures image quality, not correctness.** The clean receipt scored 0.99 while still misreading `Milk` as `Mitk`. Do not present confidence to a user as a probability that the text is right.

**Resampling twice was costing 7 points of accuracy — now fixed.** Straightening the page and then cropping from the straightened copy blurs every glyph twice. Detection now runs on the straightened page, but each crop is sampled from the **original** pixels through the inverse transform, so a glyph is resampled once.

On a page tilted 7 degrees, measured with Levenshtein distance against the ground truth:

| | Character accuracy | Lines exactly right |
|---|---|---|
| Resampled twice | 86.8% | 2 of 8 |
| **Cropped from the original** | **93.9%** | **4 of 8** |

Errors cut by 53%. `OUICXMART` became `QUICKMART`, `MitkZL8.500` became `Milk2L8.500`, and `TOTAL T8,700` became `TOTAL 18,700` — the figure that actually matters on a receipt. Confidence rose from 0.92 to 0.95. The clean and sideways pages improved too: `Kampela` is now read correctly as `Kampala`.

## Status

| Part | State |
|---|---|
| Routing across office, PDF and image | Working |
| Per-page PDF read with OCR pages flagged | Working |
| Office formats via anydoc | Working |
| Text PDFs via pdf-inspector | Working |
| Image decode, orientation, skew | Working |
| CTC decoding, reading order, Markdown assembly | Working |
| Text recognition backends | Traits defined here; implementation is separate |
| PDF page rasterising | Not bundled by design; supply your own |
| Node, Python, WASM bindings | Not started |

42 tests pass. `cargo clippy --all-targets -- -D warnings` is clean.

## Design decisions

**No models of our own.** PP-OCRv6 is 1.5M to 34.5M parameters and beats billion-scale vision models at finding text. A detect-and-recognise pair is about 6 MB. Training anything would take months to land below that.

**No classical character recogniser.** A hand-written pipeline reaches perhaps 85 to 92 percent on clean scans. A 6 MB model beats that immediately, in 50 languages.

**OCR is a trait, not a dependency.** [`OcrBackend`] can be the engine here, a remote service, or a stub in a test. A build with no OCR at all still works — it just reports which pages it could not read.

**No PDF rasteriser bundled.** Turning a scanned PDF page into pixels needs a renderer with heavy native dependencies. Rather than force that on every user, those pages are reported as unresolved and the caller supplies images. Honest beats convenient.

**No bundled model weights.** crates.io caps a package at 10 MB and several language heads are needed. Models are fetched on first use, verified by SHA-256, cached under `ANYSCAN_HOME`.

## Architecture

```
file bytes
  │
  ├─► route::inspect        → Office | Pdf{text,mixed,scanned} | Image | Unknown
  │
  ├─ Office ──► anydoc                      → Markdown
  │
  ├─ PDF ─────► pdf-inspector, page by page
  │                ├─ page has text  ──────► Markdown        (Origin::Text)
  │                └─ page is pixels ──────► ocr, or flagged (Origin::Ocr | Unresolved)
  │
  └─ Image ───► ocr
                  ├─ decode, orient, deskew
                  ├─ detect  (trait)
                  ├─ recognize (trait) → ctc
                  ├─ layout  → columns, then lines, then order
                  └─ markdown → headings, paragraphs, low-confidence marks
```

Columns are split **before** boxes are grouped into lines. The other way round merges a line on the left of a two-column page with the line beside it on the right. There is a test named after that.

## Text recognition

`Detector` and `Recognizer` are traits. This crate ships the pipeline around them — image decode, orientation, skew correction, cropping, CTC decoding, reading order and Markdown assembly — but no model.

Supply your own, or use a separate backend crate. Model file hashes for PP-OCRv4 are pinned in `ocr::models` and verified on load, so a corrupted or swapped file is rejected rather than silently producing nonsense.

```rust
let engine = Engine::new(my_detector, my_recognizer);
let markdown = engine.to_markdown("scan.png")?;
```

## Next

1. Move to PP-OCRv6 mobile — 1.5 MB against 4.7, and reported to be faster and more accurate.
3. Batch the recognition calls. Each crop is currently a separate inference.
4. A `PageRasterizer` trait so scanned PDF pages can be read end to end.
5. napi-rs, PyO3 and wasm-bindgen bindings.
6. Benchmark against `oar-ocr` and Tesseract on real scans.

## Not supported

Handwriting. No small model does it on a CPU, and every honest engine says so.

## License

[MIT](LICENSE)
