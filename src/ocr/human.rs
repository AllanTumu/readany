//! Handwriting: notice it, mark it, and refuse to say what it says.
//!
//! # Why this is not a recogniser
//!
//! Handwriting recognition is a decided "never build" for this engine: no
//! small model does it on a CPU, and a guessed restaurant tip is a wrong tax
//! record rather than a slightly worse one. So nothing here reads anything.
//! The only question asked is *"were these marks made by a mechanism or by a
//! hand"*, and the only thing done with the answer is to withhold the
//! recogniser's output for that region.
//!
//! # What it rests on
//!
//! One external expectation about the physical world, and nothing else:
//!
//! > A machine that prints puts ink down to a fixed width and on a fixed line,
//! > because it has no way to do otherwise. A hand holding a pen has neither
//! > property.
//!
//! Everything in [`Ink`] is computed here, from the crop, by arithmetic the
//! recognition network never touched. That is deliberate. A confidence score
//! and the text it scores come out of the same forward pass, so they cannot
//! corroborate one another — a detector that marks a region because the
//! recogniser was unsure, and then justifies the mark by saying the text is
//! probably wrong, has consulted one witness twice. Confidence also does not
//! *mean* handwriting: it means "this was hard", which is equally true of
//! glare, a crease, a faded thermal till roll, a barcode and a logo.
//!
//! Confidence was measured as a third condition anyway, and declined. The
//! finding is on [`Floors`] and it is the opposite of the obvious guess: **a
//! legible handwritten figure is read confidently**, so requiring the
//! recogniser to have struggled loses the case the product rule exists for.
//!
//! # Where the expectation breaks
//!
//! It is falsifiable, and it has a named blind spot: a *printed* line can lose
//! both properties when the paper is crumpled, the thermal ink has faded
//! unevenly, or the photograph is soft — and all three are ordinary on a real
//! till receipt. That is why the floors on [`Floors`] are set where the measured
//! false positives on real printed receipts are, not where the synthetic
//! positives are best separated.
//!
//! # What a verifier receives
//!
//! The other half of this rule is arithmetic: a total that includes an unread
//! tip cannot verify against a subtotal, and it must say so rather than quietly
//! pass. That work belongs to `readany-verify`, which reads a
//! [`crate::Document`] and never sees a pixel. This is the whole of what it is
//! handed, and the whole of what it may rely on:
//!
//! ```text
//! // In the text, in the marked cell's own column, on the right row.
//! readany::ocr::HUMAN_MARK: &str             // "[handwritten]"
//! readany::ocr::TextLine::text(&self) -> String
//! ```
//!
//! `HUMAN_MARK` appears in `Document::markdown` at the position the writing was
//! in, so a row that reads `TOTAL [handwritten]` is distinguishable from a row
//! that reads `TOTAL` — "there was a figure here and we will not tell you what
//! it said" against "there was no figure here". It is a fixed constant, it holds
//! no digit, and it parses as no money and no date, so a parser that does not
//! know about it cannot accidentally read a value out of it. There is no option
//! to suppress it.
//!
//! ```text
//! // Structured, for a verifier that would rather not match on a string.
//! readany::Document::human_pages:   Vec<u32>   // 1-indexed, beside unresolved_pages
//! readany::Document::needs_a_person(&self) -> bool
//! readany::Page::human_regions:     Vec<HumanRegion>
//! readany::ocr::ScanResult::human_regions(&self) -> Vec<(usize, &HumanRegion)>
//! readany::ocr::ScanResult::needs_a_person(&self) -> bool
//! readany::ocr::TextLine::human:    Vec<HumanRegion>
//! readany::ocr::TextLine::needs_a_person(&self) -> bool
//!
//! pub struct HumanRegion { pub quad: Quad, pub evidence: Evidence }
//! pub struct Evidence { pub ink: Ink, pub confidence: f32, pub page_confidence: f32 }
//! ```
//!
//! **Three things a verifier is entitled to assume.**
//!
//! 1. A marked region carries **no text**, anywhere, in any form. [`HumanRegion`]
//!    has no field for it and the string the recogniser produced is dropped
//!    before the region is built. A check cannot accidentally sum it, because it
//!    does not exist to be summed.
//! 2. A marked region is **on a row**, and the mark stands in its own column of
//!    that row. A tip written beside a printed `TIP` label lands on the `TIP`
//!    line, not in a footnote.
//! 3. `Document::is_complete()` stays **true** for a document with marked
//!    regions, and that is deliberate: incompleteness means a page produced no
//!    text and might have. "Part of this was not read" is
//!    [`crate::Document::needs_a_person`], and a verifier that wants to refuse a
//!    verdict should ask that question rather than the completeness one.
//!
//! What is deliberately **not** provided: any page-level `Origin::Human`. See
//! [`crate::Origin`] for why — handwriting is a fact about a region, and a page
//! whose printed total was read perfectly is an `Ocr` page whatever else is on
//! it.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use super::image::{binarize, GrayImage};
use super::types::Quad;

