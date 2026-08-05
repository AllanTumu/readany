use super::GrayImage;

/// Otsu's method: pick the threshold that maximises between-class variance.
/// Returns the chosen grey level.
pub fn otsu_level(img: &GrayImage) -> u8 {
    let mut histogram = [0u64; 256];
    for p in img.pixels() {
        histogram[p.0[0] as usize] += 1;
    }

    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return 128;
    }

    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();

    let mut sum_background = 0.0f64;
    let mut weight_background = 0u64;
    let mut best_variance = -1.0f64;
    let mut best_level = 0u8;

    for (level, &count) in histogram.iter().enumerate() {
        weight_background += count;
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0 {
            break;
        }

        sum_background += level as f64 * count as f64;
        let mean_background = sum_background / weight_background as f64;
        let mean_foreground = (sum_all - sum_background) / weight_foreground as f64;
        let diff = mean_background - mean_foreground;
        let variance = weight_background as f64 * weight_foreground as f64 * diff * diff;

        if variance > best_variance {
            best_variance = variance;
            best_level = level as u8;
        }
    }

    best_level
}

/// Produce a mask where 1 marks ink and 0 marks paper.
/// Documents are assumed dark-on-light; the polarity is corrected if the
/// resulting ink coverage is implausibly high.
pub fn ink_mask(img: &GrayImage) -> Vec<bool> {
    let level = otsu_level(img);
    let mut mask: Vec<bool> = img.pixels().map(|p| p.0[0] <= level).collect();

    let ink = mask.iter().filter(|&&b| b).count();
    if ink * 2 > mask.len() {
        // More than half the page is "ink" — the page is light-on-dark.
        for b in mask.iter_mut() {
            *b = !*b;
        }
    }
    mask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_splits_a_two_tone_image() {
        let mut img = GrayImage::new(10, 10);
        for (i, p) in img.pixels_mut().enumerate() {
            p.0[0] = if i < 50 { 20 } else { 220 };
        }
        let level = otsu_level(&img);
        assert!((20..220).contains(&level), "level was {level}");
    }

    #[test]
    fn ink_mask_flips_inverted_pages() {
        // Mostly dark page with light text: ink must stay the minority.
        let mut img = GrayImage::new(10, 10);
        for (i, p) in img.pixels_mut().enumerate() {
            p.0[0] = if i < 10 { 240 } else { 15 };
        }
        let mask = ink_mask(&img);
        let ink = mask.iter().filter(|&&b| b).count();
        assert!(ink * 2 <= mask.len(), "ink coverage was {ink}/100");
    }
}
