//! Greedy CTC decoding.
//!
//! Recognition models emit a [timesteps x classes] matrix of probabilities.
//! CTC collapses it by taking the best class at each step, removing repeats,
//! and dropping the blank class. Index 0 is blank, matching the PaddleOCR
//! character dictionaries.

pub const BLANK: usize = 0;

/// Decode one probability matrix.
///
/// `probs` is row-major with `classes` values per timestep.
/// `charset` maps class index 1..=n to a character; index 0 is blank.
/// Returns the decoded text and the mean probability of the kept steps.
pub fn decode_greedy(probs: &[f32], classes: usize, charset: &[String]) -> (String, f32) {
    if classes == 0 || probs.is_empty() {
        return (String::new(), 0.0);
    }

    let mut text = String::new();
    let mut kept_scores: Vec<f32> = Vec::new();
    let mut previous = usize::MAX;

    for step in probs.chunks_exact(classes) {
        let (best_index, &best_score) = step
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or((BLANK, &0.0));

        if best_index != BLANK && best_index != previous {
            if let Some(ch) = charset.get(best_index - 1) {
                text.push_str(ch);
                kept_scores.push(best_score);
            }
        }
        previous = best_index;
    }

    let confidence = if kept_scores.is_empty() {
        0.0
    } else {
        kept_scores.iter().sum::<f32>() / kept_scores.len() as f32
    };

    (text, confidence)
}

/// Load a character dictionary: one character per line, blank not included.
pub fn parse_charset(contents: &str) -> Vec<String> {
    contents
        .lines()
        .map(|l| l.trim_end_matches(['\r', '\n']).to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charset() -> Vec<String> {
        "abc".chars().map(|c| c.to_string()).collect()
    }

    /// Build a probability matrix from a list of (class index, score).
    fn probs(steps: &[(usize, f32)], classes: usize) -> Vec<f32> {
        let mut out = vec![0.0; steps.len() * classes];
        for (t, &(idx, score)) in steps.iter().enumerate() {
            out[t * classes + idx] = score;
        }
        out
    }

    #[test]
    fn collapses_repeats_and_drops_blanks() {
        // a a blank a b  ->  "aab"
        let p = probs(&[(1, 0.9), (1, 0.8), (BLANK, 0.9), (1, 0.7), (2, 0.95)], 4);
        let (text, conf) = decode_greedy(&p, 4, &charset());
        assert_eq!(text, "aab");
        assert!((conf - 0.85).abs() < 0.01, "confidence was {conf}");
    }

    #[test]
    fn empty_input_is_safe() {
        assert_eq!(decode_greedy(&[], 4, &charset()), (String::new(), 0.0));
    }

    #[test]
    fn all_blank_yields_nothing() {
        let p = probs(&[(BLANK, 0.9), (BLANK, 0.9)], 4);
        let (text, conf) = decode_greedy(&p, 4, &charset());
        assert!(text.is_empty());
        assert_eq!(conf, 0.0);
    }

    #[test]
    fn charset_parsing_skips_blank_lines() {
        let cs = parse_charset("a\nb\n\nc\n");
        assert_eq!(cs, vec!["a", "b", "c"]);
    }
}
