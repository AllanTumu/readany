//! Probe: what does the image path do with a hostile header?
//! Prints outcome only, never contents.
fn main() {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).expect("read");
        let started = std::time::Instant::now();
        let outcome = std::panic::catch_unwind(|| readany::ocr::image::decode_bytes(&bytes));
        let took = started.elapsed().as_secs_f32();
        match outcome {
            Ok(Ok(img)) => println!("{path}: DECODED {:?} in {took:.2}s", img.dimensions()),
            Ok(Err(e)) => println!("{path}: refused ({}) in {took:.2}s", short(&e.to_string())),
            Err(_) => println!("{path}: PANICKED in {took:.2}s"),
        }
    }
}
fn short(s: &str) -> String { s.chars().take(60).collect() }
