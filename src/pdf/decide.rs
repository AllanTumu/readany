//! Deciding, per page, whether to read the text layer or rasterise.
//!
//! **Per page, never per document.** A bank statement often has a generated
//! first page and a photocopied appendix; deciding once for the whole file
//! either wastes seconds of OCR on pages that carry perfect text, or hands back
//! a blank appendix and calls the document complete.
//!
//! ## The thresholds were measured, not guessed
//!
//! Three numbers per page across the corpus, on 9 August 2026:
//!
//! | File | Page | Text items | Text area | Image area |
//! |---|---|---|---|---|
//! | CaixaBank | 1–3 | 91 | 0.089 | 0.006 |
//! | Uganda | 1–3 | 104–111 | **0.000** | 0.084 |
//! | Revolut | 1–3 | 117–160 | 0.151 | 0.008 |
//! | Mercury | 1–2 | 15–24 | 0.019–0.047 | 0.000 |
//! | Scanned photo | 1 | **0** | 0.000 | **1.000** |
//!
//! ## What the data changed
//!
//! The obvious design is to threshold on text *area*: a page whose glyphs cover
//! little of it must be a scan. **The Uganda statement falsifies that.** Its
//! text items all report `width = 0.0` — the PDF uses a font whose widths
//! `pdf-inspector` cannot measure — so its text area is 0.000 on a page
//! carrying 104 perfectly readable items. An area threshold would send 13 pages
//! of a statement that verifies 748/748 to OCR, and the failure would look like
//! bad OCR rather than a bad decision.
//!
//! So **item count is the primary signal and area is corroborating only**. A
//! count cannot be destroyed by an unmeasurable font.
//!
//! The separation on count is wide enough that no fine tuning is warranted:
//! real text pages carry 15 to 160 items, a scanned page carries 0. The cut is
//! placed at 8, which is below every text page measured and far above zero,
//! rather than at a midpoint that would look precise and mean nothing.

/// What to do with one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagePlan {
    /// The page carries its own text. Read it; cost is microseconds.
    UseTextLayer,
    /// Text exists but sits on top of a full-page image.
    ///
    /// Some scanner wrote it — this is a searchable PDF, produced by OCR that
    /// was not ours. Use it, because it is almost always better than
    /// re-recognising a JPEG of the same page, but **mark it**: we did not
    /// make this text and cannot vouch for the engine that did.
    UseTextLayerOverImage,
    /// A picture. Send it to OCR.
    Rasterise,
    /// Nothing on it. Report it as blank.
    ///
    /// Not rasterised. Spending four seconds of OCR on white paper is a bug,
    /// and reporting a blank page as unresolved would be a false alarm.
    Blank,
}

/// What was measured about one page.
///
/// Carried out of the decision so a caller can see *why*, and so the numbers
/// can be printed for a corpus without re-running the decision logic.
#[derive(Debug, Clone, Copy)]
pub struct PageEvidence {
    /// Number of text items. The primary signal: robust against a font whose
    /// widths cannot be measured, which zeroes the area signals.
    pub text_items: usize,
    /// Fraction of the page covered by text bounding boxes. Corroborating
    /// only — see the module note on the Uganda statement.
    pub text_coverage: f32,
    /// Fraction of the page covered by image objects.
    pub image_coverage: f32,
}

/// Fewer text items than this and the page is not carrying its own text.
///
/// Measured: text pages in the corpus carry 15–160 items; a scanned page
/// carries 0. Placed below the lowest real page rather than midway, so a
/// sparse but genuine page — a cover sheet, a short appendix — is never sent
/// to OCR.
pub const MIN_TEXT_ITEMS: usize = 8;

/// Image coverage above this means a full-page picture underneath.
///
/// A logo or a signature strip covers a few percent; the corpus measures
/// 0.006–0.084 for those. A scanned page measures 1.000. Half separates them
/// with an enormous margin.
pub const FULL_PAGE_IMAGE_COVERAGE: f32 = 0.5;

/// Decide what to do with one page.
pub fn plan(evidence: PageEvidence) -> PagePlan {
    let has_text = evidence.text_items >= MIN_TEXT_ITEMS;
    let has_full_page_image = evidence.image_coverage >= FULL_PAGE_IMAGE_COVERAGE;

    match (has_text, has_full_page_image) {
        // Text sitting on a full-page image: a searchable PDF somebody else
        // made. Usable, and flagged.
        (true, true) => PagePlan::UseTextLayerOverImage,
        // Ordinary generated page.
        (true, false) => PagePlan::UseTextLayer,
        // A picture with no text: the case this whole pipe exists for.
        (false, true) => PagePlan::Rasterise,
        // Neither text nor a picture. Before calling it blank, check whether
        // there is *any* ink at all — a page with two items and a small image
        // is sparse, not empty, and rasterising it may still recover something.
        (false, false) => {
            if evidence.text_items == 0 && evidence.image_coverage <= 0.01 {
                PagePlan::Blank
            } else {
                PagePlan::Rasterise
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(items: usize, text: f32, image: f32) -> PageEvidence {
        PageEvidence { text_items: items, text_coverage: text, image_coverage: image }
    }

    #[test]
    fn a_generated_page_uses_its_text_layer() {
        // CaixaBank, measured.
        assert_eq!(plan(ev(91, 0.089, 0.006)), PagePlan::UseTextLayer);
        // Revolut, measured.
        assert_eq!(plan(ev(117, 0.151, 0.008)), PagePlan::UseTextLayer);
        // Mercury, the sparsest real page in the corpus.
        assert_eq!(plan(ev(15, 0.019, 0.000)), PagePlan::UseTextLayer);
    }

    #[test]
    fn a_font_with_unmeasurable_widths_still_uses_its_text_layer() {
        // The Uganda statement: 104 items, and text_coverage 0.000 because
        // every item reports width 0. Deciding on area would send 13 pages of
        // a statement that verifies 748/748 to OCR.
        assert_eq!(plan(ev(104, 0.000, 0.084)), PagePlan::UseTextLayer);
    }

    #[test]
    fn a_scanned_page_is_rasterised() {
        // A photograph converted to PDF: no text at all, image covers the page.
        assert_eq!(plan(ev(0, 0.000, 1.000)), PagePlan::Rasterise);
    }

    #[test]
    fn a_searchable_pdf_is_used_but_marked() {
        // Text over a full-page scan: somebody else's OCR.
        assert_eq!(plan(ev(120, 0.10, 0.98)), PagePlan::UseTextLayerOverImage);
    }

    #[test]
    fn a_blank_page_is_not_rasterised() {
        // Four seconds of OCR on white paper is a bug.
        assert_eq!(plan(ev(0, 0.0, 0.0)), PagePlan::Blank);
    }

    #[test]
    fn a_sparse_page_is_rasterised_rather_than_called_blank() {
        // Two stray items and a small image is not empty. Rasterising may
        // recover something; calling it blank certainly recovers nothing.
        assert_eq!(plan(ev(2, 0.001, 0.05)), PagePlan::Rasterise);
    }

    /// Guards the threshold itself: the sparsest real page in the corpus is
    /// Mercury page 2 at 15 text items, so raising `MIN_TEXT_ITEMS` to or above
    /// that would start sending real text pages to OCR. Checked at compile
    /// time, so it cannot be skipped.
    const _: () = assert!(
        MIN_TEXT_ITEMS < 15,
        "MIN_TEXT_ITEMS must stay below the sparsest measured text page"
    );
}
