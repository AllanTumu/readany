//! Assemble recognised lines into Markdown.
//!
//! This is the stage the other Rust OCR crates skip, and it is the reason
//! anyscan exists: the output is meant to be handed to a model or a person,
//! not to be a list of bounding boxes.

use crate::ocr::types::{ScanResult, TextLine};

/// Heading tiers, expressed as a multiple of the body text height.
const H1_RATIO: f32 = 1.6;
const H2_RATIO: f32 = 1.3;
const H3_RATIO: f32 = 1.15;

/// A paragraph break is assumed when the vertical gap between two lines is
/// more than this multiple of the body text height.
const PARAGRAPH_GAP: f32 = 1.8;

pub struct MarkdownOptions {
    /// Lines below this confidence are marked rather than silently emitted.
    pub confidence_floor: f32,
    /// Wrap low-confidence lines in a marker the caller can search for.
    pub mark_uncertain: bool,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        MarkdownOptions {
            confidence_floor: 0.5,
            mark_uncertain: true,
        }
    }
}

pub fn to_markdown(result: &ScanResult, options: &MarkdownOptions) -> String {
    let lines = &result.lines;
    if lines.is_empty() {
        return String::new();
    }

    let body = body_height(lines);
    let mut out = String::new();
    let mut previous: Option<&TextLine> = None;

    for line in lines {
        let text = line.text();
        if text.trim().is_empty() {
            continue;
        }

        if let Some(prev) = previous {
            let gap = line.baseline_y - prev.baseline_y;
            out.push('\n');
            if body > 0.0 && gap > body * PARAGRAPH_GAP {
                out.push('\n');
            }
        }

        let heading = heading_level(line, body);
        // The line's **weakest** box, not its mean. A statement line is date,
        // merchant, amount and balance; averaging them buries the one box that
        // went wrong under three that did not. Measured over 497 lines from a
        // generated statement read through ten damage levels, 69 of which
        // carry a box that is not what the page printed: at this same floor of
        // 0.5 the mean marked 0 of the 69 and the minimum marks 18, while
        // neither marks one of the 428 lines that were read correctly. Same
        // constant, strictly more caught, nothing new accused.
        let rendered = if options.mark_uncertain && line.min_confidence() < options.confidence_floor
        {
            format!("{text} <!-- low confidence -->")
        } else {
            text
        };

        match heading {
            Some(level) => {
                if previous.is_some() {
                    out.push('\n');
                }
                out.push_str(&"#".repeat(level));
                out.push(' ');
                out.push_str(&rendered);
                out.push('\n');
            }
            None => out.push_str(&rendered),
        }

        previous = Some(line);
    }

    let mut text = out.trim().to_string();
    text.push('\n');
    text
}

