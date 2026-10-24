# pagetriage

A deterministic, non-ML page-content analyser for degraded scanned legal documents, written in Rust.

For each page it will report **what the page contains**: text layer vs scan, script, handwriting,
stamps and seals, signatures, thumb impressions, rulings, fold lines, scan quality. Every finding
carries a calibrated confidence, a bounding region, and an evidence list of named measurements with
values and thresholds. It uses only classical computer vision and PDF structure: no neural networks
and no model files, so the same input gives the same output, byte for byte, on every platform. The
main target is Indian property paperwork (sale deeds, affidavits, notarised documents): multi-
generation photocopies with skew, bleed-through, rubber stamps, wet signatures and thumb
impressions, in English mixed with Devanagari and other Indic scripts.

One core crate compiles to a native CLI, a Python wheel (PyO3), and browser/Node WASM.

## Status: Phase 0 (skeleton)

The plumbing is built and tested; the detectors are not. Today it can do this:

- **Analyse a page image** (PNG, JPEG, TIFF, PNM, BMP) and print a JSON report that validates
  against a checked-in JSON Schema (`schema/pagetriage-0.1.schema.json`).
- **Produce byte-identical reports on native, Python and WASM**, checked against golden snapshots on
  every CI run (`scripts/determinism.sh`). The WASM build is about 150 KB gzipped.
- **One placeholder detector**, `blank_page`, which exists to exercise the whole path (pixels,
  evidence, fusion, finding). It is known to be wrong on faded scans and dark borders; the failures
  are written up in [`docs/detectors/blank_page.md`](docs/detectors/blank_page.md), including what
  it does on real benchmark pages.
- A benchmark harness (`bench/`) that downloads and verifies the DIBCO datasets and runs the
  analyser over them.

Not done yet: PDF input, every real detector (script, handwriting, stamps, signatures, thumbprints,
Indian-deed specifics), calibration, and the npm package. See [`docs/modules.md`](docs/modules.md)
for the module list and the phase each one lands in.

## Try it

```sh
cargo run -p pagetriage-cli -- analyse page.png --pretty
cargo run -p pagetriage-cli -- analyse page.png --debug      # include raw detector features
cargo run -p pagetriage-cli -- schema                        # JSON Schema of the report
cargo run -p pagetriage-cli -- params                        # embedded default thresholds, as TOML
```

Run the checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
scripts/determinism.sh        # needs maturin, node, and the pinned wasm-bindgen CLI + wasm-opt
                              # (scripts/install-wasm-tools.sh installs the pinned versions)
```

## Layout

| Path | What |
|---|---|
| `crates/pagetriage-core` | The analyser: schema types, params, fusion, detectors. Pure Rust, no I/O |
| `crates/pagetriage-cli` | `pagetriage` command line |
| `crates/pagetriage-py` | Python bindings (PyO3, built with maturin) |
| `crates/pagetriage-wasm` | WASM bindings (wasm-bindgen) |
| `schema/` | Generated JSON Schema, checked in and guarded by a drift test |
| `tests/golden`, `tests/determinism` | Synthetic golden pages and the cross-runtime runners |
| `bench/` | Dataset fetcher, runner and metrics. Datasets are never committed |
| `docs/` | Module list, per-detector pages, [references](docs/REFERENCES.md) |
| `CLAUDE.md` | Project rules, including the determinism rules |

## Design rules

No dependency may load a model file or use a neural network. Every threshold lives in a TOML file
with its source named. Platform math functions (`exp`, `ln`, `sin`, ...) and `HashMap` are banned by
clippy because they break byte-for-byte reproducibility. The full list is in `CLAUDE.md`.

## Data and privacy

Never commit real document pages: deeds contain personal data. The golden test pages are synthetic.
The benchmark datasets have no stated licence, so they are fetched at runtime and not redistributed.

## Licence

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT licence](LICENSE-MIT)
at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 licence, shall be dual licensed as above, without
any additional terms or conditions.
