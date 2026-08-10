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
println!("{}", doc.receipt());
// 4 page(s): 3 extracted, 1 recognised, 0 unresolved, 0 marked for a person, in 41ms
```

## Why it exists

Three engines already solve three parts of this problem, and nobody joins them up.

| Engine | Reads | Fails on |
|---|---|---|
| [anydoc](https://github.com/firecrawl/anydoc) | 14 office formats | pages with no text layer |
| [pdf-inspector](https://github.com/firecrawl/pdf-inspector) | text-based PDFs | pages with no text layer |
| the `ocr` module here | scans and photographs | containers it links no decoder for |

readany routes each file — and each **page** — to whichever engine can read it.

Pictures decode from JPEG, PNG, TIFF, BMP and WebP in pure Rust, on every
surface including WebAssembly. **HEIC does not**, and that is deliberate: the
only usable decoder is LGPL, and static linking it into a mobile app is a real
licence question rather than a formality. iOS and Android both decode HEIC in
the platform already, so `ocr::image::DecodeImage` is the seam they plug into —
and where nothing does, HEIC is refused **by name**, not reported as an
unrecognised file. See [Decoding what this crate cannot](#decoding-what-this-crate-cannot).

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

The OCR row was produced by the **Chinese** `ch_PP-OCRv4_rec` recogniser on the
Ugandan receipt described below, and is a timing rather than an accuracy claim.

Skew of exactly 7.0 degrees was measured as 7.10.

> **Corrected 10 August 2026: orientation detection does not work on
> photographs.** This line used to end "a page turned on its side was reported
> as rotation 90", from a synthetic fixture, and generalised from it.
>
> Measured on five real photographed receipts, 4032 × 3024, every one of them
> genuinely on its side: `orient::detect` called **five of five upright**, and
> called the same five upright when they were turned upright first. It is not
> weak on this input, it is inverted, and the cause is visible the moment the
> ink mask is saved and looked at — the largest dark region in a photograph of
> a receipt is the shadow under the paper, not the print.
>
> The consequence was the whole of the receipt corpus: a sideways line of type
> becomes a tall narrow crop, a tall narrow crop scaled to the recogniser's
> fixed height is a few pixels wide, and the corpus came back at **1.0 to 1.1
> characters a box**. `Engine::scan_bytes` now settles this by reading rather
> than by measuring pixels — a page below the confidence floor is read again a
> quarter turned, and the attempt that read more, more surely, is kept.
>
> | | before | after |
> |---|---|---|
> | boxes a page | 12–22 | 18–46 |
> | characters a box | 1.0–1.1 | **8.7–15.0** |
> | mean confidence | 0.22–0.29 | **0.80–0.97** |
> | below the floor | 5 of 5 | **0 of 5** |
>
> Both columns: same five photographs, `ScanOptions::for_photograph`, detector
> `max_side` 2560, Latin PP-OCRv5. The corpus is one real person's receipts and
> is not in this repository.

## Reading an actual receipt

Measured with the **Chinese** `ch_PP-OCRv4_rec` recogniser and `ppocr_keys_v1.txt` behind the traits — the `6624 characters` in the run below is that dictionary's own size, and is the record of which model produced every figure on this page. The receipt is Ugandan and its text is pure ASCII, which is why a Chinese dictionary read it at all; on a euro receipt the same model silently drops the `€`. See `readany-ocr/src/expect.rs`.

> **The transcript below is not verbatim, and the difference matters.** The
> receipt was a real person's, from the corpus this project measures against,
> and printing someone's shopping in a public README is not a thing to do for
> an illustration. The **merchant, street and item names have been replaced
> with invented ones**; the amounts are invented too and are arithmetically
> consistent with each other rather than with anything anybody bought.
>
> What is unchanged is everything the section is actually evidence for: eight
> lines of eight, 656 ms, rotation 0, skew −0.10, mean confidence 0.99, and
> **two character errors in about 120 characters** — reproduced in the same two
> places they fell, one in the shop's name and one in an item. The substitution
> preserves the shape of the result and none of the transaction.

```
$ cargo run --release --features onnx --example read_scan -- \
      receipt.jpg det.onnx rec.onnx ppocr_keys_v1.txt

models loaded in 198ms, 6624 characters
read 8 lines in 656ms, rotation 0, skew -0.10, mean confidence 0.99

