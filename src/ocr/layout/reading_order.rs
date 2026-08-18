//! Reading order.
//!
//! Detection returns boxes in no useful order. Columns must be separated
//! **before** boxes are grouped into lines, otherwise a line on the left of a
//! two-column page merges with the line beside it on the right and the text
//! comes out interleaved. So the order is: find the gutters, split the boxes
//! into columns, group each column into lines, then read columns left to
//! right and lines top to bottom.

use crate::ocr::types::{TextBox, TextLine};

/// Two boxes belong to the same line when their vertical spans overlap by at
/// least this fraction of the shorter box.
const LINE_OVERLAP: f32 = 0.5;

/// Centres may differ by at most this fraction of the shorter box height.
/// A detector box stretched across two receipt rows otherwise overlaps both
/// and becomes a bridge that collapses the rows into one.
const LINE_CENTRE_DISTANCE: f32 = 0.75;

/// A gutter must be at least this fraction of the page wide to count.
const MIN_GUTTER: f32 = 0.04;

/// The number of vertical slices used to look for gutters.
const BINS: usize = 100;

/// Turn detected boxes into ordered lines.
pub fn assemble(boxes: Vec<TextBox>, page_width: f32) -> Vec<TextLine> {
    if boxes.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for column in split_columns(boxes, page_width) {
        out.extend(group_into_lines(column));
    }
    out
}

