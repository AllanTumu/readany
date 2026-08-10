# The size of the WebAssembly blob

Everything here was measured on one machine on 10 August 2026: aarch64 macOS,
`rustc 1.97.1`, `wasm-pack 0.15.0`, `wasm-pack build wasm --target nodejs
--release`. Every row is the size of `readany_wasm_bg.wasm` in bytes. Each
change was applied on top of the row above it and built on its own, so the
column means what it says.

## What was there to start with

The published 0.1.0 blob is **6,130,279 bytes**, packing to 2,496,903 in a
tarball of 7 files. Rebuilding the same source on today's toolchain gives
6,182,823 — a drift of 52,544 bytes that belongs to the compiler, not to us.
That rebuild is the baseline, so that toolchain drift is not counted as a win
or a loss.

## Where the bytes come from

`cargo tree --target wasm32-unknown-unknown` first, before any compiler switch,
because a dependency that should not be linked is worth more than any flag.

**`aes` and `chacha20` are in the blob, and they stay.** The suspicion was that
they arrive through `anydoc` and are pure weight in a document reader. They
arrive through `pdf-inspector` to `lopdf`, and they are load-bearing: `lopdf`
uses AES and RC4 to open encrypted PDFs, which is a path this crate documents,
depends on and tests. A great many bank statements carry an owner password
only, and `tests/encrypted_pdf.rs` pins that they read byte-identically to
their unencrypted twin. Removing the cipher would remove that.

`chacha20` is the weaker case — it comes in behind `rand`, which `lopdf` needs
for *writing* encrypted PDFs, and nothing here writes one. It cannot be
feature-gated away without a change in `lopdf`, so it is a finding rather than
a fix.

**Two complete zip implementations were linked**, and that one was ours. This
crate asked for `zip 5` and `anydoc`'s `calamine` asks for `zip 8`, so cargo
resolved both. The bomb guard in `src/archive.rs` uses `ZipArchive`, `by_index`
and `by_index_raw`, all unchanged across the gap, so moving to `zip 8` compiled
without a single edit.

Also present and not ours to remove: `regex` and an `include_dir` payload from
`pdf-inspector`, the `encoding_rs` tables, `zopfli` (a compressor, reached
through `calamine`'s zip), and `chrono` for Excel date cells.

## Each change, measured on its own

| | bytes | change |
|---|---|---|
| 0.1.0 as published, 6 August | 6,130,279 | |
| same source, today's toolchain — **baseline** | 6,182,823 | +52,544 drift |
| the 0.2.0 binding: provenance, typed refusals, `prepare` | 6,994,335 | **+811,512** |
| `zip` 5 to 8, deduplicating the second implementation | 6,965,155 | −29,180 |
| `--remap-path-prefix` for the build-machine paths | 6,961,875 | −3,280 |
| `wasm-opt -Oz` | **5,581,307** | **−1,380,568** |

Net against the published blob: **−548,972 bytes, −9.0%**.

`wasm-opt` is the whole story. `opt-level = "z"` and fat LTO were already on,
and the second pass still found 19.8% — it optimises the WebAssembly rustc
emitted rather than the Rust, so it reaches unreachable functions and duplicate
bodies across the whole module that rustc never sees as one unit. It costs
about seven seconds.

The `zip` deduplication is worth keeping and is not worth much: 29 KB, because
most of the second copy was already dead-code-eliminated. It stays for hygiene
— one zip implementation in a binary is a correctness property as well as a
size one.

## Three switches that did not work

**`trim-paths = "all"` is not available.** It is the right tool and cargo
1.97.1 rejects it as unstable, in spite of the crate requiring 1.88. Requiring
nightly to cut a release is a worse trade than the paths, so
`scripts/build-wasm.sh` uses `--remap-path-prefix`, stable for years, and then
greps the blob to prove it worked. Revisit when it stabilises.

**`panic = "abort"` made the blob larger.** 5,582,862 against 5,581,307:
**+1,555 bytes**. `wasm32-unknown-unknown` does not support unwinding, so the
strategy is effectively abort already and there is nothing to remove; setting
it only changes how the panic path is emitted. A switch famous for shrinking
binaries, that grows this one.

**`--strip-debug --strip-producers --strip-target-features` on top of `-Oz`**
gave 5,587,308 against 5,581,307, also larger. `-Oz` has already done that
work. There is no name-section fat left to reclaim; the 5.58 MB is code.

## What the new binding cost, and why it was paid

Building the *old* binding through the *same* full pipeline gives 4,974,161.
So the 0.2.0 surface costs **607,146 bytes after optimisation**, not the
811,512 it appears to cost before it.

Almost all of that is `prepare`. Before 0.2.0 nothing in the binding reached
the image path, so `image`'s decoders and the whole geometric pipeline —
crop, flatten, orient, deskew — were dead-code-eliminated out of the blob. The
package claimed in its own module documentation to contain "the image
correction stages" while shipping none of them.

It is paid because the capability is real and is the privacy-relevant half: a
browser can decode, straighten and grayscale a photograph locally and send
*that* to a worker, instead of the original photograph with its EXIF and its
GPS tag. Reversing the decision means deleting `prepare` and getting 607 KB
back, and the number is recorded here so that stays a decision rather than a
discovery.

## The packed number, which is the one a browser pays

The tarball is what actually travels:

| | packed | unpacked | files |
|---|---|---|---|
| 0.1.0 as published | 2,496,903 | 6,141,305 | 7 |
| 0.2.0 | see `scripts/build-wasm.sh` output | | 9 |

**Unpacked falls and packed does not fall with it.** `wasm-opt` output
compresses slightly worse than what it replaces, and 0.2.0 adds real code plus
a second JavaScript glue. A browser downloading over a compressing transport
therefore sees roughly what it saw before, while the memory the module occupies
once instantiated falls by about half a megabyte. Both numbers are worth
knowing and only one of them is the download.

The browser glue is 9 files rather than 7 and costs about 21 KB, because the
`.wasm` is byte-identical between wasm-pack's `nodejs` and `web` targets. That
is checked at build time, not assumed: shipping one glue against the other's
blob would fail at runtime, and shipping two blobs would double the package.

## What has not been tried

Splitting the readers so a consumer who only wants office documents does not
download the PDF stack. That is the only remaining lever of this size, and it
is a public API change rather than a build flag.
