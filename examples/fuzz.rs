//! A fuzzer that runs on stable, so it can live in CI rather than on a laptop.
//!
//! `cargo-fuzz` needs nightly and libFuzzer. This is deliberately weaker and
//! deliberately always available: a seeded generator, a mutator, and
//! `catch_unwind` around each target. It finds the shallow reachable panics —
//! which is the class that matters here, because every one of them is a denial
//! of service available to anyone who can send us a file.
//!
//! It is seeded and therefore reproducible. A crash prints the seed and the
//! input's shape, never its contents.
//!
//! ```text
//! cargo run --release --example fuzz -- 200000        # all targets
//! cargo run --release --example fuzz -- 200000 image  # one target
//! ```
//!
//! **Privacy: prints counts, seeds and byte shapes only.**

use std::panic::{catch_unwind, AssertUnwindSafe};

/// xorshift64*, so a run is reproducible from its seed without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next() % n as u64) as usize
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

/// Bytes that look enough like a real file to get past a sniffer.
///
/// Pure noise is refused at the front door and exercises nothing. Most of the
/// interesting surface is *behind* a successful format guess, so most inputs
/// start from a real magic number and are corrupted from there.
fn generate(rng: &mut Rng) -> Vec<u8> {
    const MAGICS: [&[u8]; 8] = [
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff\xe0",
        b"GIF89a",
        b"BM",
        b"II*\x00",
        b"MM\x00*",
        b"RIFF____WEBPVP8 ",
        b"%PDF-1.4\n",
    ];

    let mut out = Vec::new();
    if rng.below(10) > 0 {
        out.extend_from_slice(MAGICS[rng.below(MAGICS.len())]);
    }
    // A short body finds header arithmetic; a long one finds buffer sizing.
    let len = match rng.below(4) {
        0 => rng.below(32),
        1 => rng.below(512),
        2 => rng.below(8 * 1024),
        _ => rng.below(64 * 1024),
    };
    for _ in 0..len {
        // Biased towards 0x00 and 0xff, which is where size fields live.
        out.push(match rng.below(4) {
            0 => 0,
            1 => 0xff,
            _ => rng.byte(),
        });
    }
    out
}

struct Target {
    name: &'static str,
    run: fn(&[u8]),
}

/// Image bytes, decoded and then put through the whole preparation pipeline.
///
/// The target that would have caught the `flatten` wrap, and the surface every
/// photograph upload touches.
fn target_image(bytes: &[u8]) {
    let Ok(img) = readany::ocr::image::decode_bytes(bytes) else { return };
    let flat = readany::ocr::image::flatten::flatten(&img);
    let skew = readany::ocr::image::deskew::estimate_skew(&flat);
    let _ = readany::ocr::image::deskew::rotate(&flat, skew);
    let orientation = readany::ocr::image::orient::detect(&flat);
    let _ = readany::ocr::image::orient::apply(&flat, orientation);
    let _ = readany::ocr::image::binarize::otsu_level(&flat);
    let _ = readany::ocr::image::binarize::ink_mask(&flat);
    let _ = readany::ocr::image::frame::content_bounds(&flat);
}

/// Sniffing and classification on arbitrary bytes.
fn target_route(bytes: &[u8]) {
    let _ = readany::route::inspect_named(bytes, Some("x.csv"));
    let _ = readany::route::inspect_named(bytes, None);
}

/// The archive guard, which by design reads attacker-written headers.
fn target_archive(bytes: &[u8]) {
    let _ = readany::archive::check(bytes, &readany::limits::Limits::default());
}

/// The whole reader, at its real entry point, under the documented limits.
fn target_read(bytes: &[u8]) {
    let _ = readany::read_with(
        bytes,
        &readany::Options { limits: readany::limits::Limits::default(), ..Default::default() },
    );
}

const TARGETS: [Target; 4] = [
    Target { name: "image", run: target_image },
    Target { name: "route", run: target_route },
    Target { name: "archive", run: target_archive },
    Target { name: "read", run: target_read },
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let iterations: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(50_000);
    let only = args.get(1).cloned();

    // Panics are reported by us, not by the default hook, so a run of a
    // million inputs does not produce a million backtraces.
    std::panic::set_hook(Box::new(|_| {}));

    let mut failures = 0usize;
    for target in TARGETS.iter() {
        if only.as_deref().is_some_and(|o| o != target.name) {
            continue;
        }
        let mut crashes: Vec<(u64, usize)> = Vec::new();
        let started = std::time::Instant::now();
        for i in 0..iterations {
            let seed = 0x9E37_79B9_7F4A_7C15u64 ^ (i as u64).wrapping_mul(0x0010_0000_01B3);
            let mut rng = Rng(seed | 1);
            let input = generate(&mut rng);
            let run = target.run;
            if catch_unwind(AssertUnwindSafe(|| run(&input))).is_err() {
                crashes.push((seed, input.len()));
                if crashes.len() >= 5 {
                    break;
                }
            }
        }
        println!(
            "{:<8} {iterations} inputs in {:.1}s — {} crash(es)",
            target.name,
            started.elapsed().as_secs_f32(),
            crashes.len()
        );
        for (seed, len) in &crashes {
            println!("    seed {seed:#018x}, {len} bytes");
            failures += 1;
        }
    }

    let _ = std::panic::take_hook();
    if failures > 0 {
        std::process::exit(1);
    }
}
