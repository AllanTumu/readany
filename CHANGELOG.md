## [0.2.1] - 2026-08-19

### Fixed

- **A tall detection box no longer bridges two receipt rows.** A detector crop
  whose height reached into the row beneath it overlapped both and merged them,
  so two printed lines became one carrying a single amount and the other row was
  lost entirely. Overlap alone cannot tell that case from one row detected
  twice, so the boxes' centres must also be close — at most 0.75 of the shorter
  box's height.

  Found on a real supermarket receipt where `1 CORN…` and `1 30 B.ESCU…` were
  read as one line.

# Changelog

## 0.2.0 — 10 August 2026

First release to crates.io. `readany` 0.1.0 existed only on npm; the crate name
was unclaimed until now.

**This release breaks things, and 0.x is the honest signal.** `DecodeImage`
still has to grow a rasterising counterpart and will break again.

### Breaking

**`Document` gained three fields and `Page` gained two.** `Document::ocr_pages`
and `Document::human_pages` join `unresolved_pages`; `Page::human_regions` and
`Page::confidence` are new. Constructing either struct literally will no longer
compile. They are plain data on purpose, so the alternative was accessors on a
type whose whole job is to be read.

**`Document::receipt()` says one more thing.** It now ends `..., N marked for a
person, in Xms`. Anything parsing that string, or asserting on it, changes.

**A fully textual PDF is no longer one page.** It used to be read in a single
call and returned as one page numbered 1, however long it was — a six-page bank
statement reported "1 page". Every PDF is now read page by page. A caller
counting `pages.len()` gets a different number, and a correct one.

**`ReadError::PasswordRequired` is now actually returned.** It was in the public
enum and never constructed: `pdf-inspector`'s `Encrypted` became
`ReadError::Pdf(String)`, so a caller matching on `PasswordRequired` to prompt
for a password never saw it fire. `ReadError::PasswordRejected` is new beside
it — different fact, different action.

**`ReadError::TooLarge` is new**, and is returned for input that crosses any
limit. Code matching exhaustively on `ReadError` will not compile.

**`ScanOptions::default()` changed meaning.** It is now `for_photograph()`
explicitly, and two more profiles exist. If you were reading rendered PDF pages
with the default you were cropping the amount column off your bank statements;
see below.

**`zip` moved from 5 to 8.** Only visible if you were relying on this crate's
resolution of it.

#### npm package

**`doc.pages` is an array, not a count.** It was the number of pages; it is now
every page, in order, each with `origin`, `confidence` and `human_regions`. The
count moved to `doc.page_count`. This is the change the release exists for — a
consumer could not tell which page came from a text layer and which did not,
which is the distinction the whole library is built around.

**Every function takes an options object as a second argument.** Optional, and
omitting it behaves as before.

**Errors are typed.** They were `Error` with an English message. They are now
`Error` with `name: "ReadError"` and a `kind`, plus `limit`, `allowed` and
`found` on a refusal. Anything matching on message text should move to `kind`.

**The package now requires an explicit entry point in browsers**: `readany/web`,
which needs `await init()` before the first call. The default entry is still
CommonJS for Node and is unchanged in how it loads.

### Added

- **Handwriting is noticed and withheld.** A region judged to be pen rather than
  print becomes an `ocr::HumanRegion`, which **has no text field** — the
  recogniser's string is dropped at the moment of judgement, into a type with
  nowhere to hold it. In the markdown it appears in its own column of its own
  row as `[handwritten]`. The judgement is made from the ink alone: stroke width,
  its variation, and baseline drift. Recognition confidence is recorded and
  deliberately **not** consulted — adding it removed two to four real marks to
  save one false positive, because a legible handwritten figure is read
  confidently. Measured at the shipped floors: 2 of 268 real printed regions
  marked, 6 of 8 synthetic pen regions marked, 0 of 16 machine-drawn controls.
  The negatives are real receipts; the positives are drawn on. There is no
  option to turn the marker off.
- **A third scan profile, `ScanOptions::for_scanned_page()`.** A scan is neither
  a photograph nor a render, and reading it as either loses the document. On a
  generated statement, one degree of tilt was the difference between 18 of 18
  rows read and 7 of 18 with a `Failed` verdict.
- **`TextLine::min_confidence` and `ScanResult::min_confidence`**, reporting the
  weakest box rather than the average of them. On 497 lines of a damaged
  statement, at floor 0.5: the mean flagged 0 of 69 wrong lines, the minimum
  flagged 18, and neither accused any of the 428 correct ones. Better as a
  ranking than as a gate — the note on the method says why.
- **`ScanResult::weak_boxes`** and **`ScanResult::human_regions`**, both
  returning the line index and the cell, which is what marking a field needs.
- **A limits registry, `limits::Limits`**, with a documented default for input
  bytes, pages, pixels per page, decompressed bytes, archive entries and archive
  depth. `Limits::none()` exists and has to be asked for.
- **An archive bomb guard.** Measured before it existed: a 4.6 MB `.xlsx`,
  structurally valid, expanded to 1.04 GB and read to completion in 3.0 s using
  1.075 GB of resident memory. It is now refused in 0.05 s using 11.5 MB,
  because the bytes are counted as they are produced rather than trusted from
  the header — which an attacker writes.
