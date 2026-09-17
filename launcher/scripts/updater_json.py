#!/usr/bin/env python3
"""Writes the updater manifest (latest.json) of a launcher release from the release's own assets.

tauri-action's includeUpdaterJson builds latest.json inside each build job by download / delete /
upload of the shared file; the Linux job of 0.3.1 deleted the Windows job's copy and then failed
to upload the merged one, which left the release with no manifest at all. This runs once, after
both builds, from what is actually in the release, and uploads with --clobber.

    updater_json.py <tag> [--upload]      e.g. updater_json.py launcher-v0.3.1 --upload

Needs `gh` (logged in, or GH_TOKEN) for the upload; reading the release is anonymous on a public
repo but uses GH_TOKEN when set. Repo: GITHUB_REPOSITORY or Yak0vkaSup/gridlock-launcher.
"""
import json
import os
import subprocess
import sys
import urllib.request
from datetime import datetime, timezone

REPO = os.environ.get("GITHUB_REPOSITORY", "Yak0vkaSup/gridlock-launcher")
# asset suffix -> updater platform keys (the same keys tauri-action writes)
KEYS = {
    ".AppImage": ["linux-x86_64", "linux-x86_64-appimage"],
    ".deb": ["linux-x86_64-deb"],
    ".rpm": ["linux-x86_64-rpm"],
    ".msi": ["windows-x86_64", "windows-x86_64-msi"],
    "-setup.exe": ["windows-x86_64-nsis"],
}


def get(url, binary=False):
    req = urllib.request.Request(url, headers={"User-Agent": "gridlock-launcher-release"})
    tok = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if tok and "api.github.com" in url:
        req.add_header("Authorization", f"Bearer {tok}")
    with urllib.request.urlopen(req, timeout=60) as r:
        data = r.read()
    return data if binary else data.decode()


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    tag = sys.argv[1]
    upload = "--upload" in sys.argv
    rel = json.loads(get(f"https://api.github.com/repos/{REPO}/releases/tags/{tag}"))
    assets = {a["name"]: a for a in rel["assets"]}
    platforms = {}
    for name, a in sorted(assets.items()):
        keys = next((k for suffix, k in KEYS.items() if name.endswith(suffix)), None)
        if not keys:
            continue
        sig = assets.get(name + ".sig")
        if not sig:
            sys.exit(f"{name} has no .sig in the release")
        signature = get(sig["browser_download_url"]).strip()
        for k in keys:
            platforms[k] = {"url": a["browser_download_url"], "signature": signature}
    missing = [k for keys in KEYS.values() for k in keys if k not in platforms]
    if missing:
        sys.exit(f"release {tag} lacks assets for {missing}")
    manifest = {
        "version": tag.removeprefix("launcher-v"),
        "notes": rel.get("body") or "",
        "pub_date": datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
        "platforms": platforms,
    }
    with open("latest.json", "w") as f:
        json.dump(manifest, f, indent=2)
        f.write("\n")
    print(f"latest.json: {manifest['version']} with {sorted(platforms)}")
    if upload:
        subprocess.run(["gh", "release", "upload", tag, "latest.json", "--clobber", "-R", REPO], check=True)
        print("uploaded")


if __name__ == "__main__":
    main()