/// The token a marked region contributes wherever text is assembled.
///
/// It is a constant, it is identical for every region, and it contains no
/// digit and no letter a reader could take for a value. That is the point: a
/// marked region has to appear *in place* — a row whose amount was written by
/// hand must not come out looking like a row that had no amount — while being
/// impossible to mistake for something the engine read.
///
/// There is no option to suppress it. A marker a caller can turn off is the
/// silent drop this crate exists to refuse.
pub const HUMAN_MARK: &str = "[handwritten]";

/// A crop smaller than this in either direction is not measured. Below about
/// this size the thickness samples below are a handful of pixels and their
/// spread is noise rather than a pen.
const MIN_SIDE: u32 = 10;

/// Fewest stroke-width samples worth taking a median of.
const MIN_STROKE_SAMPLES: usize = 40;

/// Fewest ink columns worth fitting a baseline through.
const MIN_INK_COLUMNS: usize = 12;

/// Measurements taken from a region's pixels alone.
///
/// **Nothing in here came from the recogniser.** Every field is computed from
/// the ink mask of the crop, by [`measure`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ink {
    /// Median stroke thickness, in pixels.
    ///
    /// Taken from the ridge of a chamfer distance transform: twice the distance
    /// to the nearest paper, sampled where that distance is locally greatest —
    /// which is the middle of a stroke, whichever way the stroke runs.
    ///
    /// **The obvious cheap version of this measures the wrong thing.** The first
    /// draft took the smaller of each ink pixel's horizontal and vertical run.
    /// That is thickness for a stem and thickness for a crossbar, and it is
    /// `thickness / sin θ` for everything in between — so a glyph made of
    /// curves reports a wide spread of "thicknesses" while a glyph made of
    /// uprights and bars reports a narrow one. Measured on a real receipt, that
    /// statistic put drawn digits of exactly one width (0.850) *above* the
    /// receipt's own print (0.375): it had ranked stroke *direction*, not stroke
    /// weight, and it ranked it backwards for this purpose.
    pub stroke_width: f32,
    /// Spread of that thickness, as `(p75 - p25) / p50`.
    ///
    /// Dimensionless, so it does not move with the resolution the page was
    /// photographed at.
    pub stroke_variation: f32,
    /// How far the foot of the ink wanders off a straight line, as a fraction
    /// of the ink's own height.
    ///
    /// A *straight* line is fitted first and the residual measured against it,
    /// so a tilted crop — which is every crop off a photograph — is not
    /// mistaken for a wandering one. The fit is Theil-Sen rather than least
    /// squares, because descenders cluster: see `theil_sen` in this module for the two
    /// numbers that cost. The statistic is the median absolute residual rather
    /// than the largest, so that a comma a few columns wide does not read the
    /// same as a whole word bending away.
    pub baseline_drift: f32,
    /// Fraction of the crop that is ink.
    pub coverage: f32,
}

/// Why a region was marked. Carried so a consumer can explain the mark to a
/// person, and so a measurement can be reproduced.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Evidence {
    /// The two measurements the decision was actually made on.
    pub ink: Ink,
    /// What the recogniser thought of this region.
    ///
    /// **Recorded, not consulted.** [`judge`] does not read this field, and
    /// [`Floors`] explains at length why: a legible handwritten figure is read
    /// confidently, so requiring the recogniser to have struggled would blind
    /// the detector to the case the rule exists for. It is kept because it is
    /// worth showing a person — "this cell read 0.31 where the page's median
    /// was 0.98" is a useful sentence — and because a number nobody can see is
    /// a number nobody can argue with.
    pub confidence: f32,
    /// The page's median box confidence: what its neighbours managed.
    pub page_confidence: f32,
}

/// A region the engine judged to be handwritten, and therefore refused to read.
///
/// **There is no text field on this type, and that is the whole design.** The
/// recogniser did produce characters for this region — it always does — and
/// they are dropped at the point the judgement is made, before anything can
/// hold them. A consumer cannot print them, cannot sum them and cannot store
/// them, because there is nowhere they exist. Marking is a thing the engine
/// does *instead of* reading, not a warning attached to a reading.
///
/// What is kept is where to draw the box and what the measurements were.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HumanRegion {
    pub quad: Quad,
    pub evidence: Evidence,
}