- **The PDF rasterising seam, `pdf::render::Rasterise`**, declared with no
  implementation, alongside `cap_dpi`, `MAX_SIDE_PX` and the coordinate mapping
  between points and pixels.
- **Delimited text is recognised from content.** A CSV has no signature, and a
  spreadsheet export was reported as unrecognised while the documentation
  promised we read it. `Options::filename` is consulted last, for the
  one-column case that content sniffing cannot decide.
- **Lighting flattening and content cropping** before detection, and a
  geometric retry when a page comes back below the confidence floor.
- **`readany::VERSION`**, so there is one version in this project rather than
  one per manifest.
- **`THIRD-PARTY-LICENSES.md` ships in the npm package.** The blob statically
  links 104 crates under permissive licences that all require their notices to
  travel with a binary. `docs/licensing.md` had said so since it was written;
  the package shipped this crate's own LICENSE and nothing else.
- **The npm package runs in browsers**, via `readany/web`. The `.wasm` is
  byte-identical between wasm-pack's two targets, so this costs about 21 KB of
  glue rather than a second binary — checked at build time, not assumed.
- **`prepare()` in the npm package**: decode, crop, flatten, orient and deskew,
  the whole half of the OCR pipeline that needs no model. A browser can
  straighten a photograph locally and send a corrected grayscale page rather
  than the original with its EXIF.
- **`defaultLimits()` and `version()` in the npm package.**

### Fixed

- **Orientation detection does not work on photographs, and the engine stopped
  believing it.** On five real photographed receipts, every one genuinely on its
  side, `orient::detect` called five of five upright — and called the same five
  upright when they were turned upright first. It is not weak on this input, it
  is inverted: the largest dark region in a photograph of a receipt is the
  shadow under the paper. `Engine::scan_bytes` now settles it by reading rather
  than by measuring pixels. Characters a box went from 1.0–1.1 to 8.7–15.0, mean
  confidence from 0.22–0.29 to 0.80–0.97, and pages below the floor from 5 of 5
  to 0 of 5.
- **`crop_to_content` was cutting the numbers off bank statements.** It finds the
  document by texture, and an amount column is sparse right-aligned figures on
  white, which texture detection reads as background. On a rendered page at
  300 dpi: 21 lines and **0 with digits** under the photograph profile, against
  68 lines and 44 with digits under the new rendered-page profile. That one fact
  explained a whole set of flat measurements — dpi, `max_side` and both detector
  thresholds all changed nothing, because every one of them was operating on an
  image the amounts had already been cut out of.
- **A reachable panic in the pixel path**, and the panicking forms are now
  denied by lint wherever untrusted bytes arrive.
- **Pixel ceilings are computed in checked `u64`.** A page declaring 65,536
  square overflows `u32` to exactly zero and passed every limit beneath it.
- **A skew estimate that ran out of search is refused** rather than returned.
- **The page-limit test failed to fail.** It passed against no corpus at all;
  `STATEMENT_CORPUS_ABSENT=1` now has to be declared rather than inferred.
- **The crate no longer depends on a private crate its own CI cannot have.**

### Documentation

- Every OCR figure on the README now names the model that produced it. They were
  measured with the **Chinese** `ch_PP-OCRv4_rec` recogniser on an ASCII-only
  Ugandan receipt — a dictionary with no `€`, which silently drops the currency
  symbol from a euro total. Latin PP-OCRv5 is what ships and is asserted at
  load. On the same image the two found the same 19 lines and 39 boxes at 0.949
  and 0.993 mean confidence; character accuracy has **not** been re-measured
  against the Latin model and the old figures are marked as not describing it.
- The status table and the "next" list both called the WebAssembly binding not
  started, while it had been published on npm since 6 August.
- `Options` documents why there is no `pdf_password` — passing one through
  measured as a *different answer*, not a slower one: 11 of 12 pages produced
  different markdown, because the ordinary route computes font statistics across
  the whole document and the password-capable route sees one page at a time.
- `Origin` documents why there is no `Human` variant: it answers a question
  about a page, and handwriting is a fact about a region.
- `docs/wasm-size.md` records what each build switch actually bought, including
  the two that made the blob larger.
- `cargo fmt --check` was removed from CI rather than left failing, with the
  reasoning written down.

### Known and unfixed

- **`chacha20` is linked and unused.** It arrives behind `rand`, which `lopdf`
  needs for *writing* encrypted PDFs, and nothing here writes one. It cannot be
  feature-gated away without a change in `lopdf`. `aes` is in the blob too and
  stays: `lopdf` uses it to *open* encrypted PDFs, which this crate documents,
  depends on and tests.
- **`trim-paths` is unavailable on stable cargo**, so build-machine paths are
  removed with `--remap-path-prefix` instead, and the build greps the blob to
  prove it worked.
- **Orientation still needs a classifier.** The projection heuristic is wrong
  five times out of five on real photographs and the retry that covers for it
  costs a second inference pass on every page it saves.

## 0.1.0 — 6 August 2026

Published to npm only. Office documents, text-layer PDFs and file inspection,
through WebAssembly.
