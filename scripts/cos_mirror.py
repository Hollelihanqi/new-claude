#!/usr/bin/env python3
"""Mirror signed release bytes, then promote the verified manifest last.

Credentials are read only from the CI environment, never written to disk or logs.
The publisher needs only GetObject / HeadObject / PutObject on pathmux/*.
"""

import argparse
import copy
import hashlib
import json
import os
import re
import sys
from pathlib import Path
from urllib.parse import quote, unquote, urlsplit
from urllib.request import Request, urlopen


PREFIX = "pathmux"
NO_CACHE = "no-store, no-cache, must-revalidate, max-age=0"
IMMUTABLE = "public, max-age=31536000, immutable"
TAG_PATTERN = re.compile(r"v(\d+)\.(\d+)\.(\d+)$")


def encode_json(value):
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode()


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def validate_tag(tag):
    if not TAG_PATTERN.fullmatch(tag):
        raise ValueError("Only stable version tags vX.Y.Z may enter this mirror")


def validate_base(base):
    parts = urlsplit(base)
    if parts.scheme != "https" or not parts.hostname or parts.username or parts.password:
        raise ValueError("COS public base URL must use HTTPS without credentials")
    if parts.path not in ("", "/") or parts.query or parts.fragment:
        raise ValueError("COS public base URL must be a bucket origin without a path")
    return base.rstrip("/")


def object_url(base, key):
    return f"{base}/{quote(key, safe='/')}"


def release_plan(manifest, tag, repo, base, asset_names):
    """Change only platform URLs; reject incomplete or foreign release inputs."""
    validate_tag(tag)
    base = validate_base(base)
    if manifest.get("version") != tag[1:]:
        raise ValueError("Release tag and updater manifest version disagree")
    platforms = manifest.get("platforms")
    if not isinstance(platforms, dict) or not platforms:
        raise ValueError("Updater manifest has no platforms")
    for target in ("darwin-aarch64", "darwin-x86_64", "windows-x86_64"):
        if target not in platforms:
            raise ValueError(f"Updater manifest is missing {target}")
    rewritten = copy.deepcopy(manifest)
    required = set()
    for target, platform in platforms.items():
        if not isinstance(platform.get("signature"), str) or not platform["signature"].strip():
            raise ValueError(f"Missing update signature: {target}")
        url = platform.get("url", "")
        parts = urlsplit(url)
        expected = f"/{repo}/releases/download/{tag}/"
        if (parts.scheme != "https" or parts.netloc != "github.com"
                or not parts.path.startswith(expected) or parts.query or parts.fragment):
            raise ValueError(f"Unexpected source release URL for {target}")
        name = unquote(parts.path[len(expected):])
        if name not in asset_names or "/" in name or "\\" in name or name in (".", ".."):
            raise ValueError(f"Missing or invalid referenced asset for {target}")
        required.add(name)
        rewritten["platforms"][target]["url"] = object_url(
            base, f"{PREFIX}/releases/{tag}/{name}"
        )
    return rewritten, required


class CosStore:
    def __init__(self):
        from qcloud_cos import CosConfig, CosS3Client
        self.bucket = os.environ["TENCENT_COS_BUCKET"]
        region = os.environ["TENCENT_COS_REGION"]
        self.base = validate_base(os.environ["TENCENT_COS_PUBLIC_BASE_URL"])
        expected = f"https://{self.bucket}.cos.{region}.myqcloud.com"
        if self.base != expected:
            raise ValueError("Mirror URL does not match the configured COS bucket and region")
        self.client = CosS3Client(CosConfig(
            Region=region, SecretId=os.environ["TENCENT_COS_SECRET_ID"],
            SecretKey=os.environ["TENCENT_COS_SECRET_KEY"], Scheme="https",
        ))

    def read(self, key):
        from qcloud_cos import CosServiceError
        try:
            response = self.client.get_object(Bucket=self.bucket, Key=key)
            with response["Body"].get_raw_stream() as body:
                return body.read()
        except CosServiceError as error:
            if error.get_status_code() == 404 and error.get_error_code() == "NoSuchKey":
                return None
            raise

    def put(self, key, data, cache_control):
        self.client.put_object(
            Bucket=self.bucket, Key=key, Body=data, EnableMD5=True,
            ContentType="application/json; charset=utf-8" if key.endswith(".json")
            else "application/octet-stream",
            CacheControl=cache_control, StorageClass="STANDARD",
            Metadata={"x-cos-meta-sha256": sha256(data)},
        )

    def put_immutable(self, key, data):
        from qcloud_cos import CosServiceError
        try:
            existing = self.client.head_object(Bucket=self.bucket, Key=key)
        except CosServiceError as error:
            if error.get_status_code() != 404:
                raise
            existing = None
        if existing is not None:
            digest = existing.get("x-cos-meta-sha256")
            if digest != sha256(data) or int(existing["Content-Length"]) != len(data):
                raise ValueError(f"Refusing to overwrite different versioned bytes: {key}")
            return
        self.put(key, data, IMMUTABLE)


def public_digest(url):
    """Verify real anonymous downloads including all bytes, not just HEAD status."""
    request = Request(url, headers={"Cache-Control": "no-cache"})
    with urlopen(request, timeout=120) as response:
        digest = hashlib.sha256()
        size = 0
        while chunk := response.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    return {"sha256": digest.hexdigest(), "size": size}


