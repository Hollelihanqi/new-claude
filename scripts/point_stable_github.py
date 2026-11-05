"""Temporarily point the domestic stable manifest at GitHub release assets.

Used when the cross-border upload to COS is degraded but the release must ship.
The manifest is only a few KB, so this avoids mirroring the large installers.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

from cos_mirror import CosStore, NO_CACHE, PREFIX


def main():
    tag, path = sys.argv[1], sys.argv[2]
    data = open(path, "rb").read()
    parsed = json.loads(data)
    if parsed.get("version") != tag.lstrip("v"):
        raise SystemExit(f"version mismatch: {parsed.get('version')} != {tag}")
    for target, asset in parsed.get("platforms", {}).items():
        url = asset.get("url", "")
        if "github.com" not in url:
            raise SystemExit(f"non-GitHub URL for {target}: {url}")
    CosStore().put(f"{PREFIX}/stable/latest.json", data, NO_CACHE)
    print(f"stable manifest now points to GitHub for {tag}")


if __name__ == "__main__":
    main()
