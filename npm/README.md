# readany

Route and read documents into clean Markdown. Word, Excel, PowerPoint, OpenDocument, RTF, EPUB, CSV and text-layer PDF — one call, no service, no API key, no account.

**This package does not recognise text from pixels.** It holds the routing, the readers and the whole geometric half of the OCR pipeline — decode, crop, flatten, orient, deskew — but the detector and the recogniser are Rust traits (`Detector`, `Recognizer`) with no implementation here. They need a native neural runtime, which WebAssembly is the wrong shape for. The reference implementation of those traits lives in `readany-ocr`, which is **not published**. The same is true of `Rasterise`, the trait that turns a PDF page into pixels: PDFium is native C++, and `readany-pdfium` is not published either.

So a page made only of pixels comes back **named** in `unresolved_pages`, never silently skipped. That is the point of the package, not a gap in it.

```bash
npm install readany
```

```js
const fs = require('fs');
const { read, inspect, toMarkdown } = require('readany');

console.log(toMarkdown(fs.readFileSync('report.docx')));
```

In a browser or any ESM bundler, use the `web` entry point and initialise once:

```js
import init, { read } from 'readany/web';

await init();                     // fetches and instantiates the .wasm
const doc = read(new Uint8Array(await file.arrayBuffer()));
```

Both entry points share one `.wasm` binary and expose the same functions. Only the loading differs: Node reads the blob from disk synchronously, the browser fetches it.

## It tells you what it could not read

This is the reason it exists. Give most converters a PDF where some pages were typed and some were scanned, and they return the typed pages, exit successfully, and never mention the rest. You cannot tell a whole document from a partial one.

```js
const doc = read(fs.readFileSync('mixed.pdf'));

doc.markdown          // the pages that could be read
doc.complete          // false
doc.unresolved_pages  // [3]
doc.receipt           // "4 page(s): 3 extracted, 0 recognised, 1 unresolved, 0 marked for a person, in <n>ms"
```

Every page is accounted for, and each one says where its text came from:

```js
doc.pages[0]          // { number: 1, markdown: "...", origin: "text",
                      //   confidence: null, human_regions: [] }
doc.pages[2].origin   // "unresolved" — needed recognition, did not get it
```

