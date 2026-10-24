"""Offline tests for bench/fetch.py: manifest rules, download verification, safe unpacking."""
import hashlib
import io
import os
import sys
import tempfile
import unittest
import urllib.error
import zipfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fetch  # noqa: E402


class FakeResponse(io.BytesIO):
    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def opener_for(payload: bytes):
    return lambda url: FakeResponse(payload)


def quiet(*_a, **_k):
    pass


def make_zip(path: Path, entries: dict[str, bytes]) -> None:
    with zipfile.ZipFile(path, "w") as zf:
        for name, data in entries.items():
            zf.writestr(name, data)


class ManifestTests(unittest.TestCase):
    def test_checked_in_manifest_is_valid(self):
        datasets = fetch.load_manifest()
        self.assertGreaterEqual(len(datasets), 11)
        self.assertIn("dibco2014", datasets)
        self.assertIn("tobacco800", datasets)
        self.assertIs(datasets["tobacco800"]["verified"], False)

    def test_every_dibco_archive_is_pinned_and_only_tobacco800_is_not(self):
        unpinned = [
            (key, fetch.target_name(f))
            for key, ds in fetch.load_manifest().items()
            for f in ds["files"]
            if not f["sha256"]
        ]
        self.assertEqual(unpinned, [("tobacco800", "tobacco_data_zhugy.tar")])

    def test_rejects_http_bad_hash_and_bad_size(self):
        bad = {
            "x": {"files": [{"url": "http://example.org/a.zip", "sha256": "zz", "size": -1}]},
        }
        problems = "\n".join(fetch.validate_manifest(bad))
        self.assertIn("must be https", problems)
        self.assertIn("sha256", problems)
        self.assertIn("size", problems)

    def test_target_name_cannot_escape_the_downloads_dir(self):
        self.assertEqual(fetch.target_name({"url": "https://h/p/a.zip"}), "a.zip")
        self.assertEqual(fetch.target_name({"url": "https://h/p/a.zip", "filename": "../evil"}), "")
        self.assertEqual(fetch.target_name({"url": "https://h/p/a.zip", "filename": "/etc/passwd"}), "")
        self.assertEqual(fetch.target_name({"url": "https://h/"}), "")


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))

    def test_good_download_is_verified_and_renamed(self):
        data = b"hello dataset"
        entry = {"url": "https://h/a.zip", "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}
        dest = self.tmp / "a.zip"
        digest = fetch.download(entry, dest, opener_for(data), quiet)
        self.assertEqual(digest, entry["sha256"])
        self.assertEqual(dest.read_bytes(), data)
        self.assertFalse(dest.with_name("a.zip.part").exists())

    def test_hash_mismatch_leaves_nothing_behind(self):
        entry = {"url": "https://h/a.zip", "sha256": "0" * 64}
        dest = self.tmp / "a.zip"
        with self.assertRaisesRegex(fetch.FetchError, "sha256"):
            fetch.download(entry, dest, opener_for(b"tampered"), quiet)
        self.assertFalse(dest.exists())
        self.assertFalse(dest.with_name("a.zip.part").exists())

    def test_size_mismatch_is_rejected(self):
        entry = {"url": "https://h/a.zip", "size": 5}
        with self.assertRaisesRegex(fetch.FetchError, "size"):
            fetch.download(entry, self.tmp / "a.zip", opener_for(b"too long"), quiet)

    def test_existing_good_file_is_not_downloaded_again(self):
        data = b"cached"
        dest = self.tmp / "a.zip"
        dest.write_bytes(data)
        entry = {"url": "https://h/a.zip", "sha256": hashlib.sha256(data).hexdigest()}

        def boom(url):
            raise AssertionError("must not hit the network")

        fetch.download(entry, dest, boom, quiet)

    def test_stale_file_is_replaced(self):
        dest = self.tmp / "a.zip"
        dest.write_bytes(b"old")
        new = b"new contents"
        entry = {"url": "https://h/a.zip", "sha256": hashlib.sha256(new).hexdigest()}
        fetch.download(entry, dest, opener_for(new), quiet)
        self.assertEqual(dest.read_bytes(), new)

    def test_unreachable_host_gives_a_clear_error_and_no_part_file(self):
        def down(url):
            raise urllib.error.URLError("timed out")

        dest = self.tmp / "a.zip"
        with self.assertRaisesRegex(fetch.FetchError, "host may be down"):
            fetch.download({"url": "https://h/a.zip"}, dest, down, quiet)
        self.assertFalse(dest.with_name("a.zip.part").exists())

    def test_default_opener_refuses_plain_http(self):
        with self.assertRaisesRegex(fetch.FetchError, "non-https"):
            fetch.default_open("http://example.org/a.zip")


class UnpackTests(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp, ignore_errors=True))

    def test_normal_zip_unpacks(self):
        archive = self.tmp / "ok.zip"
        make_zip(archive, {"img/1.bmp": b"a", "img/GT/1.bmp": b"b"})
        dest = self.tmp / "out"
        fetch.unpack(archive, dest)
        self.assertEqual((dest / "img" / "1.bmp").read_bytes(), b"a")

    def test_path_traversal_and_absolute_paths_are_refused_and_cleaned_up(self):
        for evil in ("../escape.txt", "a/../../escape.txt", "/abs.txt", "C:/win.txt"):
            archive = self.tmp / "evil.zip"
            make_zip(archive, {evil: b"x"})
            dest = self.tmp / "out"
            with self.assertRaises(fetch.FetchError, msg=evil):
                fetch.unpack(archive, dest)
            self.assertFalse(dest.exists(), f"{evil}: partial output left behind")
            self.assertFalse((self.tmp.parent / "escape.txt").exists())

    def test_symlink_entry_is_refused(self):
        archive = self.tmp / "link.zip"
        with zipfile.ZipFile(archive, "w") as zf:
            info = zipfile.ZipInfo("link")
            info.external_attr = (0o120777 << 16)
            zf.writestr(info, "/etc/passwd")
        with self.assertRaisesRegex(fetch.FetchError, "symlink"):
            fetch.unpack(archive, self.tmp / "out")

    def test_zip_bomb_guard(self):
        archive = self.tmp / "big.zip"
        make_zip(archive, {"a.bin": b"\0" * 10_000})
        with self.assertRaisesRegex(fetch.FetchError, "refusing"):
            fetch.extract_zip(archive, self.tmp / "out", max_bytes=1_000)

    def test_verify_tree_rejects_symlinks(self):
        root = self.tmp / "tree"
        root.mkdir()
        (root / "real.txt").write_text("x")
        os.symlink(root / "real.txt", root / "link.txt")
        with self.assertRaisesRegex(fetch.FetchError, "symlink"):
            fetch.verify_tree(root)

    def test_sha256_file(self):
        p = self.tmp / "f"
        p.write_bytes(b"abc")
        self.assertEqual(
            fetch.sha256_file(p), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )


if __name__ == "__main__":
    unittest.main()
