# Releasing

Nothing in this repository publishes by itself. Each step below is a deliberate command run by the person
who owns the package names and the registry accounts. The dry runs they rely on are scripts that run
locally; a CI job for them is given at the end (it is not installed: the account that pushed this
branch could not edit workflow files).

## What is checked before a release

| Check | Command | What it proves |
|---|---|---|
| Determinism gate | `scripts/determinism.sh` | Native, Python and WASM reports are byte-identical on every golden input; the WASM build is inside its size budget |
| Bench harness tests | `python -I -m unittest discover -s bench/tests` | The benchmark runner still agrees with the golden reports |
| Site build | `scripts/build-site.sh` | The in-browser WASM build reproduces the native fingerprints in `site/samples/samples.json` (regenerate it with `site/tools/make_samples.py` when a report changes) |
| npm tarball | `scripts/build-wasm.sh && scripts/package-npm.sh` | The tarball installs in a scratch project and `require("pagetriage")` analyses a page |
| Python wheel | `scripts/package-python.sh` | The wheel installs into a fresh virtual environment and `pagetriage_py` analyses a page |
| Crate | `cargo package -p pagetriage-core` | The core crate builds from its own tarball |

## Publishing, in order

1. **Version.** Set `version` under `[workspace.package]` in `Cargo.toml`; the wheel and the npm package
   take it from there. Commit, tag `vX.Y.Z`.
2. **crates.io.** `cargo publish -p pagetriage-core`, then (once the index has it) `cargo publish -p
   pagetriage-cli`. `pagetriage-py` and `pagetriage-wasm` are marked `publish = false`: they are shipped
   as a wheel and an npm package.
3. **PyPI.** Build wheels for each platform in a manylinux container (or with `maturin-action` in CI), so
   the tags are portable (the wheel `scripts/package-python.sh` builds carries this machine's glibc tag),
   then `maturin upload` or `twine upload dist/wheels/*`. The wheel is abi3: one per platform serves
   CPython 3.9 and later.
4. **npm.** `npm publish dist/npm/pagetriage-X.Y.Z.tgz` (add `--access public` for a scoped name). The
   package name `pagetriage` has not been checked for availability.
5. **Site.** The `pages` workflow deploys `site/` on every push to `main` that touches it; it refuses to
   deploy when the WASM build stops reproducing the native fingerprints.

## What a release does not claim

Every detector from module M4 on was checked on synthetic pages only, and no confidence is calibrated
(`docs/detectors/fusion.md`). A release note should say so in the same words.

## CI job for the packaging dry runs (not installed)

Add this under `jobs:` in `.github/workflows/ci.yml`:

```yaml
  # Packaging dry runs: the npm tarball and the Python wheel are built, installed in a scratch project,
  # and run. Nothing is published from CI.
  packaging:
    name: packaging dry runs (npm tarball, Python wheel, crate)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - uses: actions/setup-node@v4
        with:
          node-version: "22"
      - name: Install pinned wasm-bindgen CLI and wasm-opt
        run: scripts/install-wasm-tools.sh
      - run: scripts/build-wasm.sh
      - name: npm tarball installs and runs under Node
        run: scripts/package-npm.sh
      - name: Python wheel installs and runs in a fresh virtual environment
        run: scripts/package-python.sh
      - name: Core crate packages and builds from its tarball
        run: cargo package -p pagetriage-core
```
