//! Shared types. Deliberately close to pdf-inspector's `TextItem` so results
//! from a scanned page and a text PDF can be merged without translation.

/// A quadrilateral in image pixel coordinates, clockwise from top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub points: [(f32, f32); 4],
}

impl Quad {
    pub fn from_rect(x: f32, y: f32, w: f32, h: f32) -> Self {
        Quad {
            points: [(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
        }
    }

    /// Axis-aligned bounding box: (x, y, width, height).
    pub fn bbox(&self) -> (f32, f32, f32, f32) {
        let xs = self.points.iter().map(|p| p.0);
        let ys = self.points.iter().map(|p| p.1);
        let min_x = xs.clone().fold(f32::MAX, f32::min);
        let max_x = xs.fold(f32::MIN, f32::max);
        let min_y = ys.clone().fold(f32::MAX, f32::min);
        let max_y = ys.fold(f32::MIN, f32::max);
        (min_x, min_y, max_x - min_x, max_y - min_y)
    }

    pub fn height(&self) -> f32 {
        self.bbox().3
    }

    pub fn center_y(&self) -> f32 {
        let (_, y, _, h) = self.bbox();
        y + h / 2.0
    }

    pub fn left(&self) -> f32 {
        self.bbox().0
    }
}

/// One recognised piece of text with its position and confidence.
#[derive(Debug, Clone)]
pub struct TextBox {
    pub text: String,
    pub quad: Quad,
    /// 0.0 to 1.0, the mean probability of the characters the recogniser kept.
    ///
    /// **A mean over characters cannot see a character that was never
    /// emitted.** Measured on a rendered statement: a merchant name that lost
    /// its inter-word space came back at 0.999, *higher* than the same name
    /// read correctly at 0.975, because the space was the only uncertain step
    /// and dropping it removed the one number that was pulling the mean down.
    /// Confidence answers "was what I read hard to read", never "did I read
    /// everything".
    pub confidence: f32,
}

/// Text boxes grouped into a reading line.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub boxes: Vec<TextBox>,
    /// Regions on this line the engine marked as handwritten and refused to
    /// read. See [`crate::ocr::human::HumanRegion`]: there is no text on that
    /// type, so a marked cell cannot be filled by anything downstream.
    ///
    /// Empty on almost every line. A line can hold both: a printed label and a
    /// figure written in beside it is the whole case this exists for.
    pub human: Vec<super::human::HumanRegion>,
    pub baseline_y: f32,
}

impl TextLine {
    /// The line as text, with every marked region standing in its own place.
    ///
    /// A row whose amount was written by hand comes back as its printed label
    /// followed by [`crate::ocr::human::HUMAN_MARK`], in the column the writing
    /// was in — not as a row that simply had no amount. The two are different
    /// facts and a consumer is entitled to tell them apart: "there was no
    /// figure here" and "there was a figure here and we will not tell you what
    /// it said" lead to different questions being asked of a person.
    ///
    /// When the line has no marked region this is exactly the old
    /// implementation: the boxes' text, joined with spaces.
    pub fn text(&self) -> String {
        if self.human.is_empty() {
            return self
                .boxes
                .iter()
                .map(|b| b.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
        }
        let mut cells: Vec<(f32, &str)> = self
            .boxes
            .iter()
            .map(|b| (b.quad.left(), b.text.as_str()))
            .chain(
                self.human
                    .iter()
                    .map(|r| (r.quad.left(), super::human::HUMAN_MARK)),
            )
            .collect();
        cells.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        cells
            .into_iter()
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The vertical extent of the text on this line, top and bottom.
    ///
    /// `None` for a line that holds no text box, which is a line that is
    /// nothing but a marked region.
    pub fn span(&self) -> Option<(f32, f32)> {
        let mut top = f32::MAX;
        let mut bottom = f32::MIN;
        for b in &self.boxes {
            let (_, y, _, h) = b.quad.bbox();
            top = top.min(y);
            bottom = bottom.max(y + h);
        }
        (bottom > top).then_some((top, bottom))
    }

    /// True when part of this line was marked for a person rather than read.
    ///
    /// Deliberately not folded into [`TextLine::min_confidence`]. A low
    /// confidence says the engine read something badly; this says the engine
    /// declined to read at all, and a caller that thresholds the first would
    /// silently pass a row it should have stopped on.
    pub fn needs_a_person(&self) -> bool {
        !self.human.is_empty()
    }

    pub fn confidence(&self) -> f32 {
        if self.boxes.is_empty() {
            return 0.0;
        }
        self.boxes.iter().map(|b| b.confidence).sum::<f32>() / self.boxes.len() as f32
    }

    /// The confidence of the **weakest** box on this line.
    ///
    /// A line of a bank statement is four boxes: date, description, amount,
    /// balance. Averaging them buries the one that went wrong under three that
    /// did not, and a line is exactly the unit a person is asked to check — so
    /// the number attached to a line has to be the one that moves when any part
    /// of it is doubtful.
    ///
    /// Measured on a generated eighteen-row statement, rendered at 150 dpi and
    /// read through ten damage levels: 497 lines, 69 of which carry a box whose
    /// text is not what the page printed. At the floor this crate already ships
    /// for marking uncertain output, 0.5:
    ///
    /// | Statistic at floor 0.5 | Wrong lines flagged | Right lines flagged |
    /// |---|---|---|
    /// | mean of the line's boxes | **0 of 69** | 0 of 428 |
    /// | minimum of the line's boxes | **18 of 69** | 0 of 428 |
    ///
    /// Eighteen more caught and not one correct line newly accused — the same
    /// constant, a statistic that can move. Raising the floor buys more, and
    /// stops being free above 0.90: minimum at 0.90 flags 32 of 69 wrong lines
    /// and 0 of 428 right ones; at 0.94, 41 of 69 and 9 of 428.
    ///
    /// **What it still cannot see.** Ink bleed at radius 1 changed one letter
    /// of a merchant name while its amount and balance stayed right. That box
    /// read 0.949, inside the range correct boxes on the same page occupy
    /// (down to 0.908), so no floor separates it without accusing several
    /// correct lines. What *does* separate it is the ordering: sorted weakest
    /// first, the four wrong lines on that page came 6th, 7th, 11th and 13th
    /// of 44 — all of them inside the worst third, none of them findable by a
    /// floor. The number is worth more as a ranking than as a gate, and a
    /// caller that can only put a handful of lines in front of a person should
    /// sort by it rather than threshold it.
    ///
    /// Returns 0.0 for an empty line, matching [`TextLine::confidence`]: no
    /// boxes is not high confidence.
    pub fn min_confidence(&self) -> f32 {
        // `f32::min` returns the other operand when one side is NaN, so a
        // single NaN from a backend is stepped over rather than swallowing the
        // line. When the line is empty, or every box is NaN, the fold never
        // moves off its seed — and `INFINITY` is the one answer that must not
        // escape here, because it clears every floor. The first draft of this
        // returned it, and the line that could not be read at all would have
        // reported itself as the best on the page.
        let lowest = self
            .boxes
            .iter()
            .map(|b| b.confidence)
            .fold(f32::INFINITY, f32::min);
        if lowest.is_finite() {
            lowest
        } else {
            0.0
        }
    }

    /// Median glyph height, used to rank headings.
    pub fn height(&self) -> f32 {
        if self.boxes.is_empty() {
            return 0.0;
        }
        let mut hs: Vec<f32> = self.boxes.iter().map(|b| b.quad.height()).collect();
        hs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        hs[hs.len() / 2]
    }
}

/// What the caller gets back.
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub lines: Vec<TextLine>,
    pub width: u32,
    pub height: u32,
    /// Rotation applied before recognition, in degrees: 0, 90, 180 or 270.
    pub rotation: u16,
    /// Skew corrected before recognition, in degrees.
    pub skew: f32,
    pub processing_time_ms: u64,
    /// Straightening the page: decode, orientation, deskew.
    pub prepare_ms: u64,
    /// Finding where the text is. One pass over the whole page.
    pub detect_ms: u64,
    /// Reading the text. One call per box, or one per batch.
    pub recognize_ms: u64,
}

impl ScanResult {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Mean confidence across every recognised box.
    pub fn confidence(&self) -> f32 {
        let boxes: Vec<&TextBox> = self.lines.iter().flat_map(|l| l.boxes.iter()).collect();
        if boxes.is_empty() {
            return 0.0;
        }
        boxes.iter().map(|b| b.confidence).sum::<f32>() / boxes.len() as f32
    }

