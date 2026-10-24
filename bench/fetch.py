#!/usr/bin/env python3
"""Download and unpack the benchmark datasets listed in bench/datasets.toml.

    python bench/fetch.py --list
    python bench/fetch.py                       # everything
    python bench/fetch.py --only dibco2014,dibco2016
    python bench/fetch.py --verify              # re-check downloaded archives, no network

Everything fetched is untrusted data. This script:
  * only talks HTTPS,
  * checks size and SHA-256 against the manifest before anything is unpacked,
  * unpacks into its own fresh directory, refusing absolute paths, `..`, symlinks and archives that
    expand past a size cap,
  * never executes anything it downloaded.

Datasets land in bench/data/ (gitignored). Their terms forbid redistribution or grant no licence, so
they must never be committed. Run it with `python -I` so nothing in the data directory is importable.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_MANIFEST = HERE / "datasets.toml"
DEFAULT_DATA_DIR = HERE / "data"
USER_AGENT = "pagetriage-bench-fetch/0.1 (+https://github.com/jtushar53/pagetriage)"
CHUNK = 1 << 20
TIMEOUT_S = 60
MAX_EXTRACTED_BYTES = 8 * 1024**3  # archive-bomb guard


class FetchError(Exception):
    """A dataset could not be fetched or unpacked; the message says why."""


# --------------------------------------------------------------------------- manifest


def load_manifest(path: Path = DEFAULT_MANIFEST) -> dict:
    with open(path, "rb") as f:
        datasets = tomllib.load(f).get("datasets", {})
    problems = validate_manifest(datasets)
    if problems:
        raise FetchError("invalid manifest:\n  " + "\n  ".join(problems))
    return datasets


def validate_manifest(datasets: dict) -> list[str]:
    problems = []
    if not datasets:
        problems.append("no [datasets.*] tables")
    for key, ds in datasets.items():
        files = ds.get("files", [])
        if not files:
            problems.append(f"{key}: no files")
        for i, f in enumerate(files):
            where = f"{key}.files[{i}]"
            url = f.get("url", "")
            if urllib.parse.urlsplit(url).scheme != "https":
                problems.append(f"{where}: url must be https, got {url!r}")
            sha = f.get("sha256", "")
            if sha and (len(sha) != 64 or any(c not in "0123456789abcdef" for c in sha)):
                problems.append(f"{where}: sha256 must be 64 lowercase hex characters or empty")
            if "size" in f and (not isinstance(f["size"], int) or f["size"] <= 0):
                problems.append(f"{where}: size must be a positive integer when given")
            if not target_name(f):
                problems.append(f"{where}: cannot derive a file name from the url; set `filename`")
    return problems


def target_name(file_entry: dict) -> str:
    """Local file name for a manifest entry: explicit `filename`, else the URL's last path part."""
    name = file_entry.get("filename") or Path(urllib.parse.urlsplit(file_entry["url"]).path).name
    # Never let a manifest entry choose a path outside the downloads directory.
    return name if name and name == Path(name).name and name not in (".", "..") else ""


# --------------------------------------------------------------------------- download


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while chunk := f.read(CHUNK):
            h.update(chunk)
    return h.hexdigest()