NORTHGATE MINIMART
Riverbank Rd, Ashfotd
Mitk 2L 8,500
Bread 4,000
Sugar 1kg 6,200
TOTAL 18,700
CASH 20,000
CHANGE 1,300
```

Eight lines out of eight. Two character errors in about 120 characters: `Ashfotd` for Ashford and `Mitk` for Milk — both an `r`/`t` or `l`/`t` confusion on a low stroke, which is the error this recogniser actually makes. **Every figure is correct**, which is what a receipt is for.

The same receipt turned on its side read identically, with orientation detected
as 90 and corrected before recognition. **That result does not generalise, and
was read as though it did.** It is a clean 700 × 900 crop of a receipt filling
its own frame; on the five 4032 × 3024 phone photographs in the corpus the same
detector is wrong five times out of five. See the correction under
[Measured](#measured).

### Two honest findings from this run

**Confidence measures image quality, not correctness.** The clean receipt scored 0.99 while still misreading `Milk` as `Mitk`. Do not present confidence to a user as a probability that the text is right.

**Resampling twice was costing 7 points of accuracy — now fixed.** Straightening the page and then cropping from the straightened copy blurs every glyph twice. Detection now runs on the straightened page, but each crop is sampled from the **original** pixels through the inverse transform, so a glyph is resampled once.

On a page tilted 7 degrees, measured with Levenshtein distance against the ground truth:

| | Character accuracy | Lines exactly right |
|---|---|---|
| Resampled twice | 86.8% | 2 of 8 |
| **Cropped from the original** | **93.9%** | **4 of 8** |

Both rows were measured with the Chinese `ch_PP-OCRv4_rec` recogniser, on an ASCII-only Ugandan receipt. The comparison between them is sound — one model, one image, one variable — but neither number describes the Latin recogniser this project ships, and neither should be quoted for European documents. Re-measured on `sk-bench/receipt.jpg` on 10 August 2026 with the detector and settings held constant, both recognisers found the same 19 lines and 39 boxes; mean confidence was **0.949 with Chinese PP-OCRv4** and **0.993 with Latin PP-OCRv5**, at 830 ms and 767 ms. Character accuracy against ground truth has not been re-measured, because the ground truth for that image is not in this repository.

Errors cut by 53%. The shop's name lost a substituted letter and a transposed one, `MitkZL8.500` became `Milk2L8.500`, and `TOTAL T8,700` became `TOTAL 18,700` — the figure that actually matters on a receipt. Confidence rose from 0.92 to 0.95. The clean and sideways pages improved too: the street name that had been read with an `r` as a `t` came back right. Merchant and street names are held back here for the reason given above; the error classes are the measurement and they are unchanged.

## Handwriting is marked, never guessed

A restaurant tip is written in by hand, and a guessed tip is a wrong tax record
rather than a slightly worse one. Handwriting **recognition** is a decided
"never build" — no small model does it on a CPU — so this crate does the other
half: it notices that a region was probably made by a pen, and then refuses to
say what it says.

A marked region becomes an `ocr::HumanRegion`, which **has no text field**. The
string the recogniser produced is dropped at the moment of the judgement, into a
type with nowhere to hold it, so nothing downstream has to remember not to use
it. In the markdown the region appears in its own column of its own row as
`[handwritten]` — so `TOTAL [handwritten]` is distinguishable from `TOTAL`, and
neither is mistakable for a figure. There is no option to turn the marker off.

The decision is made from the pixels alone: the spread of stroke thickness, taken
from the ridge of a distance transform, and how far the foot of the ink wanders
off a robustly fitted straight line. **Recognition confidence is recorded and not
consulted**, and that was measured rather than assumed — see
`ocr::human::Floors`. Adding "the recogniser was unsure" to the rule removed two
to four marked regions to save one false positive, because a *legible*
handwritten figure is read confidently: the drawn figures came back at a median
of 0.866 where the real printed regions sat at 0.982 with a p10 of 0.577. A
confidence score and the text it scores also come out of one forward pass, so
they are not two witnesses.

**What is real in the numbers and what is not.** The negatives are real: 268
printed regions across seven photographed receipts, none of them handwriting,
because the corpus is card payments and nobody wrote on any of them. The
positives are **synthetic** — pen strokes drawn onto those same real photographs
by `readany-ocr/examples/handwriting.rs`, which also draws the identical glyphs
with one width and one baseline as a control.

| At the shipped floors | Result |
|---|---|
| real printed regions marked | **2 of 268** |
| synthetic pen regions marked | 6 of 8 |
| synthetic machine-drawn control marked | **0 of 16** |

And the cost end to end, over the whole product path on the same seven receipts,
run once with the marking on and once with it off: 262 regions read against 258,
and **not one verdict, field, item, tax band or check changed**.

## Status

| Part | State |
|---|---|
| Routing across office, PDF and image | Working |
| Per-page PDF read with OCR pages flagged | Working |
| Office formats via anydoc | Working |
| Text PDFs via pdf-inspector | Working |
| Image decode: JPEG, PNG, TIFF, BMP, WebP | Working |
| Image decode: HEIC | Seam defined here; the platform supplies the decoder |
| Skew correction | Working |
| Orientation | **Works on scans, not on photographs** — read the correction under [Measured](#measured) before trusting `rotation` |
| CTC decoding, reading order, Markdown assembly | Working |
| Handwriting marked and withheld | Working — **negatives measured on real receipts, positives synthetic** |
| Text recognition backends | Traits defined here; implementation is separate |
| PDF page rasterising | Not bundled by design; supply your own |
| WebAssembly binding | **Published** — `npm install readany`, Node and browser |
| Node native and Python bindings | Not started |

161 tests pass — 139 unit, 21 integration and 1 documentation test, `cargo test
--release` on 10 August 2026 with `STATEMENT_CORPUS_ABSENT=1`, which the
page-limit test requires you to set rather than let it pass against no corpus.
`cargo clippy --all-targets -- -D warnings` is clean for the library and for
the binding, and `cargo check --lib --target wasm32-unknown-unknown` succeeds.

A further 18 checks run against the **built npm package** rather than against
the source — `node wasm/test/binding.test.js`, after `./scripts/build-wasm.sh`.
They exist because the binding had no test of any kind and went a whole release
behind the crate's API without anything failing: `cargo check` on this crate
passes whatever `wasm/` does, so nothing was watching the one artefact npm
receives.

## Limits

Every input is written by a stranger, so a job runs against ceilings. They are
the documented contract rather than "no limits", and a caller reading its own
files can ask for `Limits::none()` — and has to ask, so that "we forgot" and
"we chose" cannot look the same in a review.

| Ceiling | Default | Enforced |
|---|---|---|
| input bytes | 50 MB | before anything is sniffed |
| pages | 200 | after routing, before reading |
| pixels per rendered page | 40,000,000 | at decode, in checked `u64` |
| decompressed archive bytes | 200 MB | **counted as produced**, not read from the header |
| archive entries | 10,000 | from the directory |
| archive nesting depth | 2 | recursively, one level past the limit |
| wall clock | 60 s | by killing a worker, never in this process |
| resident memory | 1 GB | by the operating system, never in this process |

The last two are carried here so that one table states the whole contract, and
they are marked because Rust has no safe way to stop a thread: a parser stuck
in a loop cannot be interrupted from inside the process running it.

A refusal is `ReadError::TooLarge`, which names the limit and the value that
broke it. It is a statement about what we are willing to spend on a document,
not a judgement about the document.

Measured before the archive guard existed: a 4.6 MB `.xlsx`, structurally
valid, expanded to 1.04 GB and was read to completion in 3.0 s using 1.075 GB
of resident memory. The same file is now refused in 0.05 s using 11.5 MB,
because the bytes are counted and discarded rather than held.

## WebAssembly

```bash
npm install readany
```

The binding is in `wasm/` and is published to npm as one `.wasm` with two
JavaScript glues — CommonJS for Node, ESM for browsers. The blob is
byte-identical between them, which `scripts/build-wasm.sh` checks rather than
assumes.

**It carries the routing, the readers and the preparation stages, and no
recognition.** `Detector`, `Recognizer` and `Rasterise` all need a native
runtime, so a page of pixels comes back in `unresolved_pages`. `npm/README.md`
says so in its first paragraph, because that is the sentence a person installing
the package needs before any other.

Built by `scripts/build-wasm.sh`, which also enforces one version number across
`Cargo.toml`, `wasm/Cargo.toml`, `npm/package.json` and the generated
`pkg/package.json`. `wasm/src/lib.rs` refuses to compile if the first two
disagree. Size, and what each switch bought, is in `docs/wasm-size.md`;
how a release is cut is in `RELEASING.md`.

## Design decisions

**No models of our own.** PP-OCRv6 is 1.5M to 34.5M parameters and beats billion-scale vision models at finding text. A detect-and-recognise pair is about 6 MB. Training anything would take months to land below that.

**No classical character recogniser.** A hand-written pipeline reaches perhaps 85 to 92 percent on clean scans. A 6 MB model beats that immediately, in 50 languages.

**OCR is a trait, not a dependency.** [`OcrBackend`] can be the engine here, a remote service, or a stub in a test. A build with no OCR at all still works — it just reports which pages it could not read.

**No PDF rasteriser bundled.** Turning a scanned PDF page into pixels needs a renderer with heavy native dependencies. Rather than force that on every user, those pages are reported as unresolved and the caller supplies images. Honest beats convenient.

**No bundled model weights.** crates.io caps a package at 10 MB and several language heads are needed. Models are resolved from `ANYSCAN_HOME` and verified by SHA-256 against a declared spec before use — `ocr::models::resolve`.

> **Corrected 10 August 2026.** This paragraph said models were "fetched on first use, verified by SHA-256". Nothing fetched and nothing verified: `ocr::models::resolve` had no caller anywhere in this crate or any crate using it, and `~/.cache/anyscan` did not exist on the machine that wrote the sentence. Verification is now really performed, in `readany-ocr`, which refuses to open a recogniser whose file is not the declared model or whose dictionary cannot spell the characters it was opened to read. A document describing a defence that does not exist is worse than one that omits it, because it stops anyone looking.

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

Supply your own, or use a separate backend crate.

```rust
let engine = Engine::new(my_detector, my_recognizer);
let markdown = engine.to_markdown("scan.png")?;
```

**Where model verification actually lives.** This paragraph used to say hashes
were "pinned in `ocr::models` and verified on load", next to a correction three
sections above saying `ocr::models::resolve` had no caller. Both cannot be true
and the correction is the true one: this crate offers `ocr::models::digest` and
nothing here calls it. Verification is performed by the backend, in
`readany-ocr/src/expect.rs`, which refuses to open a model whose file is not the
declared one or whose dictionary cannot spell the characters it was opened to
read.

## Decoding what this crate cannot

`ocr::image::DecodeImage` is the third seam in this crate, and it is shaped like
the other two: declared here, implemented nowhere here.

```rust
let engine = Engine::new(det, rec).with_decoder(Box::new(my_platform_decoder));
```

| Where | Decoder |
|---|---|
| iOS | ImageIO, `CGImageSourceCreateWithData` |
| Android | `BitmapFactory`, API 28 and above |
| macOS desktop | ImageIO, in `readany-ocr::SipsDecoder` |
| Linux server | `libheif` if the licence is settled, or the refusal below |

With no decoder, a HEIC file is refused as `ScanError::NeedsPlatformDecoder`,
which names the format and says what would open it. That is a different fact
from `Unsupported` ("nobody knows what these bytes are") and from `Decode`
("this file is damaged"), and a caller acts differently on each: the same
photograph read on a phone would have succeeded.

A trait with no implementor compiles anywhere, which is why this does not cost
the WebAssembly build.

**An app taking its own photographs should capture JPEG**, and keep HEIC to the
one path where it is unavoidable — a picture chosen from the photo library.

## Next

1. **An orientation classifier.** The projection heuristic in `ocr::image::orient`
   is measured wrong five times out of five on real photographs, and the retry
   that covers for it costs a second inference pass on every page it saves.
   PaddleOCR ships a 0.6 MB one; that is the right fix, and guessing at a better
   projection statistic is not.
2. Move to PP-OCRv6 mobile — 1.5 MB against 4.7, and reported to be faster and
   more accurate.
3. napi-rs and PyO3 bindings.
4. Benchmark against `oar-ocr` and Tesseract on real scans.

Three entries were removed from this list because they were already done and
the list had not noticed. On 10 August 2026: recognition batches through
`Recognizer::recognize_batch` (measured, and defaulted **off** — see
`readany-ocr`), and the rasteriser seam exists as `pdf::render::Rasterise`. In
0.2.0: the wasm-bindgen binding, which had been **published to npm since 6
August** while both this list and the status table above called it not started.
A list that has stopped tracking what shipped is the same defect as a
measurement nobody re-ran.

## Not supported

Handwriting. No small model does it on a CPU, and every honest engine says so.

## License

[MIT](LICENSE)
