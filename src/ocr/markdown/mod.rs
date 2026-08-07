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
        let rendered = if options.mark_uncertain && line.confidence() < options.confidence_floor {
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
