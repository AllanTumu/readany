# Releasing

`readany` ships to two registries from one source tree: the crate to
crates.io, and a WebAssembly binary to npm. The binary is the reason this
document exists — a source release carries no obligations beyond its own
licence, and a binary release carries several.

## The short version

```bash
# 1. Bump Cargo.toml and wasm/Cargo.toml to the same number, by hand.
#    They must match: wasm/src/lib.rs will not compile otherwise.
./scripts/version.sh          # stamps npm/package.json from Cargo.toml

# 2. Write the changelog entry. Group it by hand and name what breaks.

# 3. Commit, then tag.
git commit -am "Release 0.3.0"
git push
git tag v0.3.0 && git push --tags
```

The tag runs `.github/workflows/release.yml`, which verifies and then publishes
to both registries. **It will not publish anything that fails verification**,
and it skips a registry that already has the version, so a half-finished
release can be re-run rather than untangled by hand.

## What it verifies before publishing anything

| Check | Why it exists |
|---|---|
| tag matches `Cargo.toml` | a `v0.3.0` tag on 0.2.0 source would publish the wrong thing under the right name |
| `scripts/version.sh --check` | four manifests used to carry four copies of one number |
| `clippy -D warnings`, crate and binding | the binding is not covered by the crate's lint run |
| `cargo test --release` | the old workflow published without running a single test |
| `scripts/build-wasm.sh` | one version, no build-machine paths, licences regenerated |
| `wasm/test/binding.test.js` | `cargo check` passes whatever the binding does; this is the only thing watching the artefact npm gets |
| blob sweep | IBAN-, card-, money-, email- and path-shaped strings must all be zero |

`workflow_dispatch` runs everything and publishes nothing unless you clear the
`dry_run` box. Use it to rehearse.

## Tokens

Both are repository secrets. **Neither is set at the time of writing**, which is
why the v0.1.0 and v0.2.0 workflow runs failed at `npm publish` while the
releases themselves went out from a laptop.

- **`NPM_TOKEN`** — npmjs.com → Access Tokens → Generate → **Granular Access
  Token**, scoped to the `readany` package only, read and write, with an
  expiry. A classic Automation token also works; granular is better because it
  cannot touch anything else if it leaks.
- **`CARGO_REGISTRY_TOKEN`** — crates.io → Account Settings → API Tokens, scoped
  to `publish-update` for the `readany` crate.

Both bypass two-factor authentication by design. That is the trade being made:
a credential in CI can publish without you present, which is exactly what makes
a tag-triggered release possible, and exactly why it should be scoped to one
package and given an expiry.

## Publishing by hand

Still supported and occasionally necessary. crates.io requires a **verified
email address**, not merely a login — an unverified account fails with a 400
that names `https://crates.io/settings/profile`. npm with 2FA on will open a
browser or ask for `--otp`.

```bash
./scripts/build-wasm.sh
cargo publish
cd pkg && npm publish --access public
git tag v0.3.0 && git push --tags
```

The npm account is **`qa-green-coder`**, which is not a name anything in this
repository would lead you to guess. `npm owner ls readany` confirms it.

If npm answers `E404 Not Found - PUT`, that is not a missing package. npm
returns 404 rather than 403 so as not to confirm a package exists to someone
who cannot see it. Read it as "not authorised".

## What the binary release owes

- **Third-party notices.** The blob statically links around a hundred crates
  under MIT, Apache-2.0, BSD, Zlib and Unlicense, every one of which requires
  its notice to travel with a binary. `scripts/third-party-licenses.py`
  regenerates `THIRD-PARTY-LICENSES.md` at every build and it ships in the
  package. It is **not** in the crates.io tarball, deliberately: that tarball is
  source and links nothing, so carrying other projects' notices in it would
  misstate what is being distributed.
- **No build-machine paths.** 0.1.0 shipped 234 of them, spelling out a
  username and a directory layout. `trim-paths` is still unstable on stable
  cargo, so the build uses `--remap-path-prefix` and then greps the blob to
  prove it worked. **rustc applies the last matching remap rule, not the
  first** — broadest prefix first, most specific last.
- **No document data.** The README once printed a real person's shop receipt
  verbatim, and the same merchant name had reached a stub backend in
  `src/lib.rs` and a doc comment in `ocr/image/correction.rs`. The workflow
  sweeps the blob; the source is on you. Read the fixtures before you tag.

## Versioning

`0.x`, and honestly so. Several things broke in 0.2.0 and `DecodeImage` still
has to grow a rasterising counterpart, which will break more. Name every
breaking change in `CHANGELOG.md` under its own heading — the npm surface and
the Rust surface break independently and a consumer of one does not read the
other.