/// Where the two pixel signals have to stand before a region is marked.
///
/// # The corpus
///
/// **The negatives are real.** Seven photographed receipts, straightened through
/// the ordinary pipeline and read at 2560 px: **268 printed regions**, not one
/// of which is handwriting, because every receipt in the corpus is a card
/// payment and nobody wrote on any of them — checked by looking at all seven,
/// not assumed.
///
/// **The positives are not real and must never be quoted as if they were.** No
/// handwritten document exists in the corpus, so `readany-ocr`'s
/// `examples/handwriting.rs` draws pen strokes onto those same photographs, in
/// the gaps between the receipts' own printed rows, at the scale of their own
/// print. Real: the paper, the crease, the lighting, the camera, and the
/// printed neighbours. Synthetic: the ink. Every recall figure below is a
/// **synthetic** recall figure.
///
/// The same file draws the identical glyphs a second way — one width, one
/// baseline, no wobble — as a control, because a detector that fires on drawn
/// strokes may only have noticed *drawing*.
///
/// # The sweep
///
/// | `stroke_variation` | `baseline_drift` | printed, real | pen, synthetic | machine control |
/// |---|---|---|---|---|
/// | 0.60 | — | 19 of 268 | 7 of 8 | 0 of 16 |
/// | 0.80 | — | 6 of 268 | 6 of 8 | 0 of 16 |
/// | 1.20 | — | 3 of 268 | 1 of 8 | 0 of 16 |
/// | — | 0.04 | **203 of 268** | 7 of 8 | 13 of 16 |
/// | — | 0.08 | **135 of 268** | 6 of 8 | 5 of 16 |
/// | — | 0.20 | 49 of 268 | 0 of 8 | 0 of 16 |
/// | 0.75 | 0.08 | 5 of 268 | 6 of 8 | 0 of 16 |
/// | **0.80** | **0.08** | **2 of 268** | **6 of 8** | **0 of 16** |
/// | 0.80 | 0.10 | 2 of 268 | 3 of 8 | 0 of 16 |
/// | 0.90 | 0.08 | 2 of 268 | 5 of 8 | 0 of 16 |
///
/// # What the two false positives cost, end to end
///
/// A false positive here withholds a real printed value, so the number worth
/// having is not the count but the consequence. `readany-ocr`'s
/// `examples/receipts.rs` runs the whole product path — photograph in, receipt
/// arithmetic out — over the same seven receipts, and it was run twice, once
/// with [`Floors::MEASURED`] and once with [`Floors::never`]:
///
/// | | marking off | marking on |
/// |---|---|---|
/// | regions read | 262 | **258** |
/// | verified / refused / unverifiable | 1 / 4 / 2 | **1 / 4 / 2** |
/// | fields extracted | 20 of 49 | **20 of 49** |
/// | checks run, per document | identical | identical |
///
/// **Four regions withheld, and not one verdict, field, item, tax band or check
/// changed.** The regions the floors are wrong about are not regions the
/// arithmetic was using. That is the honest form of the cost: it is small, it is
/// measured on real documents, and it is not zero.
///
/// The time cost is not measurable at this resolution. Per page, the same seven
/// receipts took 5.5/6.6/3.2/5.8/6.6/1.6/5.9 seconds with the marking off and
/// 5.6/6.7/3.2/5.8/6.6/1.6/5.9 with it on: a distance transform over the crops
/// disappears next to two neural passes over the page.
///
/// # The hole none of this closes
///
/// Everything above is about regions the **text detector** found. It does not
/// find most handwriting. From the same run: 28 pen figures drawn, **8** boxed
/// by the detector; 22 machine-drawn figures, **16** boxed. A region that is
/// never detected is never judged and never marked, so the marking cannot fire
/// on the majority of what it would want to mark, and no floor here can change
/// that. It is a limit of DBNet on pen-shaped glyphs and it wants a different
/// fix from a different place.
///
/// **Stroke weight is the signal; the baseline is not one on its own.** A floor
/// on drift alone refuses half of all real printed regions and a third of the
/// machine control — it is measuring the curl of a till roll and the softness of
/// a phone photograph as much as anything about a pen. It earns its place only
/// in the conjunction, where it takes the false positives from 6 to 2 without
/// costing a single marked pen region. Neither number is the best separator of
/// the synthetic positives; both are set where the *real* false positives stop
/// falling.
///
/// # Why confidence is not one of these fields
///
/// It was measured, and it was declined. Adding "the recogniser was unsure of
/// this region, and sure of its neighbours" to the conjunction above:
///
/// | added condition | printed, real | pen, synthetic |
/// |---|---|---|
/// | none | 2 of 268 | 6 of 8 |
/// | confidence ≤ 0.95, and 0.05 below the page median | 1 of 268 | 4 of 8 |
/// | confidence ≤ 0.80, and 0.05 below the page median | 1 of 268 | 2 of 8 |
///
/// One fewer false positive costs two to four marked regions, and those two
/// trades are not comparable: a region wrongly marked asks a person a question,
/// and a region wrongly read writes a number into a tax record.
///
/// The reason it fails is worth stating, because it is the opposite of what the
/// signal's name suggests. **A legible handwritten figure is read confidently.**
/// Measured on the same run: the drawn figures came back at a median of 0.866
/// and a p10 of 0.645; the real printed regions at a median of 0.982 and a p10
/// of **0.577**. The distributions overlap, and the printed tail is the *lower*
/// of the two. Requiring the recogniser to have struggled would blind the
/// detector to exactly the case the product rule exists for: the tip that is
/// readable, is read, and is wrong.
///
/// And there is a second reason it could not have carried the decision alone.
/// A confidence score and the text it scores come out of one forward pass. They
/// are not two witnesses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floors {
    /// Least `baseline_drift` that counts as a wandering line.
    pub baseline_drift: f32,
    /// Least `stroke_variation` that counts as an uneven stroke.
    pub stroke_variation: f32,
}