def verify_assets(receipt, verify=public_digest):
    for name, asset in receipt["assets"].items():
        expected = {"sha256": asset["sha256"], "size": asset["size"]}
        if not expected["size"] or verify(asset["url"]) != expected:
            raise ValueError(f"Anonymous download failed verification: {name}")


def stage(store, directory, tag, repo, base, verify=public_digest):
    directory = Path(directory)
    manifest = json.loads((directory / "latest.json").read_bytes())
    files = {p.name: p for p in directory.iterdir() if p.is_file() and p.name != "latest.json"}
    if any(p.is_symlink() for p in files.values()):
        raise ValueError("Release directory must not contain symbolic links")
    names = set(files)
    if not any(n.endswith(".dmg") for n in names) or not any(n.endswith(".msi") for n in names):
        raise ValueError("Release must include both macOS and Windows installers")
    rewritten, _ = release_plan(manifest, tag, repo, base, names)
    receipt = {"tag": tag, "repo": repo, "assets": {}}
    for name, path in sorted(files.items()):
        data = path.read_bytes()
        if not data:
            raise ValueError(f"Empty release asset: {name}")
        key = f"{PREFIX}/releases/{tag}/{name}"
        store.put_immutable(key, data)
        receipt["assets"][name] = {
            "url": object_url(base, key), "size": len(data), "sha256": sha256(data),
        }
        print(f"Uploaded or reused {name}", flush=True)
    # Write no candidate until every public download equals the source bytes.
    verify_assets(receipt, verify)
    candidate = f"{PREFIX}/candidates/{tag}"
    mirrored_bytes = encode_json(rewritten)
    receipt["manifest_sha256"] = sha256(mirrored_bytes)
    store.put_immutable(f"{candidate}/source.json", encode_json(manifest))
    store.put_immutable(f"{candidate}/verification.json", encode_json(receipt))
    store.put_immutable(f"{candidate}/latest.json", mirrored_bytes)
    print(f"Candidate verified: {tag}", flush=True)


def promote(store, tag, repo, base, latest_release, verify=public_digest):
    validate_tag(tag)
    # The caller obtains /releases/latest, not just an arbitrary public release.
    if (latest_release.get("tag_name") != tag or latest_release.get("draft") is not False
            or latest_release.get("prerelease") is not False):
        raise ValueError("Only the current public GitHub stable release may be promoted")
    candidate = f"{PREFIX}/candidates/{tag}"
    manifest_bytes = store.read(f"{candidate}/latest.json")
    receipt = json.loads(store.read(f"{candidate}/verification.json"))
    source = json.loads(store.read(f"{candidate}/source.json"))
    if receipt.get("tag") != tag or receipt.get("repo") != repo:
        raise ValueError("Candidate verification belongs to another release")
    if receipt.get("manifest_sha256") != sha256(manifest_bytes):
        raise ValueError("Candidate manifest digest mismatch")
    rewritten, required = release_plan(source, tag, repo, base, set(receipt["assets"]))
    if encode_json(rewritten) != manifest_bytes:
        raise ValueError("Candidate changed fields beyond platform download URLs")
    # Restrict verification destinations even if a receipt has been tampered with.
    for name, asset in receipt["assets"].items():
        expected = object_url(base, f"{PREFIX}/releases/{tag}/{name}")
        if "/" in name or "\\" in name or asset["url"] != expected:
            raise ValueError("Candidate receipt points outside the version directory")
    if not required.issubset(receipt["assets"]):
        raise ValueError("Candidate does not contain all platform assets")
    previous = store.read(f"{PREFIX}/stable/latest.json")
    if previous:
        previous_version = json.loads(previous)["version"]
        validate_tag("v" + previous_version)
        parts = lambda version: tuple(map(int, version.lstrip("v").split(".")))
        if parts(previous_version) > parts(tag):
            raise ValueError("Refusing to roll the domestic stable channel backward")
    verify_assets(receipt, verify)
    # COS PutObject replaces this one small object atomically. It is always last.
    store.put(f"{PREFIX}/stable/latest.json", manifest_bytes, NO_CACHE)
    if store.read(f"{PREFIX}/stable/latest.json") != manifest_bytes:
        raise ValueError("Stable manifest readback mismatch")
    if verify(object_url(base, f"{PREFIX}/stable/latest.json")) != {
        "size": len(manifest_bytes), "sha256": sha256(manifest_bytes),
    }:
        raise ValueError("Public stable manifest verification failed")
    print(f"Domestic stable channel now serves {tag}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("operation", choices=("stage", "promote"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--assets", type=Path)
    parser.add_argument("--latest-release", type=Path)
    args = parser.parse_args()
    store = CosStore()
    if args.operation == "stage":
        if not args.assets:
            parser.error("stage requires --assets")
        stage(store, args.assets, args.tag, args.repo, store.base)
    else:
        if not args.latest_release:
            parser.error("promote requires --latest-release")
        promote(store, args.tag, args.repo, store.base, json.loads(args.latest_release.read_bytes()))


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        # No credential-bearing HTTP headers or SDK exception dumps in CI logs.
        detail = str(error)[:800]
        for name in ("TENCENT_COS_SECRET_ID", "TENCENT_COS_SECRET_KEY"):
            secret = os.environ.get(name)
            if secret:
                detail = detail.replace(secret, "[redacted]")
        detail = f": {detail}" if detail else ""
        print(f"Mirror failed ({type(error).__name__}){detail}; publication was stopped.", file=sys.stderr)
        sys.exit(1)
