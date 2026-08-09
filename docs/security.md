# Threat model, and what we can promise on each surface

Written 9 August 2026, at the point where the engine stops reading documents we
chose and starts reading documents strangers send.

Privacy: counts and mechanisms only. No document content appears here.

## The model

**Server.** A stranger uploads a file. They choose every byte, and they may be
trying to take the service down rather than to have their statement read. A
wrong answer is bad; the service going down for everyone is worse. Parsing
happens partly in C++ (PDFium, image codecs) where a memory error is not
caught by the language.

**Browser.** The user opens their own file, in their own tab, through the
WebAssembly build. The realistic case is a corrupt or awkward document, not an
attack. That makes it **lower risk, not no risk** — a bomb still freezes their
tab, and they report it as our bug.

## What each surface can promise

| Mitigation | Server | Browser |
|---|---|---|
| Pre-flight decompression cap | yes | yes |
| Byte, page and pixel ceilings | yes | yes |
| Checked arithmetic on untrusted numbers | yes | yes |
| Wall-clock kill | yes | **no** |
| Memory ceiling | yes (`setrlimit`) | **no** — only the 4 GB WASM address space |
| Process isolation | yes | **no** |
| Panic containment | yes (worker dies, service lives) | **no** — the tab's call stack |

**Every `no` is a promise we cannot make, and the frontend has to know before
it is designed.** In the browser there is no process to kill, no `fork`, no
`setrlimit`, and no way to stop a computation that has begun. The countable
limits still apply because they are checked before the work starts; everything
that depends on stopping work in progress does not exist there.

The practical consequence: on the browser, refuse early and generously. A file
that would be *killed* on the server must be *refused* in the tab, because
killing is not available.

## Limits

| Limit | Value | Enforced by |
|---|---|---|
| Input size | 50 MB | `read_with`, before sniffing |
| Pages | 200 | `read_with`, from the route plan |
| Rendered/decoded pixels per page | 40 M | `decode_bytes`, from the header |
| Decompressed bytes, zip formats | 200 MB | `archive::check`, counted as produced |
| Zip entry count | 10,000 | `archive::check`, from the directory |
| Zip nesting depth | 2 | `archive::check`, recursively |
| Wall clock per job | 60 s | **killing the worker** |
| Resident memory per job | 1 GB | **`setrlimit` in the child** |

The last two cannot be enforced inside the process doing the work. Rust has no
safe way to stop a thread, so a parser in a loop can only be stopped by killing
a process. That is why the worker boundary exists and why every other limit
sits inside it.

## Findings

Both were found by looking where we expected to find something. **Recording
what we expected matters as much as recording what we found**, because it is
the only way a later reader can tell which parts of the surface have actually
been examined from which merely look calm.

### A zip bomb was accepted

*Expected:* the zip readers accept a bomb, and it is the only thing exploitable
today. **Confirmed.**

A 4.6 MB `.xlsx` — structurally valid, sniffing correctly as a spreadsheet —
expanded to **1.04 GB** and was read to completion in **3.0 s using 1.075 GB**,
exit code zero. Resident memory tracked decompressed size at about 1.03×, so at
the same ratio a 50 MB upload reaches ~11 GB, and deflate permits ratios four
times higher again.

Every Office format is a zip, so this was one surface for `xlsx`, `docx`,
`pptx`, `odt` and `epub` together.

Now refused in 0.52 s at 11 MB. With every declared size in both the central
directory and the local headers rewritten to 4 KB, so only measurement can
catch it: refused in **0.05 s at 11.5 MB**.

### A picture could exceed the pixel ceiling by 3.6×

*Expected:* at least one unchecked multiplication in the pixel path.
**Confirmed, twice, and the second was worse than expected.**

`flatten` sized a summed-area table `(w + 1) * (h + 1)` in `u32` from a decoded
image's own dimensions. At 65535 square that product is exactly 2³² and wraps
to **zero**: the table would be allocated empty and the very next line would
index it. A panic reachable by anyone who could upload a picture.

Then, looking for the choke point that would bound all such sites at once: a
**137 KB PNG** declaring 12000 square decoded to **144 megapixels**, 3.6× the
documented ceiling, in 0.04 s using 295 MB. The ceiling had been written down,
unit-tested, and **never called on this path** — a check that never runs, which
is the same class of defect the verification engine has a registry test for.

Now refused from the header at 0.00 s, before any allocation.

### What the choke point does and does not cover

`decode_bytes` bounds the dimensions of every image that enters from outside,
which bounds every buffer sized from those dimensions downstream.

It does **not** cover images created inside the pipeline, because they never
pass through it:

- **rendered PDF pages** — built by the rasteriser, not decoded;
- **crops** from `frame::content_bounds`;
- **rotations** from `deskew::rotate` and `orient::apply`;
- **resizes** before recognition, including the tensor in `readany-ocr`;
- **synthesised images** in tests and fixtures.

These are bounded transitively when derived from a decoded upload — a crop is
smaller than its source — but not when derived from a rendered page, and not if
a resize ever enlarges. So the arithmetic is converted as well as bounded:
`flatten` and `to_tensor` both compute in `u64` and refuse rather than wrap.

## Fuzzing

**The claim that PDFium is fuzzed upstream by Google: substantially true, but
not through OSS-Fuzz as stated.** PDFium carries fuzz targets in its own tree
and they run continuously on **Chromium's ClusterFuzz**, which is a different
pipeline from OSS-Fuzz's `projects/` directory. I could not confirm a
`projects/pdfium` entry in OSS-Fuzz from the sources I could reach, and I am
recording that as unconfirmed rather than rounding it up.

The conclusion the claim was used for still holds: PDFium's own parsing is
fuzzed continuously by people with far more machines than we have, so our
fuzzing belongs on **our** surfaces — sniffing, the archive guard, layout and
parse, the ledger writers, and the image pipeline.

`examples/fuzz.rs` runs on stable so it can live in CI rather than on one
laptop. `cargo-fuzz` is installed for deeper runs.

## Still open

- Part 7, the sandbox: no network, dropped privileges, read-only root.
- Fuzzing `layout`/`parse` with hostile coordinates, and the OFX/QBO writers
  with hostile strings. The OFX writer matters most: a string that escapes its
  tag is an injection into the user's accounting software.
- The panic audit of library paths on untrusted input.
- `anydoc` owns the decompression that matters; the cap is a pre-flight here.
  Offer it upstream, and only then decide about a fork.