impl Floors {
    /// The shipped setting: the row in bold in the sweep on [`Floors`].
    pub const MEASURED: Floors = Floors {
        baseline_drift: 0.08,
        stroke_variation: 0.80,
    };

    /// Mark nothing.
    ///
    /// This exists so the marking can be *measured*: a restriction that cannot
    /// be removed cannot be shown to be doing anything, and every count in the
    /// sweep above is a difference between a run with [`Floors::MEASURED`] and a
    /// run with this. It is a named constructor rather than a `bool` field so
    /// that a caller reaching for it has to say out loud what it is asking for.
    pub const fn never() -> Floors {
        Floors {
            baseline_drift: f32::INFINITY,
            stroke_variation: f32::INFINITY,
        }
    }
}

impl Default for Floors {
    fn default() -> Self {
        Floors::MEASURED
    }
}

/// Should this region be marked for a person rather than read?
///
/// Two conditions, both about the pixels and neither about the network:
///
/// 1. the ink is not one thickness,
/// 2. and it does not sit on one line.
///
/// `ink` is `None` when the crop was too small or too empty to measure, and
/// then the answer is no. **Absence of evidence never marks**, because a mark
/// costs a real reading.
pub fn judge(ink: Option<Ink>, floors: &Floors) -> bool {
    let Some(ink) = ink else { return false };
    ink.stroke_variation >= floors.stroke_variation && ink.baseline_drift >= floors.baseline_drift
}

/// The confident neighbours a region is compared against: the median box
/// confidence of the page.
///
/// The median rather than the mean, so that a page carrying several marked
/// regions does not drag its own reference down and stop marking the rest.
/// Returns 0.0 for a page with no boxes, which makes the gap in [`judge`]
/// negative and marks nothing — the right answer for a page nothing was read
/// on.
pub fn page_reference(confidences: &[f32]) -> f32 {
    let mut sorted: Vec<f32> = confidences.iter().copied().filter(|c| c.is_finite()).collect();
    if sorted.is_empty() {
        return 0.0;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    percentile(&sorted, 0.5)
}

/// Measure a crop's ink. `None` means "no opinion": too small, or too little
/// ink to say anything about.
pub fn measure(crop: &GrayImage) -> Option<Ink> {
    let (w, h) = crop.dimensions();
    if w < MIN_SIDE || h < MIN_SIDE {
        return None;
    }
    let width = w as usize;
    let height = h as usize;
    let mask = binarize::ink_mask(crop);
    if mask.len() != width.checked_mul(height)? {
        return None;
    }

    let ink_pixels = mask.iter().filter(|&&b| b).count();
    if ink_pixels == 0 {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    let coverage = ink_pixels as f32 / mask.len() as f32;

    let mut thickness = stroke_samples(&mask, width, height);
    if thickness.len() < MIN_STROKE_SAMPLES {
        return None;
    }
    thickness.sort_unstable();
    let widths: Vec<f32> = thickness.iter().map(|&t| f32::from(t) / 10.0).collect();
    let p25 = percentile(&widths, 0.25);
    let p50 = percentile(&widths, 0.50);
    let p75 = percentile(&widths, 0.75);
    if p50 <= 0.0 {
        return None;
    }
    let stroke_variation = (p75 - p25) / p50;

    let baseline_drift = baseline_drift(&mask, width, height)?;

    Some(Ink {
        stroke_width: p50,
        stroke_variation,
        baseline_drift,
        coverage,
    })
}

/// Stroke thickness in tenths of a pixel, sampled once per point on the ink's
/// medial ridge.
///
/// A chamfer distance transform gives every ink pixel its distance to the
/// nearest paper. That distance is greatest along the centre line of a stroke
/// and equals half the stroke's width there, whichever direction the stroke
/// runs — which is the property the horizontal-run version did not have.
///
/// There is **no guard against strokes the crop bisects**, and there used to be:
/// samples within two pixels of the border were dropped, on the reasoning that
/// outside the crop is paper and a stroke cut in half would report thin. The
/// chamfer below does not do that — it takes no neighbour from beyond the edge,
/// so a truncated stroke keeps the distance its own interior gives it. Measured
/// with the margin removed, on the seven-receipt corpus: the printed and pen
/// quantiles are **identical to three decimal places** and the false-positive
/// count is unchanged at 2 of 268. A guard against a failure the algorithm does
/// not have is a line that reads like a safety net and is not one.
fn stroke_samples(mask: &[bool], width: usize, height: usize) -> Vec<u16> {
    let distance = chamfer(mask, width, height);
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let Some(here) = at(&distance, width, x, y) else {
                continue;
            };
            if here == 0 {
                continue;
            }
            // On the ridge: no neighbour is strictly further from the paper.
            let mut ridge = true;
            for (dx, dy) in NEIGHBOURS {
                let (Some(nx), Some(ny)) = (
                    x.checked_add_signed(dx as isize),
                    y.checked_add_signed(dy as isize),
                ) else {
                    continue;
                };
                if at(&distance, width, nx, ny).unwrap_or(0) > here {
                    ridge = false;
                    break;
                }
            }
            if ridge {
                // Twice the half-width, in tenths of a pixel: the chamfer's
                // unit step is 3, so a distance of `d` is `d / 3` pixels.
                out.push(here.saturating_mul(20).saturating_div(3));
            }
        }
    }
    out
}

