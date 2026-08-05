# readany

Read any document into clean Markdown. Word, Excel, PowerPoint, OpenDocument, RTF, EPUB, CSV and PDF — one call, no service, no API key.

```bash
npm install readany
```

```js
const fs = require('fs');
const { read, inspect, toMarkdown } = require('readany');

const bytes = fs.readFileSync('report.docx');
console.log(toMarkdown(bytes));
```

## It tells you what it could not read

This is the reason it exists. Give most converters a PDF where some pages were typed and some were scanned, and they return the typed pages, exit successfully, and never mention the rest. You cannot tell a whole document from a partial one.

```js
const doc = read(fs.readFileSync('mixed.pdf'));

doc.markdown          // the pages that could be read
doc.complete          // false
doc.unresolved_pages  // [3]
doc.receipt           // "4 page(s): 3 extracted, 0 recognised, 1 unresolved, in 12ms"
```

Every page is accounted for. Nothing is dropped quietly.

## Look before you spend

Routing is separate from reading, so you can ask what a file will cost before paying for it.

```js
const plan = inspect(bytes);

plan.kind            // "office" | "pdf" | "image" | "unknown"
plan.summary         // "mixed PDF, 4 pages, 1 need OCR"
plan.needs_ocr       // true
plan.ocr_page_count  // 1
```

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

## What it does not read

**Pages made only of pixels** — a scan, a photograph, a fax. Text recognition needs a native runtime and is not part of this package. Such pages are reported in `unresolved_pages`, never silently skipped.

**Handwriting.** No small model does this well on a processor, and every honest engine says so.

## How it works

WebAssembly, compiled from Rust. It runs in Node and in the browser. There is no server, no account, no per-page fee, and **your documents never leave the machine they are on**.

Built on [anydoc](https://github.com/firecrawl/anydoc) and [pdf-inspector](https://github.com/firecrawl/pdf-inspector), both MIT.

## License

[MIT](LICENSE)
