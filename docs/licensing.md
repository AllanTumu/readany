# Licensing: the product is commercial and closed

This is a constraint, not a preference. The product will be sold. Its source
will not be published. Every dependency must therefore be **permissive** —
attribution at most, never a requirement to release source.

Verified 9 August 2026 across `readany` and `statement`.

## The rule

| Allowed | Forbidden |
|---|---|
| MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib, Unlicense, Unicode-3.0 | **GPL, LGPL, AGPL, SSPL, CC-BY-NC**, or any "source available" licence |

LGPL is on the forbidden list deliberately. It permits dynamic linking, but it
also obliges you to let a customer relink against their own modified version,
which is awkward to honour in a shipped binary and pointless to argue about
when permissive alternatives exist.

## Measured state of the tree

`cargo tree --format "{l}"` over both crates, deduplicated:

| Licence | Count |
|---|---|
| MIT OR Apache-2.0 | 140 |
| MIT | 22 |
| Unlicense OR MIT | 15 |
| Apache-2.0 OR MIT | 8 |
| MIT OR Apache-2.0 OR Zlib | 4 |
| (Apache-2.0 OR MIT) AND BSD-3-Clause | 4 |
| (MIT OR Apache-2.0) AND Unicode-3.0 | 3 |
| Zlib OR Apache-2.0 OR MIT | 2 |
| BSD-3-Clause OR Apache-2.0 | 2 |

**Copyleft dependencies: zero.**

## Decisions this rule has already made

**PDFium binding.** `pdfium-rs` is GPL-3.0 and was rejected on that fact alone,
before its other merits were weighed — it would have forced the entire product
open. `pdfium-render` is MIT OR Apache-2.0 and was chosen. See
`pdfium-crate-choice.md`.

**PDF rendering engine.** MuPDF is AGPL. Worse than GPL for this product,
because AGPL's network clause triggers on a hosted worker: running MuPDF on
Hetzner and letting customers upload to it would count as distribution and
oblige source release. Commercial licences exist and cost money. PDFium is
BSD-3-Clause and free.

**No static linking of PDFium.** Not a licence requirement — BSD permits it —
but keeping the binary out of the build keeps the distribution question simple
and matches how `ort` loads ONNX Runtime.

## Checking this stays true

Run before adding any dependency:

```bash
cargo tree --prefix none --format "{p} {l}" \
  | grep -iE "GPL|AGPL|SSPL|CC-BY-NC" && echo "COPYLEFT FOUND" || echo "clean"
```

A hit is a blocker, not a discussion. Find a permissive alternative or write the
part yourself.

## What attribution actually requires

MIT, BSD and Apache-2.0 all require the copyright notice and licence text to
travel with binaries. That is what `NOTICES` at the repository root is for. It
must ship with any distributed build — bundled in the app, or reachable from an
"Open source licences" screen. Shipping without it is the one way a permissive
licence can still be breached.
