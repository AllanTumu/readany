//! readany CLI.
//!
//! One command for every document. It reports which engine read the file and
//! accounts for every page, so a partial read can never look like a whole one.

use readany::{inspect, read_with, Options, Origin};
use std::process::ExitCode;

const USAGE: &str = "\
readany - read any document into Markdown

USAGE:
    readany <file> [OPTIONS]

OPTIONS:
    -o, --out <file>   Write Markdown here instead of stdout
    --inspect          Report what would happen, and read nothing
    --json             Machine-readable report
    --pages            Insert <!-- Page N --> markers
    --strict           Fail rather than return a partial document
    -h, --help         Show this message

EXIT CODES:
    0  the whole file was read
    1  the file could not be read
    2  usage error
    3  the file was read in part; some pages need OCR
";

fn main() -> ExitCode {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{USAGE}");
        return if args.is_empty() {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        };
    }

    let mut path: Option<String> = None;
    let mut out: Option<String> = None;
    let mut inspect_only = false;
    let mut json = false;
    let mut options = Options::default();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--out" => {
                i += 1;
                match args.get(i) {
                    Some(v) => out = Some(v.clone()),
                    None => {
                        eprintln!("readany: --out needs a file name");
                        return ExitCode::from(2);
                    }
                }
            }
            "--inspect" => inspect_only = true,
            "--json" => json = true,
            "--pages" => options.page_markers = true,
            "--strict" => options.strict = true,
            other if other.starts_with('-') => {
                eprintln!("readany: unknown option {other}");
                return ExitCode::from(2);
            }
            other => path = Some(other.to_string()),
        }
        i += 1;
    }

    let Some(path) = path else {
        eprintln!("readany: no input file");
        return ExitCode::from(2);
    };

    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("readany: {path}: {e}");
            return ExitCode::from(1);
        }
    };

    if inspect_only {
        return match inspect(&bytes) {
            Ok(plan) => {
                if json {
                    println!(
                        "{{\"file\":\"{}\",\"route\":\"{}\",\"needs_ocr\":{},\"ocr_pages\":{},\"inspect_ms\":{}}}",
                        escape(&path),
                        plan.summary(),
                        plan.needs_ocr,
                        plan.ocr_page_count,
                        plan.inspect_time_ms
                    );
                } else {
                    println!("{}", plan.summary());
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("readany: {e}");
                ExitCode::from(1)
            }
        };
    }

    let doc = match read_with(&bytes, &options) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("readany: {e}");
            return ExitCode::from(1);
        }
    };

    if let Some(target) = &out {
        if let Err(e) = std::fs::write(target, &doc.markdown) {
            eprintln!("readany: could not write {target}: {e}");
            return ExitCode::from(1);
        }
    }

    if json {
        let text = doc
            .pages
            .iter()
            .filter(|p| p.origin == Origin::Text)
            .count();
        println!(
            "{{\"file\":\"{}\",\"pages\":{},\"extracted\":{},\"recognised\":{},\"unresolved\":{:?},\"complete\":{},\"ms\":{}}}",
            escape(&path),
            doc.pages.len(),
            text,
            doc.ocr_pages.len(),
            doc.unresolved_pages,
            doc.is_complete(),
            doc.processing_time_ms
        );
    } else if out.is_some() {
        println!("{}", doc.receipt());
    } else {
        print!("{}", doc.markdown);
        if !doc.markdown.is_empty() && !doc.markdown.ends_with('\n') {
            println!();
        }
    }

    if !doc.is_complete() {
        eprintln!(
            "readany: {} page(s) need OCR and were not read: {:?}",
            doc.unresolved_pages.len(),
            doc.unresolved_pages
        );
        return ExitCode::from(3);
    }

    ExitCode::SUCCESS
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