const NEIGHBOURS: [(i8, i8); 8] = [
    (-1, -1),
    (0, -1),
    (1, -1),
    (-1, 0),
    (1, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
];

/// Two-pass chamfer distance transform with 3-4 weights: how far each ink pixel
/// is from the nearest paper, in thirds of a pixel.
fn chamfer(mask: &[bool], width: usize, height: usize) -> Vec<u16> {
    const FAR: u16 = u16::MAX;
    let mut d: Vec<u16> = mask.iter().map(|&ink| if ink { FAR } else { 0 }).collect();

    for y in 0..height {
        for x in 0..width {
            let mut best = at(&d, width, x, y).unwrap_or(0);
            if best == 0 {
                continue;
            }
            for (dx, dy, w) in [(-1i8, 0i8, 3u16), (-1, -1, 4), (0, -1, 3), (1, -1, 4)] {
                let (Some(nx), Some(ny)) = (
                    x.checked_add_signed(dx as isize),
                    y.checked_add_signed(dy as isize),
                ) else {
                    continue;
                };
                if let Some(n) = at(&d, width, nx, ny) {
                    best = best.min(n.saturating_add(w));
                }
            }
            if let Some(slot) = at_mut(&mut d, width, x, y) {
                *slot = best;
            }
        }
    }
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let mut best = at(&d, width, x, y).unwrap_or(0);
            if best == 0 {
                continue;
            }
            for (dx, dy, w) in [(1i8, 0i8, 3u16), (1, 1, 4), (0, 1, 3), (-1, 1, 4)] {
                let (Some(nx), Some(ny)) = (
                    x.checked_add_signed(dx as isize),
                    y.checked_add_signed(dy as isize),
                ) else {
                    continue;
                };
                if let Some(n) = at(&d, width, nx, ny) {
                    best = best.min(n.saturating_add(w));
                }
            }
            if let Some(slot) = at_mut(&mut d, width, x, y) {
                *slot = best;
            }
        }
    }
    d
}

fn offset(width: usize, x: usize, y: usize) -> Option<usize> {
    if x >= width {
        return None;
    }
    y.checked_mul(width)?.checked_add(x)
}

fn index(mask: &[bool], width: usize, x: usize, y: usize) -> Option<bool> {
    mask.get(offset(width, x, y)?).copied()
}

fn at(buffer: &[u16], width: usize, x: usize, y: usize) -> Option<u16> {
    buffer.get(offset(width, x, y)?).copied()
}

fn at_mut(buffer: &mut [u16], width: usize, x: usize, y: usize) -> Option<&mut u16> {
    buffer.get_mut(offset(width, x, y)?)
}