`origin` is `"text"` (read from the file's own text layer), `"ocr"` (recognised from pixels) or `"unresolved"`. In this package it is never `"ocr"`, because nothing here can recognise; the value exists so that code written against this package keeps working unchanged the day the work moves to a service that can.

Ask `strict: true` to throw instead of returning a partial document.

## Look before you spend

Routing is separate from reading, so you can ask what a file will cost before paying for it.

```js
const plan = inspect(bytes);

plan.kind                  // "office" | "pdf" | "image" | "unknown"
plan.summary               // "mixed PDF, 4 pages, 1 need OCR"
plan.needs_ocr             // true
plan.ocr_page_count        // 1
plan.pdf.pages_needing_ocr // [3]
plan.pdf.kind              // "text" | "mixed" | "scanned"
```

## Options

Every call takes an optional second argument. An unrecognised key is an error rather than a shrug, so a mistyped option cannot silently do nothing.

```js
read(bytes, {
  filename: 'export.csv',   // consulted only when the bytes have no signature
  page_markers: true,       // insert <!-- Page N --> between pages
  strict: false,            // throw rather than return a partial document
  limits: { pages: 500 },   // raise one ceiling, keep the rest
  // no_limits: true        // remove all ceilings; cannot be combined with `limits`
});
```

`filename` is asked last and never overrides what the bytes said. It matters for one real case: a one-column CSV has no signature and is indistinguishable from prose without a name.

## Refusals you can act on

A hostile file is refused before it is read, against [documented ceilings](https://github.com/AllanTumu/readany#limits). Failures throw a real `Error` with a `kind`, so you never have to match on English:

```js
try {
  read(bytes);
} catch (e) {
  e.name        // "ReadError"
  e.kind        // "too_large" | "password_required" | "unsupported" | ...
  e.limit       // "input size"        (kind === "too_large")
  e.allowed     // 52428800
  e.found       // 62914560, or null when the true value was never computed
}
```

`defaultLimits()` returns the ceilings in force so you can show a person why a file was refused without hard-coding numbers.

Note `found: null`. "We stopped counting at the cap" and "it was zero" are different facts, and an archive bomb is caught by the second kind — the bytes are counted as they are produced, never trusted from the header.

## Straightening a page without recognising it

`prepare` runs the half of the OCR pipeline that needs no model. It is useful on its own: a browser can decode, crop, flatten, orient and deskew a photograph locally, then send a corrected grayscale page to whatever does the recognising, instead of sending the original photograph.

```js
const page = prepare(bytes, { profile: 'photograph' });
page.width, page.height   // the straightened page
page.pixels               // Uint8Array, 8-bit grayscale, row-major
page.skew                 // degrees corrected
```

The profile matters more than it sounds. `"photograph"` crops the document out of its background by texture; on a bank statement that cuts off the amount column, because right-aligned figures on white read as background. Use `"rendered_page"` for anything that came out of a PDF and `"scanned_page"` for anything off a scanner or photocopier.

`page.rotation` is a guess. The orientation heuristic was measured wrong on every one of five real phone photographs, and the retry that corrects it in the full engine needs a recogniser this package does not have. Trust it for scans, not for photographs.

## What it reads

| Format | Extensions |
|---|---|
| Word | `.doc` `.docx` `.docm` |
| PowerPoint | `.ppt` `.pps` `.pot` `.pptx` `.pptm` `.ppsx` `.ppsm` |
| Excel | `.xls` `.xlsx` `.xlsm` `.xlsb` |
| OpenDocument | `.odt` `.ods` `.odp` |
| Rich Text | `.rtf` |
| EPUB | `.epub` |
| CSV | `.csv` |
| PDF | `.pdf`, where a text layer exists |

The format is read from the file's own bytes, so a mislabelled file still works.

Images decode from JPEG, PNG, TIFF, BMP and WebP for `prepare`. **HEIC does not decode**: the only usable decoder is LGPL, and static linking it is a real licence question. iOS and Android decode HEIC in the platform already.

## What it does not do

**Recognise text from pixels.** A scan, a photograph or a fax is reported in `unresolved_pages`. See the first paragraph for why.

**Rasterise a PDF page.** A scanned PDF page cannot even be handed to a recogniser from here, because turning it into pixels needs a renderer with heavy native dependencies.

**Read handwriting** — a decided never, in every build. No small model does it on a CPU, and every honest engine says so.

**Notice handwriting.** This is worth separating from the line above, because the full engine does do it and this package cannot. Marking a region as handwritten is a judgement made from ink — stroke width, its variation, how far the baseline wanders — measured on a crop that only a recogniser's detection pass produces. With no recogniser there are no crops, so `human_pages` is always empty here and `[handwritten]` can never appear in the markdown. **Do not read an empty `human_pages` from this package as "nothing was written on this page by hand."** It means nobody looked.

## How it works

WebAssembly, compiled from Rust. There is no server, no account and no per-page fee, and **your documents never leave the machine they are on** — which is a plain consequence of there being nothing to send them to, not a policy that could change.

Built on [anydoc](https://github.com/firecrawl/anydoc) and [pdf-inspector](https://github.com/firecrawl/pdf-inspector), both MIT.

The full engine, its measurements and the reasoning behind them: [github.com/AllanTumu/readany](https://github.com/AllanTumu/readany).

## License

[MIT](LICENSE).

The `.wasm` statically links about a hundred Rust crates, all under permissive
licences — MIT, Apache-2.0, BSD, Zlib, Unlicense — every one of which requires
its notice to travel with a binary. They are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md), shipped in this package and
regenerated from the dependency tree at every build.
