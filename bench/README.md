# bench

Benchmark harness. Python, but **no algorithm lives here**: it fetches datasets, decodes images,
calls the Rust core through `pagetriage_py`, and scores the output.

| File | Role |
|---|---|
| `datasets.toml` | The datasets: HTTPS URLs, sizes, SHA-256 pins, citation text |
| `fetch.py` | Download, verify and unpack into `bench/data/` (gitignored) |
| `run.py` | Run the analyser over a folder of images, write one JSON line per image |
| `metrics.py` | F-measure, PSNR, F1, IoU, box matching, ECE, Brier. pFM and DRD are not implemented yet (Phase 1) |
| `phash_eval.py` | Perceptual-hash evaluation: perturbed copies and distinct pairs of the 136 DIBCO pages, plus a synthetic typed-page arm. Writes `bench/results/phash_eval.json`; results in `docs/detectors/phash.md` |
| `tests/` | Offline unit tests (`python -I -m unittest discover -s bench/tests`) |

## Setup

```sh
python3 -m venv .venv && source .venv/bin/activate
pip install maturin -r bench/requirements.txt
maturin develop --release -m crates/pagetriage-py/Cargo.toml
```

## Datasets

```sh
python -I bench/fetch.py --list               # what there is, and sizes
python -I bench/fetch.py                       # everything (about 240 MB of downloads, 1.2 GB unpacked)
python -I bench/fetch.py --only dibco2014      # one set
python -I bench/fetch.py --verify              # re-check downloaded archives, no network
```

Run it with `python -I` so nothing in the data folder can be imported. The downloads are untrusted:
HTTPS only (redirects to HTTP are refused), size and SHA-256 checked before unpacking, no absolute
paths, `..` or symlinks, and a cap on unpacked size. `.rar` and `.7z` archives need `bsdtar`
(libarchive; ships with macOS, `apt install libarchive-tools`) or `7z`.

| Key | Source | Status |
|---|---|---|
| `dibco2009` ... `dibco2018`, `dibco2019_track_a`, `dibco2019_track_b` | DIBCO / H-DIBCO organisers (users.iit.demokritos.gr, utopia.duth.gr, vc.ee.duth.gr) | Downloaded and verified 2026-10-24. 136 images across 11 sets. There was no 2015 edition |
| `tobacco800` | IAPR TC11 | **Unverified.** The host `tc11.cvc.uab.es` timed out on 2026-10-24, so the live download, its contents and whether it needs a login are all unconfirmed. The URL comes from a Wayback copy of the page. `fetch.py` fails with a clear message if the host is down. Once it works, check the size and tar contents and pin the hash |

None of these hosts states a licence, so nothing here is redistributed from this repository. The
host asks Tobacco800 users to cite the references listed under it in `datasets.toml`.

Things that bite, all from the original archives:

- Folder names keep the organisers' typos (`DIBC02009_...` with a zero, `DIPCO2016_dataset`).
- 2011 and 2012 keep images and ground truth in the same folder (`*_GT.tiff`, `*_GT.tif`), so give
  `run.py` a `--glob` for the images.
- Image names repeat across editions (`H01`, `1.bmp`): always keep the edition in the path.
- The 2018 ground-truth archive also contains 20 `weights/` files, which are not images.
- Track B (papyri) carries an `info.txt` crediting the source collections; third-party image
  rights are not stated.

## Running the analyser over a set

`run.py` takes an explicit folder (`--images`) because the extracted layouts differ by year:

| Set | `--images` (under `bench/data/`) | `--glob` |
|---|---|---|
| `dibco2009` | `dibco2009/DIBC02009_Test_images-handwritten/DIBC02009_Test_images-handwritten` and `dibco2009/DIBCO2009_Test_images-printed/DIBCO2009_Test_images-printed` | `*` |
| `dibco2010` | `dibco2010/H_DIBCO2010_test_images` | `*` |
| `dibco2011` | `dibco2011/DIBCO11-handwritten`, `dibco2011/DIBCO11-machine_printed` | `*.png` |
| `dibco2012` | `dibco2012/H-DIBCO2012-dataset/H-DIBCO2012-dataset/H-DIBCO2012-dataset` | `*.png` |
| `dibco2013` | `dibco2013/DIBCO2013-dataset/OriginalImages` | `*` |
| `dibco2014` | `dibco2014/original_images` | `*` |
| `dibco2016` | `dibco2016/DIBCO2016_dataset-original/DIPCO2016_dataset` | `*` |
| `dibco2017` | `dibco2017/DIBCO2017_Dataset/Dataset` | `*` |
| `dibco2018` | `dibco2018/dibco2018_Dataset/dataset` | `*` |
| `dibco2019_track_a` | `dibco2019_track_a/dibco2019_dataset_trackA/Dataset` | `*` |
| `dibco2019_track_b` | `dibco2019_track_b/dibco2019_dataset_trackB/Dataset` | `*` (skips `info.txt`) |

```sh
python -I bench/run.py --images bench/data/dibco2014/original_images --debug --out bench/results/dibco2014.jsonl
```

Ground truth sits beside each set (for example `dibco2014/GT/H01_estGT.tiff`, `dibco2018/dibco2018-GT/gt`).
DIBCO ground truth stores ink as black, so use `metrics.ink_mask(...)` before scoring.

Nothing under `bench/data/` or `bench/results/` is committed. Never put real deed pages in either.