/// Median absolute residual of the ink's lowest row against a fitted straight
/// line, as a fraction of the ink's median height.
fn baseline_drift(mask: &[bool], width: usize, height: usize) -> Option<f32> {
    let mut xs: Vec<f32> = Vec::new();
    let mut bottoms: Vec<f32> = Vec::new();
    let mut heights: Vec<f32> = Vec::new();

    for x in 0..width {
        let mut top: Option<usize> = None;
        let mut bottom: Option<usize> = None;
        for y in 0..height {
            if index(mask, width, x, y).unwrap_or(false) {
                if top.is_none() {
                    top = Some(y);
                }
                bottom = Some(y);
            }
        }
        let (Some(top), Some(bottom)) = (top, bottom) else {
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        {
            xs.push(x as f32);
            bottoms.push(bottom as f32);
            heights.push((bottom.saturating_sub(top)) as f32);
        }
    }

    if xs.len() < MIN_INK_COLUMNS {
        return None;
    }

    // **A robust fit, not a least squares one.** The foot of a `p`, a `y`, a
    // comma or a stray speck of paper texture sits a long way below the line,
    // and ordinary least squares *tilts* to follow them — after which every
    // other column is off the fitted line and a straight printed row measures
    // as a wandering one. Worse, those excursions cluster: a word ending in `y`
    // puts them all at one end, which is exactly the arrangement with the most
    // leverage over a slope.
    //
    // Theil-Sen takes the median of the pairwise slopes instead, so up to about
    // a quarter of the columns can be anywhere at all without moving the line.
    // A single round of trimmed least squares was tried first and was not
    // enough: on the fixture in `refit_tests` it still reported 0.188 against a
    // floor of 0.10, because when the first fit is already tilted every residual
    // is large and a cut proportional to them excludes nothing.
    //
    // Residuals are then taken over **every** column, including the ones the fit
    // ignored. The outliers are not what is being removed; the tilt is.
    let (slope, intercept) = theil_sen(&xs, &bottoms)?;
    let mut residuals: Vec<f32> = xs
        .iter()
        .zip(bottoms.iter())
        .map(|(&x, &b)| (b - (slope * x + intercept)).abs())
        .collect();
    residuals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));

    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let ink_height = percentile(&heights, 0.5);
    if ink_height <= 0.0 {
        return None;
    }
    Some(percentile(&residuals, 0.5) / ink_height)
}

/// Theil-Sen: the slope is the median of the slopes of every pair of points,
/// and the intercept is the median of `y - slope * x`.
///
/// `SLOPE_SAMPLES` columns at most take part in the pairwise step, spread evenly
/// across the crop, so the cost is bounded whatever the crop's width — a full
/// statement line is two thousand columns and four million pairs, and the answer
/// does not need them. The intercept is taken over every column.
///
/// # What the robust fit is worth, measured
///
/// Two things were tried before this. Ordinary least squares over every column:
/// on the seven-receipt corpus the printed regions' median drift was 0.176 and
/// the false positives were **4 of 268**. One round of trimmed least squares —
/// fit, drop the columns more than 2.5 median residuals out, fit again — got the
/// median to 0.113 and the false positives to 2 of 268, but still failed the
/// fixture in `refit_tests` at 0.188 against a floor of 0.10: when the first fit
/// is already tilted, every residual is large and a cut proportional to them
/// excludes nothing. Theil-Sen takes the printed median to **0.086** and passes
/// it.
///
/// Falsified by replacing the median of the pairwise slopes with their mean,
/// which is least squares in disguise: `refit_tests` goes red.
///
/// `None` when there are too few points, or when every sampled pair shares an x.
fn theil_sen(xs: &[f32], ys: &[f32]) -> Option<(f32, f32)> {
    const SLOPE_SAMPLES: usize = 120;
    let n = xs.len().min(ys.len());
    if n < 2 {
        return None;
    }
    let step = n.div_ceil(SLOPE_SAMPLES).max(1);

    let mut slopes: Vec<f32> = Vec::new();
    let mut i = 0usize;
    while i < n {
        let mut j = i.saturating_add(step);
        while j < n {
            let (Some(&xi), Some(&xj), Some(&yi), Some(&yj)) =
                (xs.get(i), xs.get(j), ys.get(i), ys.get(j))
            else {
                break;
            };
            let dx = xj - xi;
            if dx.abs() > f32::EPSILON {
                slopes.push((yj - yi) / dx);
            }
            j = j.saturating_add(step);
        }
        i = i.saturating_add(step);
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let slope = percentile(&slopes, 0.5);

    let mut offsets: Vec<f32> = xs
        .iter()
        .zip(ys.iter())
        .map(|(&x, &y)| y - slope * x)
        .collect();
    offsets.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    Some((slope, percentile(&offsets, 0.5)))
}

/// Nearest-rank percentile of an already-sorted slice.
fn percentile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len().saturating_sub(1);
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let index = ((q.clamp(0.0, 1.0) * last as f32).round() as usize).min(last);
    sorted.get(index).copied().unwrap_or(0.0)
}


