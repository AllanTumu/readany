//! Timing that works everywhere, including WebAssembly.
//!
//! `std::time::Instant::now()` traps on `wasm32-unknown-unknown` — there is no
//! clock in that environment — and a trap surfaces to JavaScript as an
//! unhelpful "unreachable". Timings are a nicety, so on WebAssembly they
//! report zero rather than bringing the whole call down.

/// A stopwatch. Reports 0 where no clock exists.
pub struct Stopwatch {
    #[cfg(not(target_arch = "wasm32"))]
    started: std::time::Instant,
}

impl Stopwatch {
    pub fn start() -> Self {
        Stopwatch {
            #[cfg(not(target_arch = "wasm32"))]
            started: std::time::Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed().as_millis() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }
}

impl Default for Stopwatch {
    fn default() -> Self {
        Self::start()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stopwatch_reports_a_plausible_duration() {
        let watch = Stopwatch::start();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let ms = watch.elapsed_ms();
        assert!(ms >= 4, "reported {ms}ms for a 5ms sleep");
        assert!(ms < 5_000, "reported an implausible {ms}ms");
    }
}