class _HttpsOnlyRedirect(urllib.request.HTTPRedirectHandler):
    """urllib follows https -> http redirects by default, which would defeat the https-only rule."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if urllib.parse.urlsplit(newurl).scheme != "https":
            raise FetchError(f"refusing redirect to non-https url: {newurl}")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


_OPENER = urllib.request.build_opener(_HttpsOnlyRedirect)


def default_open(url: str):
    if urllib.parse.urlsplit(url).scheme != "https":
        raise FetchError(f"refusing non-https url: {url}")
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    return _OPENER.open(req, timeout=TIMEOUT_S)


def download(entry: dict, dest: Path, open_url=default_open, log=print) -> str:
    """Download one manifest entry to `dest` and return its SHA-256.

    Reuses an existing file that matches the manifest. Writes to `dest.part` and only renames on
    success, so an interrupted run never leaves a file that looks complete.
    """
    url, want_size, want_sha = entry["url"], entry.get("size"), entry.get("sha256", "")
    if dest.exists():
        have = sha256_file(dest)
        if (want_sha and have == want_sha) or (not want_sha and want_size in (None, dest.stat().st_size)):
            log(f"  have  {dest.name}")
            return have
        log(f"  stale {dest.name} does not match the manifest; downloading again")
        dest.unlink()

    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    h = hashlib.sha256()
    received = 0
    log(f"  get   {url}")
    try:
        with open_url(url) as resp, open(part, "wb") as out:
            while chunk := resp.read(CHUNK):
                out.write(chunk)
                h.update(chunk)
                received += len(chunk)
    except (urllib.error.URLError, TimeoutError, ConnectionError, OSError) as e:
        part.unlink(missing_ok=True)
        reason = getattr(e, "reason", e)
        raise FetchError(f"cannot download {url}: {reason}. The host may be down; try again later.") from e

    digest = h.hexdigest()
    problem = None
    if want_size is not None and received != want_size:
        problem = f"size {received} != expected {want_size}"
    elif want_sha and digest != want_sha:
        problem = f"sha256 {digest} != expected {want_sha}"
    if problem:
        part.unlink(missing_ok=True)
        raise FetchError(
            f"{dest.name}: {problem}. The host changed the file or served something else; "
            "inspect it before updating bench/datasets.toml."
        )
    part.replace(dest)
    if not want_sha:
        log(f"  note  {dest.name} is not pinned in the manifest; pin it with: sha256 = \"{digest}\"")
    return digest


# --------------------------------------------------------------------------- unpack


def _inside(root: Path, candidate: Path) -> bool:
    try:
        candidate.resolve().relative_to(root.resolve())
        return True
    except ValueError:
        return False


def extract_zip(archive: Path, dest: Path, max_bytes: int = MAX_EXTRACTED_BYTES) -> None:
    with zipfile.ZipFile(archive) as zf:
        infos = zf.infolist()
        if sum(i.file_size for i in infos) > max_bytes:
            raise FetchError(f"{archive.name}: expands to more than {max_bytes} bytes; refusing")
        for info in infos:
            name = info.filename
            parts = Path(name).parts
            if name.startswith(("/", "\\")) or ".." in parts or (parts and ":" in parts[0]):
                raise FetchError(f"{archive.name}: unsafe path in archive: {name!r}")
            # Unix mode 0o120000 is a symlink; plain extraction would write it as a regular file,
            # but a symlink entry in a dataset archive is never legitimate, so refuse it outright.
            if (info.external_attr >> 16) & 0o170000 == 0o120000:
                raise FetchError(f"{archive.name}: symlink entry in archive: {name!r}")
        dest.mkdir(parents=True, exist_ok=True)
        for info in infos:
            target = dest / info.filename
            if not _inside(dest, target):
                raise FetchError(f"{archive.name}: path escapes the target directory: {info.filename!r}")
        zf.extractall(dest)


def _external_extractor() -> list[str]:
    for tool, args in (("bsdtar", ["bsdtar", "-xf"]), ("7zz", ["7zz", "x", "-y"]), ("7z", ["7z", "x", "-y"])):
        if shutil.which(tool):
            return args
    raise FetchError(
        "need an extractor for .rar/.7z/.tar archives: install libarchive's `bsdtar` "
        "(macOS ships it; `apt install libarchive-tools`) or 7-Zip (`7z`)"
    )


def extract_external(archive: Path, dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    cmd = _external_extractor()
    if cmd[0] == "bsdtar":
        full = [*cmd, str(archive), "-C", str(dest)]
    else:
        full = [*cmd, str(archive), f"-o{dest}"]
    result = subprocess.run(full, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise FetchError(f"{archive.name}: extraction failed ({cmd[0]}): {result.stderr.strip()[:300]}")


def verify_tree(root: Path, max_bytes: int = MAX_EXTRACTED_BYTES) -> int:
    """After unpacking: no symlinks or escapes, and a size cap. Returns total bytes."""
    total = 0
    for path in root.rglob("*"):
        if path.is_symlink():
            raise FetchError(f"symlink in extracted data: {path.relative_to(root)}")
        if not _inside(root, path):
            raise FetchError(f"extracted path escapes the target: {path}")
        if path.is_file():
            total += path.stat().st_size
            if total > max_bytes:
                raise FetchError(f"extracted data exceeds {max_bytes} bytes; refusing")
    return total


def unpack(archive: Path, dest: Path) -> None:
    """Unpack into a fresh `dest`; on any failure `dest` is removed so nothing half-done remains."""
    if dest.exists():
        shutil.rmtree(dest)
    try:
        if zipfile.is_zipfile(archive):
            extract_zip(archive, dest)
        else:
            extract_external(archive, dest)
        verify_tree(dest)
    except BaseException:
        shutil.rmtree(dest, ignore_errors=True)
        raise


# --------------------------------------------------------------------------- driver


def fetch_dataset(key: str, ds: dict, data_dir: Path, extract: bool = True, open_url=default_open, log=print) -> None:
    log(f"[{key}] {ds.get('name', key)}")
    if ds.get("verified") is False:
        log("  warn  this entry is marked unverified in the manifest; see the note in bench/datasets.toml")
    downloads = data_dir / "downloads" / key
    out_dir = data_dir / key
    marker = out_dir / ".complete.json"

    digests = {}
    for entry in ds["files"]:
        digests[target_name(entry)] = download(entry, downloads / target_name(entry), open_url, log)

    if not extract:
        return
    if marker.exists() and json.loads(marker.read_text()) == digests:
        log("  have  extracted data is current")
        return
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True)
    for name in digests:
        archive = downloads / name
        stem = name[: -len("".join(Path(name).suffixes))] if Path(name).suffixes else name
        log(f"  unpack {name}")
        unpack(archive, out_dir / stem)
    marker.write_text(json.dumps(digests, indent=2, sort_keys=True) + "\n")


def verify_dataset(key: str, ds: dict, data_dir: Path, log=print) -> bool:
    ok = True
    for entry in ds["files"]:
        path = data_dir / "downloads" / key / target_name(entry)
        if not path.exists():
            log(f"  missing {path.name}")
            ok = False
            continue
        want = entry.get("sha256", "")
        have = sha256_file(path)
        if want and have != want:
            log(f"  BAD     {path.name}: sha256 {have} != {want}")
            ok = False
        elif not want:
            log(f"  unpinned {path.name}: sha256 {have}")
        else:
            log(f"  ok      {path.name}")
    return ok


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--manifest", type=Path, default=DEFAULT_MANIFEST)
    ap.add_argument("--data-dir", type=Path, default=DEFAULT_DATA_DIR)
    ap.add_argument("--only", help="comma-separated dataset keys")
    ap.add_argument("--list", action="store_true", help="list datasets and exit")
    ap.add_argument("--verify", action="store_true", help="check downloaded archives against the manifest; no network")
    ap.add_argument("--no-extract", action="store_true", help="download only")
    args = ap.parse_args(argv)

    try:
        datasets = load_manifest(args.manifest)
    except (FetchError, OSError, tomllib.TOMLDecodeError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2

    if args.list:
        for key, ds in datasets.items():
            total = sum(f.get("size", 0) for f in ds["files"])
            flag = "  (UNVERIFIED)" if ds.get("verified") is False else ""
            print(f"{key:<20} {total / 1e6:8.1f} MB  {ds.get('name', '')}{flag}")
        return 0

    keys = list(datasets)
    if args.only:
        keys = [k.strip() for k in args.only.split(",") if k.strip()]
        unknown = [k for k in keys if k not in datasets]
        if unknown:
            print(f"error: unknown dataset(s): {', '.join(unknown)}; see --list", file=sys.stderr)
            return 2

    failed = []
    for key in keys:
        try:
            if args.verify:
                print(f"[{key}]")
                if not verify_dataset(key, datasets[key], args.data_dir):
                    failed.append(key)
            else:
                fetch_dataset(key, datasets[key], args.data_dir, extract=not args.no_extract)
        except FetchError as e:
            print(f"  FAIL  {e}", file=sys.stderr)
            failed.append(key)

    if failed:
        print(f"\nfailed: {', '.join(failed)}", file=sys.stderr)
        return 1
    print("\nall requested datasets are in place under", args.data_dir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