#[cfg(test)]
mod tests {
    // The tests build small images by hand and index them; see the same allow
    // in `binarize`.
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;

    /// A printed line: even stems, all standing on one row.
    fn printed(width: u32, height: u32) -> GrayImage {
        let mut img = GrayImage::from_pixel(width, height, image::Luma([255u8]));
        let baseline = height - 6;
        let mut x = 8;
        while x + 4 < width - 8 {
            for y in 8..baseline {
                for dx in 0..4 {
                    img.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            x += 12;
        }
        img
    }

    /// The same marks made by something that can hold neither a width nor a
    /// line: the stem thickness cycles and the feet bend away from straight.
    ///
    /// The bend is a **curve**, not a slope. `baseline_drift` fits a straight
    /// line before measuring, on purpose — a tilted photograph produces a
    /// perfectly straight sloping baseline and must not be called handwriting —
    /// so a fixture that only slopes would measure zero drift and prove
    /// nothing.
    fn written(width: u32, height: u32) -> GrayImage {
        let mut img = GrayImage::from_pixel(width, height, image::Luma([255u8]));
        let mut x = 8u32;
        let mut n = 0u32;
        while x + 10 < width - 8 {
            let thickness = 2 + (n % 6);
            let phase = x as f32 / width as f32 * std::f32::consts::TAU * 2.0;
            let bend = (phase.sin() * height as f32 * 0.30) as i32;
            let foot = ((height as i32 - 8) + bend).clamp(12, height as i32 - 2) as u32;
            for y in 8..foot {
                for dx in 0..thickness {
                    img.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            x += thickness + 8;
            n += 1;
        }
        img
    }

    #[test]
    fn a_crop_too_small_to_measure_has_no_opinion() {
        assert!(measure(&GrayImage::from_pixel(6, 6, image::Luma([0u8]))).is_none());
    }

    #[test]
    fn blank_paper_has_no_opinion_and_is_not_marked() {
        // No ink at all. `None` must not be confused with "measured, and it
        // looked handwritten": absence of evidence never marks.
        let blank = GrayImage::from_pixel(200, 40, image::Luma([255u8]));
        assert!(measure(&blank).is_none());
        assert!(!judge(measure(&blank), &Floors::MEASURED));
    }

    #[test]
    fn print_holds_its_width_and_its_line() {
        let ink = measure(&printed(240, 48)).expect("measurable");
        assert!(
            ink.stroke_variation < Floors::MEASURED.stroke_variation,
            "printed stroke varied {}",
            ink.stroke_variation
        );
        assert!(!judge(Some(ink), &Floors::MEASURED));
    }

    #[test]
    fn a_hand_holds_neither() {
        let ink = measure(&written(420, 72)).expect("measurable");
        assert!(
            ink.stroke_variation >= Floors::MEASURED.stroke_variation,
            "written stroke varied only {}",
            ink.stroke_variation
        );
        assert!(
            ink.baseline_drift >= Floors::MEASURED.baseline_drift,
            "written baseline drifted only {}",
            ink.baseline_drift
        );
        assert!(judge(Some(ink), &Floors::MEASURED));
    }

    /// **Both conditions are required, and each one alone is not enough.**
    ///
    /// This is the whole of the measured result stated as a test. On the seven
    /// real receipts, a floor on `baseline_drift` alone refused 144 of 268
    /// printed regions — more than half the corpus — and a floor on
    /// `stroke_variation` alone refused 6. Together they refuse 2. A conjunction
    /// nobody can see the necessity of gets taken apart by the next person to
    /// read it, so here are the two halves, each shown insufficient.
    #[test]
    fn neither_signal_marks_on_its_own() {
        // Even width, bending line: a printed row on a curled till roll.
        let mut even_but_bent = GrayImage::from_pixel(280, 64, image::Luma([255u8]));
        let mut x = 8u32;
        while x + 10 < 272 {
            let phase = x as f32 / 280.0 * std::f32::consts::TAU * 3.0;
            let foot = ((44.0 + phase.sin() * 14.0) as u32).clamp(20, 62);
            for y in 8..foot {
                for dx in 0..4 {
                    even_but_bent.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            x += 12;
        }
        let bent = measure(&even_but_bent).expect("measurable");
        assert!(
            bent.baseline_drift >= Floors::MEASURED.baseline_drift,
            "this fixture has to clear the drift floor or it proves nothing: {}",
            bent.baseline_drift
        );
        assert!(
            !judge(Some(bent), &Floors::MEASURED),
            "a bending line of even print was called handwriting"
        );

        // Uneven width, straight line: a worn thermal head, or ink bleed.
        let mut uneven_but_straight = GrayImage::from_pixel(280, 64, image::Luma([255u8]));
        let mut x = 8u32;
        let mut n = 0u32;
        while x + 10 < 272 {
            let thickness = 2 + (n % 6);
            for y in 8..56 {
                for dx in 0..thickness {
                    uneven_but_straight.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            x += thickness + 8;
            n += 1;
        }
        let uneven = measure(&uneven_but_straight).expect("measurable");
        assert!(
            uneven.stroke_variation >= Floors::MEASURED.stroke_variation,
            "this fixture has to clear the variation floor or it proves nothing: {}",
            uneven.stroke_variation
        );
        assert!(
            !judge(Some(uneven), &Floors::MEASURED),
            "an unevenly inked but straight printed row was called handwriting"
        );
    }

    /// A straight line at an angle is what every photograph of a receipt
    /// produces, and it must not be evidence of anything.
    #[test]
    fn a_tilted_line_of_print_is_not_a_wandering_one() {
        let (w, h) = (280u32, 80u32);
        let mut img = GrayImage::from_pixel(w, h, image::Luma([255u8]));
        let mut x = 8u32;
        let mut step = 0u32;
        while x + 4 < w - 8 {
            let foot = 50 + step;
            for y in (10 + step)..foot {
                for dx in 0..4 {
                    img.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            x += 12;
            step += 1;
        }
        let ink = measure(&img).expect("measurable");
        assert!(
            ink.baseline_drift < Floors::MEASURED.baseline_drift,
            "a straight tilted line reported drift {}",
            ink.baseline_drift
        );
    }

    #[test]
    fn never_marks_nothing_and_measured_marks_something() {
        let ink = measure(&written(420, 72));
        assert!(judge(ink, &Floors::MEASURED));
        assert!(
            !judge(ink, &Floors::never()),
            "`never` has to actually mark nothing, or every measurement taken \
             against it is meaningless"
        );
    }

    #[test]
    fn the_page_reference_is_a_median_not_a_mean() {
        // Four confident boxes and two marked ones. A mean would be dragged to
        // 0.71 by the two; the median stays with the neighbours.
        let page = page_reference(&[0.98, 0.97, 0.96, 0.95, 0.20, 0.22]);
        assert!(page > 0.9, "got {page}");
        assert_eq!(page_reference(&[]), 0.0);
    }

    #[test]
    fn the_mark_carries_no_value_a_reader_could_use() {
        assert!(!HUMAN_MARK.chars().any(|c| c.is_ascii_digit()));
        assert!(!HUMAN_MARK.is_empty());
    }
}

#[cfg(test)]
mod refit_tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;

    /// **A straight printed row with descenders on one end must not measure as
    /// a wandering one.**
    ///
    /// This is what the refit inside [`baseline_drift`] is for. An ordinary
    /// least squares line through the foot of every ink column is dragged by the
    /// few columns a `p`, a `y` or a comma reaches below the line — and once the
    /// line has tilted, every *other* column is off it, so a perfectly straight
    /// row reports drift.
    ///
    /// Falsified by setting the outlier cut to zero, which turns the refit off:
    /// this goes red. It is also the change that doubled the false positives on
    /// the real corpus, from 2 of 268 to 4 of 268, with the printed median drift
    /// rising 0.113 to 0.176 — the same effect, at scale.
    #[test]
    fn descenders_at_one_end_do_not_tilt_the_baseline() {
        let (w, h) = (300u32, 90u32);
        let mut img = GrayImage::from_pixel(w, h, image::Luma([255u8]));
        let mut x = 8u32;
        while x + 4 < w - 8 {
            // Every stem the same width, every foot on the same row.
            for y in 10..50 {
                for dx in 0..4 {
                    img.put_pixel(x + dx, y, image::Luma([0u8]));
                }
            }
            // Four descenders, all at the right-hand end.
            if x > w - 70 {
                for y in 50..78 {
                    for dx in 0..4 {
                        img.put_pixel(x + dx, y, image::Luma([0u8]));
                    }
                }
            }
            x += 12;
        }

        let ink = measure(&img).expect("measurable");
        assert!(
            ink.baseline_drift < Floors::MEASURED.baseline_drift,
            "a straight row with descenders reported drift {}",
            ink.baseline_drift
        );
        assert!(!judge(Some(ink), &Floors::MEASURED));
    }
}