    /// The confidence of the **weakest** box anywhere on the page.
    ///
    /// [`ScanResult::confidence`] averages every box on the page, and a
    /// rendered A4 statement produces eighty-two of them, so one bad box moves
    /// that mean by about a thousandth. Measured on the page whose ink bleed
    /// changed a letter of a merchant name while every amount stayed right:
    /// the page mean fell from 0.9904 to 0.9847, five thousandths, and nothing
    /// downstream can act on five thousandths. This number fell from 0.9551 to
    /// 0.9075 — eight times the movement, from one page number that cannot be
    /// wrong enough to another that at least can.
    ///
    /// It is still only a page number. The signal a caller can use is per
    /// line and is an ordering rather than a gate; see
    /// [`TextLine::min_confidence`].
    ///
    /// Returns 0.0 for a page with no lines.
    pub fn min_confidence(&self) -> f32 {
        // Over boxes rather than over `TextLine::min_confidence`, because an
        // empty line answers 0.0 and would drag the whole page to it. See
        // that method for why a non-finite fold seed must not escape.
        let lowest = self
            .lines
            .iter()
            .flat_map(|l| l.boxes.iter())
            .map(|b| b.confidence)
            .fold(f32::INFINITY, f32::min);
        if lowest.is_finite() {
            lowest
        } else {
            0.0
        }
    }

