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
    /// 0.0 to 1.0. Low values are the caller's cue to send this page elsewhere.
    pub confidence: f32,
}

/// Text boxes grouped into a reading line.
#[derive(Debug, Clone)]
pub struct TextLine {
    pub boxes: Vec<TextBox>,
    pub baseline_y: f32,
}

impl TextLine {
    pub fn text(&self) -> String {
        self.boxes
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn confidence(&self) -> f32 {
        if self.boxes.is_empty() {
            return 0.0;
        }
        self.boxes.iter().map(|b| b.confidence).sum::<f32>() / self.boxes.len() as f32
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

    /// True when the caller should not trust this result.
    pub fn needs_review(&self, threshold: f32) -> bool {
        self.lines.is_empty() || self.confidence() < threshold
    }
}
