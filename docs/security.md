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
| Bytes in any one file a worker writes | 256 MB | **`RLIMIT_FSIZE` in the child** |

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

## The panicking forms are denied where untrusted input arrives

Scoped to the modules that read what a stranger sent, and nothing else:
`route`, `archive`, the image decode and prepare path, `layout`, `parse`, and
the writers. Not crate-wide, and not on test modules — `unwrap` in a test is an
assertion, and denying it there produces the noise that gets a deny switched
off.

`unwrap_used`, `expect_used`, `indexing_slicing`, `arithmetic_side_effects`.
**194 sites, every one converted, none allowed.** The counts are in the run
report; what matters here is which of them were more than lint-quieting.

**`indexing_slicing` produced more noise than `arithmetic_side_effects`** — 100
sites against 93 — against a prediction that the arithmetic lint would dominate
and might have to be dropped. It did not have to be dropped. The prediction was
also that the image path would be the slow half and the assemblers quick; the
assemblers were 97 sites to the image path's 80, and `parse` alone was 79.

Four sites were genuinely reachable, and all four are the same root cause.

### `Money` addition is unchecked, and document amounts reach it

`Money` is an `i64` of minor units. `Money::parse` bounds one value, but the
`Add` and `Sub` impls are bare `self.0 + rhs.0`.

Measured, through the real parser: a **two-row CSV** whose amounts are 5e16
units parses cleanly, and `total()` returns **−8446744073709551616** — a
negative movement from two positive amounts. The same document **panics inside
`from_document`** in a debug build. The worker profile inherits `release`,
where overflow checks are off, so production wraps silently.

This is not only a wrong number in a report. `score_pair` and
`score_debit_credit` decide which column is the balance by testing
`balance[i] == balance[i+1] + amount[i]`. A wrapped sum landing on the printed
balance is a hit the column never earned — a check passing *because* the
arithmetic broke. Those two are fixed: a sum that does not fit is not a match.
`Money::checked_add` and `checked_sub` exist and are used where the answer is
known.

**The operators themselves are still unchecked, and eleven call sites outside
the scoped modules still use them** — `bank::total`, `mobile_money`, `summary`,
`generic`, `payments`, `lib`. That is deliberate: what a verdict should do with
a total that does not fit is a product decision, not a lint fix. The options
are to saturate (a wrong number, quietly), to check and fail closed with a
named reason (P2's typed errors, arriving early), or to refuse an out-of-range
amount at parse. It needs deciding before P0 can close.

## The sandbox

Tested by violating each restriction from inside a worker, then falsified by
disabling the mechanism and confirming the test fails.

| Restriction | Enforced by | Holds here |
|---|---|---|
| No network | `sandbox-exec` profile, `deny network*` | yes, with a control proving the machine is online |
| No write outside one scratch directory | `deny file-write*` plus one `subpath` | yes |
| Read-only root | the same `deny file-write*` | yes |
| Scratch emptied after **every** job | the parent, after any outcome | yes, tested after an abort and after a kill |
| Bytes per file | `RLIMIT_FSIZE` | yes |
| Dropped privileges | `setgroups`/`setgid`/`setuid` before exec | **decision only** |

**The temp directory was emptied after no job at all.** The prediction was that
cleanup existed for a clean worker and not a crashed one. In fact `Scratch` was
referenced only by its own unit test — `run` never touched it, so nothing was
ever cleaned up on any path. It is the parent's now, because a worker killed at
the wall clock or aborted from C++ never reaches its own tidy-up, and those are
exactly the runs that leave a half-written page behind.

**A vacuous test, caught by falsification.** The read-only-root test wrote to
`/usr/local` and passed with sealing switched off: an unprivileged user cannot
write there anyway, so it tested the machine's permissions. It now writes
somewhere this user certainly can, with the unsealed control inside the test.

**`RLIMIT_NPROC` is not a fork-bomb ceiling and was removed.** It counts every
process owned by the *user*, not the worker's children. At 64 it was breached
by the machine's own processes, so `/bin/sh` could not fork and an unrelated
timeout test began failing depending on what else was running. Any value low
enough to stop a fork bomb breaks the user's other workers. The real control is
the pids cgroup controller, which is per-worker and lives on Linux.

### What the sandbox does not promise

**Linux is unsealed.** `sandbox-exec` is macOS-only and deprecated by Apple;
this is the development machine, not the deployment target. The Linux
equivalent — a network namespace, a mount namespace with a read-only root, the
pids cgroup — is **not written and not measured**, because there is no Linux
here to measure it on and a defence that never runs is not evidence.

`Confinement::Sealed` therefore **refuses to start a job** on any platform
where sealing is not implemented, rather than quietly downgrading it. A worker
that believes it has no network and does is worse than one that will not start,
because the belief is what the privacy claim is written against.

**Privilege dropping is a decision without an enforcement test.** The order —
supplementary groups, then group, then user — is right, and
`privileges_to_drop` is tested both ways. The `setuid` call itself never runs
on a machine that is not root, which is every development machine. It needs a
root suite on the deployment host.

## Still open

- **`Money`'s unchecked `Add`/`Sub`, and the eleven call sites that use them.**
  Blocks P0. See above.
- **Linux sealing**, and the root suite that would exercise privilege dropping.
  Blocks P0.
- Fuzzing `layout`/`parse` with hostile coordinates, and the OFX/QBO writers
  with hostile strings. The OFX writer matters most: a string that escapes its
  tag is an injection into the user's accounting software.
- `to_ofx_date` indexed a filtered vector against an unfiltered one — `n` drops
  any field too large for a `u32`, shifting every field after it. The month
  range check rejects the shifted case, so nothing reachable was wrong, but the
  alignment was an accident. Taken by name now; recorded because the *class*
  (two vectors assumed parallel when one is filtered) may exist elsewhere.
- `anydoc` owns the decompression that matters; the cap is a pre-flight here.
  Offer it upstream, and only then decide about a fork.