    /// Every box below `floor`, as the index of the line it sits on and a
    /// reference to the box itself.
    ///
    /// This is the shape a consumer needs to **mark** a field: the line index
    /// says which row, the quad says which cell, and the text is there so the
    /// mark can sit beside what was actually read. Nothing here can change a
    /// value — the references are shared, deliberately. A low confidence is
    /// grounds for asking a person; it is never grounds for the engine to
    /// substitute what it would rather have read.
    ///
    /// Lines are in reading order, and boxes within a line are left to right,
    /// so the result is in the order a person would check them.
    ///
    /// See [`TextLine::min_confidence`] for what a floor buys, measured. In
    /// short: 0.5 is free and catches a quarter of the wrong lines, 0.90 is
    /// still free and catches a half, and above that a caller starts paying in
    /// correct lines it has accused. On the ink-bleed page named there, a
    /// floor of 0.95 returned 7 boxes of 82 and two of the page's four
    /// misreadings were among them — which is the honest shape of this: a
    /// short list to check, not an answer.
    pub fn weak_boxes(&self, floor: f32) -> Vec<(usize, &TextBox)> {
        self.lines
            .iter()
            .enumerate()
            .flat_map(|(n, l)| l.boxes.iter().map(move |b| (n, b)))
            .filter(|(_, b)| b.confidence < floor)
            .collect()
    }

    /// Every region marked as handwritten, as the index of the line it sits on
    /// and a reference to the region.
    ///
    /// The same shape as [`ScanResult::weak_boxes`] and for the same reason:
    /// the line index says which row and the quad says which cell, so a person
    /// can be shown *where*. The references are shared — but here that is not
    /// the guarantee, because [`crate::ocr::human::HumanRegion`] has no text
    /// field to fill even with a mutable one. A weak box is a reading the
    /// engine is unsure of; a marked region is not a reading at all.
    ///
    /// Lines are in reading order and regions within a line are left to right.
    pub fn human_regions(&self) -> Vec<(usize, &super::human::HumanRegion)> {
        self.lines
            .iter()
            .enumerate()
            .flat_map(|(n, l)| l.human.iter().map(move |r| (n, r)))
            .collect()
    }

