//! What a hostile file is not allowed to do.
//!
//! Every value here exists because the input is written by strangers. None of
//! them improves a reading; all of them are the price of running the engine
//! where other people can reach it.
//!
//! ## Two kinds of limit, and only one can live in this process
//!
//! **Countable limits** — bytes, pages, pixels, archive entries — are checked
//! before the work is done, and this module enforces them.
//!
//! **Wall clock and resident memory cannot be enforced here.** Rust has no
//! safe way to stop a thread, so a parser stuck in a loop or a page that
//! renders for nine minutes cannot be interrupted from inside the process it
//! is running in. Those two are enforced by killing a worker process, and the
//! values are carried here only so that one table states the whole contract.
//!
//! ## Arithmetic on untrusted numbers
//!
//! A page declares its own dimensions. `width * height * channels` on a page
//! claiming 65,536 points square overflows `u32` and wraps to **zero**, which
//! then passes every limit test underneath it with room to spare. Every count derived from
//! input uses checked arithmetic and treats overflow as "over the limit"
//! rather than as a number — see [`Limits::pixels_within`].

/// The ceilings a single job may not cross.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of input accepted at all.
    pub input_bytes: u64,
    /// Pages in a document.
    pub pages: u32,
    /// Pixels in one rendered page, after any dpi reduction.
    pub pixels_per_page: u64,
    /// Wall clock for one job. **Enforced by killing the worker**, never here.
    pub wall_clock_seconds: u64,
    /// Resident memory for one job. **Enforced by the operating system**,
    /// never here.
    pub resident_bytes: u64,
    /// Bytes an archive may decompress to, counted as they are produced.
    pub decompressed_bytes: u64,
    /// Entries an archive may contain.
    pub archive_entries: u64,
    /// How deeply archives may nest inside one another.
    pub archive_depth: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            input_bytes: 50 * MB,
            pages: 200,
            pixels_per_page: 40_000_000,
            wall_clock_seconds: 60,
            resident_bytes: 1024 * MB,
            decompressed_bytes: 200 * MB,
            archive_entries: 10_000,
            archive_depth: 2,
        }
    }
}

const MB: u64 = 1024 * 1024;

impl Limits {
    /// Limits that permit anything, for a caller reading its own files.
    ///
    /// Never the default. A caller that wants no ceilings has to say so, so
    /// that "we forgot to configure limits" and "we chose to have none" cannot
    /// look the same in a code review.
    pub fn none() -> Limits {
        Limits {
            input_bytes: u64::MAX,
            pages: u32::MAX,
            pixels_per_page: u64::MAX,
            wall_clock_seconds: u64::MAX,
            resident_bytes: u64::MAX,
            decompressed_bytes: u64::MAX,
            archive_entries: u64::MAX,
            archive_depth: u32::MAX,
        }
    }

    /// Is a page of these dimensions within the pixel limit?
    ///
    /// **Checked arithmetic, and overflow means refused.** A page may declare
    /// any dimensions it likes; `65536 * 65536` is exactly `2^32` and wraps to
    /// zero in `u32`, so such a page would report *no pixels at all* and pass
    /// every limit beneath it. Computed in `u64` from `u64` inputs and refused
    /// on overflow, which is the only answer that is safe in both directions.
    pub fn pixels_within(&self, width: u64, height: u64, channels: u64) -> bool {
        match width.checked_mul(height).and_then(|a| a.checked_mul(channels)) {
            Some(pixels) => pixels <= self.pixels_per_page,
            None => false,
        }
    }
}

/// A limit that a file crossed.
///
/// Carries the limit and the value that broke it, because "too large" is not
/// an actionable message and a caller may want to raise a ceiling knowingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exceeded {
    pub limit: &'static str,
    pub allowed: u64,
    /// What the file asked for. `None` when the true value is unknown because
    /// computing it would itself have been unbounded — an overflowed product,
    /// or a stream still arriving when the cap was reached.
    pub found: Option<u64>,
}

impl std::fmt::Display for Exceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.found {
            Some(found) => write!(f, "{} is {found}, over the limit of {}", self.limit, self.allowed),
            None => write!(f, "{} is over the limit of {}", self.limit, self.allowed),
        }
    }
}

impl Exceeded {
    pub fn new(limit: &'static str, allowed: u64, found: u64) -> Exceeded {
        Exceeded { limit, allowed, found: Some(found) }
    }

    pub fn unbounded(limit: &'static str, allowed: u64) -> Exceeded {
        Exceeded { limit, allowed, found: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_contract_is_the_documented_one() {
        let l = Limits::default();
        assert_eq!(l.input_bytes, 50 * MB);
        assert_eq!(l.pages, 200);
        assert_eq!(l.pixels_per_page, 40_000_000);
        assert_eq!(l.wall_clock_seconds, 60);
        assert_eq!(l.resident_bytes, 1024 * MB);
        assert_eq!(l.decompressed_bytes, 200 * MB);
        assert_eq!(l.archive_entries, 10_000);
        assert_eq!(l.archive_depth, 2);
    }

    #[test]
    fn an_ordinary_page_is_within_the_pixel_limit() {
        let l = Limits::default();
        // A4 at 300 dpi, four channels.
        assert!(l.pixels_within(2480, 3508, 4) == (2480 * 3508 * 4 <= 40_000_000));
        // A4 at 150 dpi, four channels: comfortably inside.
        assert!(l.pixels_within(1240, 1754, 4));
    }

    /// The overflow that would otherwise pass every limit beneath it.
    #[test]
    fn a_page_declaring_absurd_dimensions_is_refused_not_wrapped() {
        let l = Limits::default();

        // 65536 × 65536 is exactly 2^32, so in u32 arithmetic it wraps to
        // **zero** — and zero times any number of channels is still zero. A
        // page declaring 65536 square would have reported no pixels at all and
        // passed every limit beneath it with room to spare.
        let wrapped = 65_536u32.wrapping_mul(65_536).wrapping_mul(4);
        assert_eq!(wrapped, 0, "the wrap this test exists to prevent");
        assert!(wrapped as u64 <= l.pixels_per_page, "and it would have passed");

        // Computed honestly, it is refused.
        assert!(!l.pixels_within(65_536, 65_536, 4));
        assert!(!l.pixels_within(60_000, 60_000, 4));

        // And a product that overflows even u64 is refused rather than wrapped.
        assert!(!l.pixels_within(u64::MAX, 2, 1));
        assert!(!l.pixels_within(u64::MAX, u64::MAX, u64::MAX));
    }

    #[test]
    fn zero_sized_pages_do_not_panic_or_pass_by_accident() {
        let l = Limits::default();
        assert!(l.pixels_within(0, 0, 0));
        assert!(l.pixels_within(1, 1, 1));
    }

    #[test]
    fn having_no_limits_must_be_asked_for() {
        assert_ne!(Limits::none(), Limits::default());
        assert!(Limits::none().pixels_within(u64::MAX / 4, 2, 1));
    }

    #[test]
    fn an_exceeded_limit_names_itself_and_the_value() {
        let e = Exceeded::new("input size", 100, 500);
        assert!(e.to_string().contains("500"));
        assert!(e.to_string().contains("100"));
        // And an unbounded one does not invent a number it never had.
        let u = Exceeded::unbounded("decompressed bytes", 200);
        assert!(!u.to_string().contains("None"));
    }
}
