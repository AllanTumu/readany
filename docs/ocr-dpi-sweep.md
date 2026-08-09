# Rasterise-and-OCR: what the measurements say

Measured 9 August 2026 on an M-series Mac. Ground truth is the text layer of
PDFs that have one — no hand labelling.

Privacy: counts, timings and confidences only. No balance, account number or
name appears here, and no PDF holding real data is committed.

**Supersedes the first version of this file.** Every sweep in that version was
masked by a bug and none of its numbers meant anything. The bug is below.

## The bug: `crop_to_content` removed the numbers before the network ran

A knob that costs time and changes no output is not a weak knob, it is a
disconnected one. Every setting was flat — dpi 150 to 400, `max_side` 960 to
3200, both detector thresholds — while `max_side` 2560 cost 48% more time than
960. The bigger image was really made, really processed, and changed nothing.

Dumping the exact image handed to the detector found why:

| | pixels |
|---|---|
| Rendered page | 2480 × 3507 |
| **Handed to the detector** | **560 × 2594** |
| Tensor at `max_side` 960 | 192 × 960 |
| Tensor at `max_side` 2560 | 544 × 2560 |

**22.6% of the page width.** The saved PNG contains the description column and
nothing else: the header and the merchant names. The date, amount and balance
columns are absent — not faint, not shrunk, absent.

`crop_to_content` finds the document by texture. On a bank statement the
merchant descriptions are dense while the amount and balance columns are sparse
right-aligned figures on white, so texture detection reads that white space as
background and cuts the page down to the dense column. **The numbers never
reached the network at all**, which is why no threshold, dpi or `max_side`
could recover them.

## Ablation: which correction did it

CaixaBank page 1 at 300 dpi, `max_side` 2560:

| Configuration | Handed to detector | Width kept | Lines | Lines with digits | ms |
|---|---|---|---|---|---|
| all off | 2480 × 3507 | 100% | **68** | **44** | 1655 |
| **+ `crop_to_content`** | **560 × 2594** | **22.6%** | **21** | **0** | 584 |
| + `flatten_lighting` | 2480 × 3507 | 100% | 68 | 44 | 1572 |
| + `fix_skew` | 2480 × 3507 | 100% | 68 | 44 | 1645 |
| + `fix_orientation` | 2480 × 3507 | 100% | 68 | 44 | 1576 |
| all on (old default) | 560 × 2594 | 22.6% | 21 | 0 | 610 |

**`crop_to_content` is the killer**, and it alone reproduces the full damage.
The other three corrections are harmless on a rendered page — and useless: it
has no fold shadow, no skew and no wrong orientation.

## The fix: two profiles

`ScanOptions::for_photograph()` keeps every correction. They were each earned on
seven real photographed receipts.

`ScanOptions::for_rendered_page()` turns them all off. The rasteriser path uses
it. Nothing else about the seam changes.

## Confirmed on three documents, not one

Page 1 of each, 300 dpi, `max_side` 2560:

| File | Profile | Lines | Lines with digits | Confidence |
|---|---|---|---|---|
| CaixaBank | photograph | 21 | 0 | 0.984 |
| CaixaBank | **rendered page** | **68** | **44** | 0.993 |
| Uganda | photograph | 25 | 21 | 0.989 |
| Uganda | rendered page | 25 | 21 | 0.991 |
| Revolut | photograph | 33 | 24 | 0.968 |
| Revolut | **rendered page** | **51** | **32** | 0.944 |

Two of three improve markedly and none is harmed. Uganda is unaffected because
its layout survives the texture crop — which is worth knowing: the bug is
layout-dependent, so a corpus of one would have found it or missed it by luck.

Santander could not be included: it is `.xlsx` and has no PDF to render.

## The two sweeps, re-run now that they mean something

### dpi — the predicted knee at 300 is **FALSE**

CaixaBank page 1, `max_side` 2560, rendered-page profile:

| dpi | pixels | Lines | Lines with digits | Confidence | OCR ms |
|---|---|---|---|---|---|
| 150 | 1240 × 1754 | 68 | 44 | 0.991 | 1287 |
| 200 | 1653 × 2338 | 68 | 44 | 0.992 | 1471 |
| 300 | 2480 × 3507 | 68 | 44 | 0.993 | 1572 |
| 400 → 342 capped | 2828 × 4000 | 68 | 44 | 0.994 | 1562 |

Recall is **saturated at 150 dpi**. Every dpi finds the same 68 lines and the
same 44 with digits; confidence creeps 0.991 → 0.994 for 22% more time.

**150 dpi is the better default for rendered pages**, not 300. It is as
accurate and 18% faster, and it quarters the memory: 2.1 MB a page against
8.3 MB.

### `max_side` — matters, but for confidence rather than recall

| `max_side` | Lines | Lines with digits | Confidence | OCR ms |
|---|---|---|---|---|
| 960 | 68 | 44 | 0.980 | 930 |
| 1600 | 68 | 44 | 0.987 | 1260 |
| 2560 | 68 | 44 | 0.993 | 1653 |
| 3200 | 68 | 44 | 0.987 | 1834 |

Recall is flat; confidence peaks at 2560 and falls again at 3200. Raising it
from 960 buys +0.013 confidence for 78% more time.

## Region, not resolution

The earlier finding that cropping the right-hand 45% read balances at
confidence 1.00 was attributed to bigger digits. It was not. The same crop at
**150 dpi** reads them at **confidence 0.990** — so the crop worked because it
removed the dense column, not because it magnified anything.

That was a one-minute test and it separates the two causes cleanly.

## Predictions, scored

| Prediction | Verdict |
|---|---|
| `crop_to_content` is cutting the amount columns off | **TRUE** — the whole cause |
| Only `det_db_thresh` was swept; `box_thresh` untested | **FALSE** — both were swept together, 0.30/0.50 down to 0.10/0.25 |
| `flatten_lighting` useless on a clean render | **TRUE** — identical output, and it costs time |
| dpi and `max_side` both matter once fixed | **PARTLY** — `max_side` moves confidence only; dpi moves almost nothing. Recall saturates at 150 dpi |
| The crop worked by region, not resolution | **TRUE** — confirmed at 150 dpi |

## What is still not true

A rendered CaixaBank page now yields 44 lines with digits, but that is not yet
a verified statement. Whether those lines assemble into rows the running-balance
invariant can test is a separate question, and it is the next one — the layout
assembler exists and has never been given OCR output.