    /// True when any part of this page was marked for a person.
    pub fn needs_a_person(&self) -> bool {
        self.lines.iter().any(TextLine::needs_a_person)
    }

    /// True when the caller should not trust this result.
    ///
    /// This asks about the page as a whole, and it answers with a mean, so it
    /// is the wrong question to ask about one wrong word — see
    /// [`ScanResult::min_confidence`] and [`ScanResult::weak_boxes`] for that.
    /// It is left on the mean on purpose: it drives the retry in
    /// [`crate::ocr::Engine::scan_bytes`], which exists for pages that came
    /// back as nonsense *entirely*, and a minimum would fire that second pass
    /// on pages that only had one hard glyph.
    pub fn needs_review(&self, threshold: f32) -> bool {
        self.lines.is_empty() || self.confidence() < threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(text: &str, x: f32, confidence: f32) -> TextBox {
        TextBox {
            text: text.into(),
            quad: Quad::from_rect(x, 10.0, 40.0, 12.0),
            confidence,
        }
    }

    fn line(confidences: &[f32]) -> TextLine {
        TextLine {
            boxes: confidences
                .iter()
                .enumerate()
                .map(|(i, &c)| tb("cell", i as f32 * 50.0, c))
                .collect(),
            human: Vec::new(),
            baseline_y: 16.0,
        }
    }

    /// The whole point, stated as the numbers that were actually measured.
    ///
    /// A statement line is four cells. The engine read the merchant wrongly and
    /// the date, amount and balance correctly, so the mean sits where three
    /// good cells put it and says nothing.
    #[test]
    fn a_line_reports_its_weakest_cell_not_its_average() {
        let l = line(&[0.999, 0.471, 0.997, 0.996]);
        assert!(
            l.confidence() > 0.86,
            "the mean is dragged by one cell in four, not led by it: {}",
            l.confidence()
        );
        assert!(
            (l.min_confidence() - 0.471).abs() < 1e-6,
            "got {}",
            l.min_confidence()
        );
        // The claim that matters: at the floor this crate ships, the two
        // statistics disagree about this line.
        assert!(l.confidence() >= 0.5, "the mean would not have flagged it");
        assert!(l.min_confidence() < 0.5, "the minimum must flag it");
    }

    /// A line whose cells are all fine must not be flagged by either. Without
    /// this the test above would pass for a `min_confidence` that returned a
    /// constant 0.
    #[test]
    fn a_line_of_good_cells_is_flagged_by_neither() {
        let l = line(&[0.999, 0.981, 0.997, 0.996]);
        assert!(l.min_confidence() >= 0.5);
        assert!(l.min_confidence() > 0.98, "got {}", l.min_confidence());
    }

    #[test]
    fn an_empty_line_is_not_confident() {
        // No boxes is not high confidence, and it must not be `INFINITY`
        // either — a caller comparing against a floor would read that as the
        // best line on the page.
        let l = TextLine {
            boxes: Vec::new(),
            human: Vec::new(),
            baseline_y: 0.0,
        };
        assert_eq!(l.min_confidence(), 0.0);
        assert_eq!(l.confidence(), 0.0);
    }

    #[test]
    fn a_nan_from_a_backend_does_not_swallow_the_line() {
        // The obvious way to write a minimum over floats in Rust is
        // `min_by(|a, b| a.partial_cmp(b).unwrap())`, and `partial_cmp`
        // returns `None` for NaN — so that spelling **panics**, on the one
        // page where something has already gone wrong. The fold over
        // `f32::min` returns the other operand instead, so a NaN is stepped
        // over and the real minimum survives. NaN is placed first, in the
        // middle and last, because a fold's answer depends on where it lands.
        for cells in [
            vec![f32::NAN, 0.9, 0.4],
            vec![0.9, f32::NAN, 0.4],
            vec![0.9, 0.4, f32::NAN],
        ] {
            let got = line(&cells).min_confidence();
            assert!((got - 0.4).abs() < 1e-6, "{cells:?} gave {got}");
            assert!(got < 0.5, "{cells:?} gave {got}");
        }
        // A line that is nothing but NaN has no usable minimum. A caller flags
        // with `value < floor`, and both `INFINITY < 0.5` and `NaN < 0.5` are
        // false — so either answer would let that line through untouched. The
        // assertion is written the way a caller writes the test, deliberately.
        let all_nan = line(&[f32::NAN, f32::NAN]).min_confidence();
        assert!(
            all_nan < 0.5,
            "a line of NaN reported {all_nan}, which a floor would wave through"
        );
    }

    fn page(lines: Vec<TextLine>) -> ScanResult {
        ScanResult {
            lines,
            width: 1239,
            height: 1754,
            rotation: 0,
            skew: 0.0,
            processing_time_ms: 0,
            prepare_ms: 0,
            detect_ms: 0,
            recognize_ms: 0,
        }
    }

    /// Eighty-two boxes is what a rendered A4 statement produces, and one bad
    /// box among them moves the page mean by about four thousandths. Measured:
    /// the page that changed a letter of a merchant name reported 0.986
    /// against the clean render's 0.990.
    #[test]
    fn one_bad_box_is_invisible_to_the_page_mean_and_not_to_the_page_minimum() {
        let mut lines: Vec<TextLine> = (0..20).map(|_| line(&[0.99, 0.99, 0.99, 0.99])).collect();
        lines.push(line(&[0.99, 0.471]));
        let p = page(lines);

        assert!(
            p.confidence() > 0.98,
            "a mean over 82 boxes cannot represent one of them: {}",
            p.confidence()
        );
        assert!(
            !p.needs_review(0.5),
            "and so the page declares itself fine, which is the bug"
        );
        assert!(
            (p.min_confidence() - 0.471).abs() < 1e-6,
            "got {}",
            p.min_confidence()
        );
    }

    #[test]
    fn an_empty_page_has_no_minimum_rather_than_a_perfect_one() {
        assert_eq!(page(Vec::new()).min_confidence(), 0.0);
        assert_eq!(page(vec![line(&[])]).min_confidence(), 0.0);
    }

    /// What a consumer is handed: which row, and which cell in it.
    #[test]
    fn weak_boxes_names_the_row_and_the_cell() {
        let p = page(vec![
            line(&[0.99, 0.99]),
            line(&[0.99, 0.471, 0.99]),
            line(&[0.99, 0.99]),
        ]);

        let weak = p.weak_boxes(0.5);
        assert_eq!(weak.len(), 1, "exactly one cell is below the floor");
        assert_eq!(weak[0].0, 1, "and it is on the second line");
        assert!((weak[0].1.confidence - 0.471).abs() < 1e-6);
        // The cell can be pointed at on the page, which is what "flag a cell"
        // needs: a person has to be shown *where*.
        assert_eq!(weak[0].1.quad.left(), 50.0);

        // A floor no cell is under finds nothing. Without this the assertion
        // above would pass for a `weak_boxes` that returned everything.
        assert!(p.weak_boxes(0.4).is_empty());
        assert_eq!(p.weak_boxes(1.0).len(), 7, "and one under every cell finds all");
    }

    /// Reading order, because a person checking flagged cells reads the page
    /// the way the page is laid out.
    #[test]
    fn weak_boxes_come_back_in_reading_order() {
        let p = page(vec![line(&[0.2, 0.3]), line(&[0.1])]);
        let weak = p.weak_boxes(0.5);
        let seen: Vec<(usize, f32)> = weak.iter().map(|(n, b)| (*n, b.confidence)).collect();
        assert_eq!(seen, vec![(0, 0.2), (0, 0.3), (1, 0.1)]);
    }
}