/// Group boxes that are already known to belong to one column.
pub fn group_into_lines(mut boxes: Vec<TextBox>) -> Vec<TextLine> {
    if boxes.is_empty() {
        return Vec::new();
    }
    boxes.sort_by(|a, b| {
        a.quad
            .center_y()
            .partial_cmp(&b.quad.center_y())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut lines: Vec<Vec<TextBox>> = Vec::new();
    for b in boxes {
        match lines.last_mut() {
            Some(line) if overlaps(line.last().expect("never empty"), &b) => line.push(b),
            _ => lines.push(vec![b]),
        }
    }

    lines
        .into_iter()
        .map(|mut line| {
            line.sort_by(|a, b| {
                a.quad
                    .left()
                    .partial_cmp(&b.quad.left())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let baseline_y =
                line.iter().map(|b| b.quad.center_y()).sum::<f32>() / line.len() as f32;
            TextLine {
                boxes: line,
                human: Vec::new(),
                baseline_y,
            }
        })
        .collect()
}

fn overlaps(a: &TextBox, b: &TextBox) -> bool {
    let (_, ay, _, ah) = a.quad.bbox();
    let (_, by, _, bh) = b.quad.bbox();
    let top = ay.max(by);
    let bottom = (ay + ah).min(by + bh);
    let shared = (bottom - top).max(0.0);
    let shorter = ah.min(bh).max(1.0);
    let centres_apart = (a.quad.center_y() - b.quad.center_y()).abs();
    shared / shorter >= LINE_OVERLAP && centres_apart / shorter <= LINE_CENTRE_DISTANCE
}

/// Split boxes into columns at every vertical band no box crosses.
/// A page with no gutter comes back as a single column.
fn split_columns(boxes: Vec<TextBox>, page_width: f32) -> Vec<Vec<TextBox>> {
    if boxes.len() < 4 || page_width <= 0.0 {
        return vec![boxes];
    }

    let bin_width = page_width / BINS as f32;
    let mut occupied = [false; BINS];
    for b in &boxes {
        let (x, _, w, _) = b.quad.bbox();
        let from = ((x / bin_width).floor().max(0.0) as usize).min(BINS - 1);
        let to = (((x + w) / bin_width).ceil() as usize).min(BINS);
        for slot in occupied.iter_mut().take(to).skip(from) {
            *slot = true;
        }
    }

    let boundaries = gutter_boundaries(&occupied, bin_width);
    if boundaries.is_empty() {
        return vec![boxes];
    }

    let mut columns: Vec<Vec<TextBox>> = vec![Vec::new(); boundaries.len() + 1];
    for b in boxes {
        let centre = {
            let (x, _, w, _) = b.quad.bbox();
            x + w / 2.0
        };
        let index = boundaries.iter().filter(|&&g| centre > g).count();
        columns[index].push(b);
    }
    columns.retain(|c| !c.is_empty());
    columns
}

/// Put each marked region on the line it belongs to.
///
/// Marked regions are held out of [`assemble`] because they are not text and
/// [`TextLine`] holds text — but a region that is not attached to a row is
/// almost useless. "There is handwriting somewhere on this receipt" does not
/// tell anyone which figure is missing; "the row that says `TOTAL` has a
/// handwritten cell in it" does.
///
/// A region joins the line whose vertical span it overlaps most, using the same
/// `LINE_OVERLAP` rule the text boxes were grouped by, so a written figure
/// beside a printed label lands on the label's row. A region that overlaps no
/// line becomes a line of its own, in reading order — writing in a margin is
/// still writing, and dropping it because it sat beside nothing is exactly the
/// silent omission this engine refuses.
pub fn attach_human(lines: &mut Vec<TextLine>, regions: Vec<crate::ocr::human::HumanRegion>) {
    for region in regions {
        let (_, ry, _, rh) = region.quad.bbox();
        let best = lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| {
                let (top, bottom) = line.span()?;
                let shared = (bottom.min(ry + rh) - top.max(ry)).max(0.0);
                let shorter = (bottom - top).min(rh).max(1.0);
                let fraction = shared / shorter;
                (fraction >= LINE_OVERLAP).then_some((i, fraction))
            })
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i);

        match best.and_then(|i| lines.get_mut(i)) {
            Some(line) => {
                line.human.push(region);
                line.human.sort_by(|a, b| {
                    a.quad
                        .left()
                        .partial_cmp(&b.quad.left())
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            None => {
                let baseline_y = region.quad.center_y();
                let at = lines
                    .iter()
                    .position(|l| l.baseline_y > baseline_y)
                    .unwrap_or(lines.len());
                lines.insert(
                    at,
                    TextLine {
                        boxes: Vec::new(),
                        human: vec![region],
                        baseline_y,
                    },
                );
            }
        }
    }
}

/// Midpoints of every run of empty bins wide enough to be a gutter,
/// ignoring the page margins.
fn gutter_boundaries(occupied: &[bool; BINS], bin_width: f32) -> Vec<f32> {
    let min_run = ((BINS as f32) * MIN_GUTTER).ceil() as usize;
    let (lo, hi) = (BINS * 15 / 100, BINS * 85 / 100);

    let mut boundaries = Vec::new();
    let mut run_start: Option<usize> = None;

    for (i, &busy) in occupied.iter().enumerate().take(hi).skip(lo) {
        if !busy {
            run_start.get_or_insert(i);
        } else if let Some(start) = run_start.take() {
            if i - start >= min_run {
                boundaries.push((start + (i - start) / 2) as f32 * bin_width);
            }
        }
    }
    if let Some(start) = run_start {
        if hi - start >= min_run {
            boundaries.push((start + (hi - start) / 2) as f32 * bin_width);
        }
    }
    boundaries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::types::Quad;

    fn tb(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextBox {
        TextBox {
            text: text.into(),
            quad: Quad::from_rect(x, y, w, h),
            confidence: 0.9,
        }
    }

    fn texts(lines: &[TextLine]) -> Vec<String> {
        lines.iter().map(|l| l.text()).collect()
    }

    #[test]
    fn boxes_on_the_same_row_become_one_line() {
        let lines = group_into_lines(vec![
            tb("world", 60.0, 10.0, 50.0, 12.0),
            tb("hello", 10.0, 11.0, 40.0, 12.0),
        ]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text(), "hello world");
    }

    #[test]
    fn separate_rows_stay_separate() {
        let lines = group_into_lines(vec![
            tb("one", 10.0, 10.0, 40.0, 12.0),
            tb("two", 10.0, 40.0, 40.0, 12.0),
        ]);
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn a_tall_detection_box_does_not_bridge_two_receipt_rows() {
        let lines = group_into_lines(vec![
            // The detector read the first description from one crop whose
            // height reaches into the following row.
            tb("1 CORN", 10.0, 10.0, 80.0, 30.0),
            tb("2,20", 160.0, 10.0, 35.0, 12.0),
            tb("1 B.LOLOT", 10.0, 29.0, 90.0, 12.0),
            tb("2,30", 160.0, 29.0, 35.0, 12.0),
        ]);

        assert_eq!(texts(&lines), vec!["1 CORN 2,20", "1 B.LOLOT 2,30"]);
    }

    #[test]
    fn two_columns_are_read_one_after_the_other() {
        let mut boxes = Vec::new();
        for i in 0..6 {
            let y = 10.0 + i as f32 * 20.0;
            boxes.push(tb(&format!("L{i}"), 10.0, y, 150.0, 12.0));
            boxes.push(tb(&format!("R{i}"), 250.0, y, 150.0, 12.0));
        }
        assert_eq!(
            texts(&assemble(boxes, 420.0)),
            vec!["L0", "L1", "L2", "L3", "L4", "L5", "R0", "R1", "R2", "R3", "R4", "R5"]
        );
    }

    #[test]
    fn three_columns_are_handled() {
        let mut boxes = Vec::new();
        for i in 0..4 {
            let y = 10.0 + i as f32 * 20.0;
            boxes.push(tb(&format!("A{i}"), 10.0, y, 80.0, 12.0));
            boxes.push(tb(&format!("B{i}"), 160.0, y, 80.0, 12.0));
            boxes.push(tb(&format!("C{i}"), 310.0, y, 80.0, 12.0));
        }
        let out = texts(&assemble(boxes, 420.0));
        assert_eq!(&out[0..4], &["A0", "A1", "A2", "A3"]);
        assert_eq!(&out[4..8], &["B0", "B1", "B2", "B3"]);
        assert_eq!(&out[8..12], &["C0", "C1", "C2", "C3"]);
    }

    #[test]
    fn single_column_keeps_top_to_bottom_order() {
        let boxes: Vec<TextBox> = (0..6)
            .map(|i| {
                tb(
                    &format!("line{i}"),
                    10.0,
                    10.0 + i as f32 * 20.0,
                    380.0,
                    12.0,
                )
            })
            .collect();
        let out = texts(&assemble(boxes, 420.0));
        assert_eq!(out.first().map(String::as_str), Some("line0"));
        assert_eq!(out.last().map(String::as_str), Some("line5"));
        assert_eq!(out.len(), 6);
    }

    #[test]
    fn a_headline_spanning_both_columns_does_not_create_a_gutter() {
        // A banner across the top touches every bin, so no gutter is found
        // beneath it and the page stays one column.
        let mut boxes = vec![tb("BANNER HEADLINE", 10.0, 5.0, 400.0, 20.0)];
        for i in 0..4 {
            let y = 40.0 + i as f32 * 20.0;
            boxes.push(tb(&format!("L{i}"), 10.0, y, 150.0, 12.0));
            boxes.push(tb(&format!("R{i}"), 250.0, y, 150.0, 12.0));
        }
        let out = texts(&assemble(boxes, 420.0));
        assert_eq!(out[0], "BANNER HEADLINE");
        assert_eq!(out[1], "L0 R0");
    }

    #[test]
    fn empty_input_is_safe() {
        assert!(group_into_lines(Vec::new()).is_empty());
        assert!(assemble(Vec::new(), 100.0).is_empty());
    }
}