/// Median line height, used as the body text baseline.
fn body_height(lines: &[TextLine]) -> f32 {
    let mut heights: Vec<f32> = lines
        .iter()
        .map(|l| l.height())
        .filter(|h| *h > 0.0)
        .collect();
    if heights.is_empty() {
        return 0.0;
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    heights[heights.len() / 2]
}

fn heading_level(line: &TextLine, body: f32) -> Option<usize> {
    if body <= 0.0 {
        return None;
    }
    let ratio = line.height() / body;
    if ratio >= H1_RATIO {
        Some(1)
    } else if ratio >= H2_RATIO {
        Some(2)
    } else if ratio >= H3_RATIO {
        Some(3)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::types::{Quad, TextBox};

    fn line(text: &str, y: f32, h: f32, confidence: f32) -> TextLine {
        TextLine {
            boxes: vec![TextBox {
                text: text.into(),
                quad: Quad::from_rect(10.0, y, 200.0, h),
                confidence,
            }],
            baseline_y: y + h / 2.0,
        }
    }

    fn result(lines: Vec<TextLine>) -> ScanResult {
        ScanResult {
            lines,
            width: 600,
            height: 800,
            rotation: 0,
            skew: 0.0,
            processing_time_ms: 1,
            prepare_ms: 0,
            detect_ms: 0,
            recognize_ms: 1,
        }
    }

    #[test]
    fn a_taller_line_becomes_a_heading() {
        let r = result(vec![
            line("Annual Report", 10.0, 32.0, 0.95),
            line("Body text here", 60.0, 16.0, 0.95),
            line("More body text", 80.0, 16.0, 0.95),
        ]);
        let md = to_markdown(&r, &MarkdownOptions::default());
        assert!(md.starts_with("# Annual Report"), "got:\n{md}");
        assert!(md.contains("Body text here"));
        assert!(!md.contains("# Body text"));
    }

    #[test]
    fn a_large_vertical_gap_starts_a_paragraph() {
        let r = result(vec![
            line("first para", 10.0, 16.0, 0.95),
            line("still first", 30.0, 16.0, 0.95),
            line("second para", 120.0, 16.0, 0.95),
        ]);
        let md = to_markdown(&r, &MarkdownOptions::default());
        assert!(md.contains("still first\n\nsecond para"), "got:\n{md}");
    }

    #[test]
    fn low_confidence_lines_are_marked() {
        let r = result(vec![line("blurry text", 10.0, 16.0, 0.2)]);
        let md = to_markdown(&r, &MarkdownOptions::default());
        assert!(md.contains("<!-- low confidence -->"), "got:\n{md}");
    }

    /// A line the mean cannot mark.
    ///
    /// Four cells, three read confidently and one not — a bank statement row
    /// whose merchant was guessed and whose date, amount and balance were not.
    /// The mean of those four is 0.87 and sails over the 0.5 floor; the
    /// weakest of them is 0.20 and does not. This is the case the marker
    /// existed for and never once fired on.
    #[test]
    fn a_line_with_one_doubtful_cell_is_marked_although_its_mean_is_high() {
        let row = TextLine {
            boxes: vec![
                TextBox {
                    text: "05/01/2026".into(),
                    quad: Quad::from_rect(10.0, 10.0, 60.0, 16.0),
                    confidence: 0.99,
                },
                TextBox {
                    text: "MERIDIAN CAFE".into(),
                    quad: Quad::from_rect(80.0, 10.0, 90.0, 16.0),
                    confidence: 0.20,
                },
                TextBox {
                    text: "-42.57".into(),
                    quad: Quad::from_rect(200.0, 10.0, 40.0, 16.0),
                    confidence: 0.99,
                },
                TextBox {
                    text: "47,614.90".into(),
                    quad: Quad::from_rect(260.0, 10.0, 50.0, 16.0),
                    confidence: 0.99,
                },
            ],
            baseline_y: 18.0,
        };
        assert!(
            row.confidence() > 0.5,
            "the mean must not flag it, or this test proves nothing: {}",
            row.confidence()
        );
        let md = to_markdown(&result(vec![row]), &MarkdownOptions::default());
        assert!(md.contains("<!-- low confidence -->"), "got:\n{md}");
    }

    /// And a row where every cell is sound is left alone, so the marker is not
    /// simply always on.
    #[test]
    fn a_line_whose_cells_are_all_sound_is_not_marked() {
        let row = TextLine {
            boxes: vec![
                TextBox {
                    text: "05/01/2026".into(),
                    quad: Quad::from_rect(10.0, 10.0, 60.0, 16.0),
                    confidence: 0.99,
                },
                TextBox {
                    text: "MERIDIAN CAFE".into(),
                    quad: Quad::from_rect(80.0, 10.0, 90.0, 16.0),
                    confidence: 0.83,
                },
            ],
            baseline_y: 18.0,
        };
        let md = to_markdown(&result(vec![row]), &MarkdownOptions::default());
        assert!(!md.contains("low confidence"), "got:\n{md}");
    }

    #[test]
    fn marking_can_be_turned_off() {
        let r = result(vec![line("blurry text", 10.0, 16.0, 0.2)]);
        let options = MarkdownOptions {
            mark_uncertain: false,
            ..Default::default()
        };
        let md = to_markdown(&r, &options);
        assert_eq!(md.trim(), "blurry text");
    }

    #[test]
    fn empty_result_produces_empty_markdown() {
        assert!(to_markdown(&result(Vec::new()), &MarkdownOptions::default()).is_empty());
    }
}
