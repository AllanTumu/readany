# Choosing a PDFium binding

Measured 9 August 2026. Two candidates were checked against five facts before
any code was written.

## The five facts

| | `pdfium-render` | `pdfium` (pdfium-rs) |
|---|---|---|
| Latest release | **0.9.3**, 14 Jul 2026 | 0.10.4, 17 May 2026 |
| Last commit | **26 Jul 2026** | 17 May 2026 |
| Open issues | 15, on 691 stars | 0, on 22 stars |
| Maintainer replies | **yes** — #270 answered within days, #253 closed with a documentation fix | thin: 2 of the last 5 issues closed with no comment |
| Runtime dynamic loading | **yes** — `libloading` on every non-wasm target, the `ort` pattern | yes — `libloading` |
| `aarch64-linux-android` | **yes**, documented, and `bblanchon` ships `pdfium-android-arm64.tgz` | not mentioned |
| Licence | **MIT OR Apache-2.0** | **GPL-3.0** |
| Downloads | 1,872,533 | 25,410 |

## The choice: `pdfium-render`

**The licence settles it before anything else does.** `pdfium-rs` is GPL-3.0.
Linking it would force this product open, which is the same reason the brief
bans MuPDF. It is disqualified on that fact alone, and the remaining facts only
confirm it: a twentieth of the downloads, a quarter of the release recency, and
no stated Android support.

`pdfium-render` is healthy on every axis. The open-issue count is *higher* than
its rival's zero, and that is a point in its favour rather than against —
issues get filed against libraries people use, and the maintainer answers them.
A crate with 22 stars and no open issues has no users, not no bugs.

Runtime loading is confirmed in its manifest rather than assumed:

```toml
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
libloading = "0.8"
```

`static` is an opt-in feature we do not enable. No PDFium binary enters the
build; the shared library is found at run time, exactly as `ort` finds ONNX
Runtime.

## Licence chain

Three layers, all permissive:

| Layer | Licence |
|---|---|
| `pdfium-render` (the binding) | MIT OR Apache-2.0 |
| PDFium (the C++ library, from Chromium) | BSD-3-Clause, with Apache-2.0 components |
| `bblanchon/pdfium-binaries` (the build scripts) | MIT |

Attribution required by all three is in `NOTICES` at the repository root, added
in the same commit as this document.

## What was *not* checked, and why

A build against `aarch64-linux-android` was not run. It needs the Android NDK,
which is not on this machine, and the claim in the report is therefore
"documented and binaries exist", not "measured". Anyone who needs certainty
should run it before shipping to a phone — and the seam in
`readany/src/pdf/render.rs` exists precisely so that Android can use the
platform's own `PdfRenderer` instead, in which case the question never arises.
