import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

from cos_mirror import CosStore, NO_CACHE, PREFIX, encode_json, promote, release_plan, sha256, stage


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

    def put_immutable_file(self, key, path, size, digest):
        data = path.read_bytes()
        if len(data) != size or sha256(data) != digest:
            raise ValueError("File changed during upload")
        self.put_immutable(key, data)

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


class MultipartTests(unittest.TestCase):
    def test_persistent_failure_aborts_without_completing_or_uploading_later_parts(self):
        class ServiceError(Exception):
            def __init__(self, status):
                self.status = status

            def get_status_code(self):
                return self.status

        class Client:
            def __init__(self, failure):
                self.failure = failure
                self.calls = []
                self.aborted = False
                self.completed = False

            def head_object(self, **kwargs):
                raise ServiceError(404)

            def create_multipart_upload(self, **kwargs):
                return {"UploadId": "test-upload"}

            def upload_part(self, **kwargs):
                self.calls.append(kwargs["PartNumber"])
                raise self.failure

            def abort_multipart_upload(self, **kwargs):
                self.aborted = True

            def complete_multipart_upload(self, **kwargs):
                self.completed = True

        for failure, attempts in ((TimeoutError("write timeout"), 3),
                                  (ServiceError(503), 3), (ServiceError(403), 1),
                                  (RuntimeError("invalid part"), 1)):
            with self.subTest(error=type(failure).__name__, attempts=attempts), \
                    tempfile.TemporaryDirectory() as temp, patch.dict(
                        sys.modules, {"qcloud_cos": types.SimpleNamespace(
                            CosServiceError=ServiceError, CosClientError=TimeoutError
                        )}
                    ), patch("cos_mirror.PART_SIZE", 4), patch("cos_mirror.time.sleep"):
                path = Path(temp) / "installer.dmg"
                path.write_bytes(b"abcdefgh")
                store = CosStore.__new__(CosStore)
                store.bucket = "test-bucket"
                store.client = Client(failure)
                with self.assertRaises(type(failure)):
                    store.put_immutable_file("pathmux/releases/v3.2.0/installer.dmg", path, 8, sha256(path.read_bytes()))
                self.assertEqual(store.client.calls, [1] * attempts)
                self.assertTrue(store.client.aborted)
                self.assertFalse(store.client.completed)

    def test_timeout_retries_only_the_failed_part_with_identical_bytes(self):
        class Missing(Exception):
            def get_status_code(self):
                return 404

        class ClientError(Exception):
            pass

        class Client:
            def __init__(self):
                self.calls = []
                self.completed = None

            def head_object(self, **kwargs):
                raise Missing()

            def create_multipart_upload(self, **kwargs):
                return {"UploadId": "test-upload"}

            def upload_part(self, **kwargs):
                self.calls.append((kwargs["PartNumber"], kwargs["Body"]))
                if len(self.calls) == 1:
                    raise ClientError("write operation timed out")
                return {"ETag": str(kwargs["PartNumber"])}

            def complete_multipart_upload(self, **kwargs):
                self.completed = kwargs["MultipartUpload"]["Part"]

        with tempfile.TemporaryDirectory() as temp, patch.dict(
            sys.modules, {"qcloud_cos": types.SimpleNamespace(
                CosServiceError=Missing, CosClientError=ClientError
            )}
        ), patch("cos_mirror.PART_SIZE", 4), patch("cos_mirror.time.sleep"):
            path = Path(temp) / "installer.dmg"
            path.write_bytes(b"abcdefghijkl")
            store = CosStore.__new__(CosStore)
            store.bucket = "test-bucket"
            store.client = Client()
            store.put_immutable_file("pathmux/releases/v3.2.0/installer.dmg", path, 12, sha256(path.read_bytes()))
            self.assertEqual(store.client.calls, [(1, b"abcd"), (1, b"abcd"), (2, b"efgh"), (3, b"ijkl")])
            self.assertEqual([part["PartNumber"] for part in store.client.completed], [1, 2, 3])

    def test_large_file_parts_are_ordered_and_completed(self):
        class Missing(Exception):
            def get_status_code(self):
                return 404

        class Client:
            def __init__(self):
                self.parts = {}
                self.completed = None

            def head_object(self, **kwargs):
                raise Missing()

            def create_multipart_upload(self, **kwargs):
                self.metadata = kwargs["Metadata"]
                return {"UploadId": "test-upload"}

            def upload_part(self, **kwargs):
                self.parts[kwargs["PartNumber"]] = kwargs["Body"]
                return {"ETag": str(kwargs["PartNumber"])}

            def complete_multipart_upload(self, **kwargs):
                self.completed = kwargs["MultipartUpload"]["Part"]

        with tempfile.TemporaryDirectory() as temp, patch.dict(
            sys.modules, {"qcloud_cos": types.SimpleNamespace(CosServiceError=Missing, CosClientError=TimeoutError)}
        ), patch("cos_mirror.PART_SIZE", 4):
            path = Path(temp) / "installer.dmg"
            path.write_bytes(b"abcdefghijkl")
            store = CosStore.__new__(CosStore)
            store.bucket = "test-bucket"
            store.client = Client()
            store.put_immutable_file("pathmux/releases/v3.2.0/installer.dmg", path, 12, sha256(path.read_bytes()))
            self.assertEqual(b"".join(store.client.parts[n] for n in (1, 2, 3)), path.read_bytes())
            self.assertEqual([part["PartNumber"] for part in store.client.completed], [1, 2, 3])
            self.assertEqual(store.client.metadata["x-cos-meta-sha256"], sha256(path.read_bytes()))

    def test_failed_part_aborts_incomplete_upload(self):
        class Missing(Exception):
            def get_status_code(self):
                return 404

        class Client:
            aborted = False

            def head_object(self, **kwargs):
                raise Missing()

            def create_multipart_upload(self, **kwargs):
                return {"UploadId": "test-upload"}

            def upload_part(self, **kwargs):
                raise RuntimeError("part failed")

            def abort_multipart_upload(self, **kwargs):
                self.aborted = True

        with tempfile.TemporaryDirectory() as temp, patch.dict(
            sys.modules, {"qcloud_cos": types.SimpleNamespace(CosServiceError=Missing, CosClientError=TimeoutError)}
        ), patch("cos_mirror.PART_SIZE", 4):
            path = Path(temp) / "installer.dmg"
            path.write_bytes(b"abcdefgh")
            store = CosStore.__new__(CosStore)
            store.bucket = "test-bucket"
            store.client = Client()
            with self.assertRaisesRegex(RuntimeError, "part failed"):
                store.put_immutable_file("pathmux/releases/v3.2.0/installer.dmg", path, 8, sha256(path.read_bytes()))
            self.assertTrue(store.client.aborted)


if __name__ == "__main__":
    unittest.main()
