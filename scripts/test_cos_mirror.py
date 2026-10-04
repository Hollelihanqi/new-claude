import json
import tempfile
import unittest
from pathlib import Path

from cos_mirror import NO_CACHE, PREFIX, encode_json, promote, release_plan, sha256, stage


BASE = "https://example-1250000000.cos.ap-guangzhou.myqcloud.com"
REPO = "example/pathmux"
TAG = "v3.2.0"


def source_manifest():
    return {
        "version": "3.2.0", "notes": "Release notes", "pub_date": "2026-10-04T00:00:00Z",
        "platforms": {
            target: {"signature": "signed-original-bytes", "url":
                     f"https://github.com/{REPO}/releases/download/{TAG}/{asset}"}
            for target, asset in (
                ("darwin-aarch64", "app.tar.gz"), ("darwin-x86_64", "app.tar.gz"),
                ("darwin-aarch64-app", "app.tar.gz"), ("darwin-x86_64-app", "app.tar.gz"),
                ("windows-x86_64", "setup.exe"), ("windows-x86_64-msi", "setup.msi"),
                ("windows-x86_64-nsis", "setup.exe"),
            )
        },
    }


class MemoryStore:
    def __init__(self):
        self.objects = {}
        self.writes = []

    def read(self, key):
        return self.objects.get(key)

    def put(self, key, data, cache_control):
        self.objects[key] = data
        self.writes.append((key, cache_control))

    def put_immutable(self, key, data):
        if key in self.objects and self.objects[key] != data:
            raise ValueError("Immutable asset changed")
        self.put(key, data, "immutable")

    def download_digest(self, url):
        key = url.removeprefix(BASE + "/")
        data = self.objects[key]
        return {"size": len(data), "sha256": sha256(data)}


class MirrorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.directory = Path(self.tmp.name)
        (self.directory / "latest.json").write_bytes(encode_json(source_manifest()))
        for asset in ("app.dmg", "app.tar.gz", "setup.msi", "setup.exe", "setup.exe.sig"):
            (self.directory / asset).write_bytes(asset.encode())
        self.store = MemoryStore()
        self.latest = {"tag_name": TAG, "draft": False, "prerelease": False}

    def stage(self, verify=None):
        stage(self.store, self.directory, TAG, REPO, BASE, verify or self.store.download_digest)

    def promote(self, latest=None, verify=None):
        promote(self.store, TAG, REPO, BASE, latest or self.latest,
                verify or self.store.download_digest)

    def test_all_platforms_signatures_and_release_fields_are_preserved(self):
        original = source_manifest()
        mirrored, required = release_plan(original, TAG, REPO, BASE,
                                          {"app.tar.gz", "setup.exe", "setup.msi"})
        self.assertEqual(required, {"app.tar.gz", "setup.exe", "setup.msi"})
        self.assertEqual(len(mirrored["platforms"]), 7)
        for target, spec in mirrored["platforms"].items():
            self.assertTrue(spec["url"].startswith(BASE + "/pathmux/releases/v3.2.0/"))
            spec["url"] = original["platforms"][target]["url"]
        self.assertEqual(original, mirrored)

    def test_foreign_url_missing_file_and_unsigned_platform_are_rejected(self):
        for modification in ("foreign", "missing", "unsigned", "traversal"):
            with self.subTest(modification=modification):
                manifest = source_manifest()
                spec = manifest["platforms"]["darwin-aarch64"]
                if modification == "foreign":
                    spec["url"] = "https://other.example/app.tar.gz"
                elif modification == "missing":
                    spec["url"] = spec["url"].replace("app.tar.gz", "missing.tar.gz")
                elif modification == "unsigned":
                    spec["signature"] = ""
                else:
                    spec["url"] = spec["url"].replace("app.tar.gz", "%2e%2e%2fapp.tar.gz")
                with self.assertRaises(ValueError):
                    release_plan(manifest, TAG, REPO, BASE, {"app.tar.gz", "setup.exe", "setup.msi"})

    def test_failed_public_download_leaves_stable_and_candidate_untouched(self):
        self.store.objects[f"{PREFIX}/stable/latest.json"] = b"previous"
        with self.assertRaises(ValueError):
            self.stage(lambda url: {"sha256": "corrupted", "size": 1})
        self.assertEqual(self.store.read(f"{PREFIX}/stable/latest.json"), b"previous")
        self.assertIsNone(self.store.read(f"{PREFIX}/candidates/{TAG}/latest.json"))

    def test_retry_refuses_changed_assets_at_same_version(self):
        self.stage()
        (self.directory / "setup.msi").write_bytes(b"different")
        with self.assertRaises(ValueError):
            self.stage()

    def test_stable_is_the_last_written_object_after_verified_downloads(self):
        self.stage()
        self.assertIsNone(self.store.read(f"{PREFIX}/stable/latest.json"))
        self.promote()
        self.assertEqual(self.store.writes[-1], (f"{PREFIX}/stable/latest.json", NO_CACHE))
        self.assertEqual(json.loads(self.store.read(f"{PREFIX}/stable/latest.json"))["version"], "3.2.0")

    def test_missing_object_prevents_promotion(self):
        self.stage()
        del self.store.objects[f"{PREFIX}/releases/{TAG}/app.tar.gz"]
        with self.assertRaises(KeyError):
            self.promote()
        self.assertIsNone(self.store.read(f"{PREFIX}/stable/latest.json"))

    def test_draft_preview_wrong_tag_and_rollback_are_rejected(self):
        self.stage()
        for changes in ({"draft": True}, {"prerelease": True}, {"tag_name": "v3.1.0"}):
            latest = {**self.latest, **changes}
            with self.assertRaises(ValueError):
                self.promote(latest)
        self.store.objects[f"{PREFIX}/stable/latest.json"] = encode_json({"version": "3.3.0"})
        with self.assertRaises(ValueError):
            self.promote()
        self.assertEqual(json.loads(self.store.read(f"{PREFIX}/stable/latest.json"))["version"], "3.3.0")

    def test_candidate_cannot_change_original_signature_even_with_updated_digest(self):
        self.stage()
        key = f"{PREFIX}/candidates/{TAG}"
        altered = json.loads(self.store.read(f"{key}/latest.json"))
        altered["platforms"]["darwin-aarch64"]["signature"] = "tampered"
        data = encode_json(altered)
        self.store.objects[f"{key}/latest.json"] = data
        receipt = json.loads(self.store.read(f"{key}/verification.json"))
        receipt["manifest_sha256"] = sha256(data)
        self.store.objects[f"{key}/verification.json"] = encode_json(receipt)
        with self.assertRaises(ValueError):
            self.promote()
        self.assertIsNone(self.store.read(f"{PREFIX}/stable/latest.json"))


if __name__ == "__main__":
    unittest.main()
