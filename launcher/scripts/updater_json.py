#!/usr/bin/env python3
"""Writes the updater manifest (latest.json) of a launcher release from the release's own assets,
uploads it, and publishes the release.

tauri-action creates the release as a DRAFT and uploads the installers from each build job; this
runs once after both builds. A draft is invisible to `releases/latest`, so the site's download
buttons and the launchers' updater only ever see a release that is complete. (tauri-action's own
includeUpdaterJson wrote latest.json from inside each job by download / delete / upload of the
shared file; the Linux job of 0.3.1 deleted the Windows job's copy and failed to put the merged
one back, which left the release without a manifest.)

    updater_json.py <tag> [--upload] [--publish]
    e.g. updater_json.py launcher-v0.3.3 --upload --publish

Token: GH_TOKEN or GITHUB_TOKEN, else `gh auth token`. Drafts need it even for reading.
Repo: GITHUB_REPOSITORY or Yak0vkaSup/gridlock-launcher.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone

REPO = os.environ.get("GITHUB_REPOSITORY", "Yak0vkaSup/gridlock-launcher")
API = f"https://api.github.com/repos/{REPO}"
# asset suffix -> updater platform keys (the same keys tauri-action wrote)
KEYS = {
    ".AppImage": ["linux-x86_64", "linux-x86_64-appimage"],
    ".deb": ["linux-x86_64-deb"],
    ".rpm": ["linux-x86_64-rpm"],
    ".msi": ["windows-x86_64", "windows-x86_64-msi"],
    "-setup.exe": ["windows-x86_64-nsis"],
}


def token():
    t = os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN")
    if not t:
        try:
            t = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True, check=True).stdout.strip()
        except (OSError, subprocess.CalledProcessError):
            t = ""
    return t


def call(url, method="GET", data=None, accept="application/vnd.github+json", content_type=None):
    # GitHub's asset service answers 500 "Error saving asset" now and then (it failed a CI upload
    # and a hand test the same evening): a 5xx or a dropped connection gets a few more tries
    for attempt in range(6):
        req = urllib.request.Request(url, method=method, data=data)
        req.add_header("Accept", accept)
        req.add_header("User-Agent", "gridlock-launcher-release")
        if token():
            req.add_header("Authorization", f"Bearer {token()}")
        if content_type:
            req.add_header("Content-Type", content_type)
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                body = r.read()
            return body if accept == "application/octet-stream" else (json.loads(body) if body else None)
        except urllib.error.HTTPError as e:
            if e.code < 500 or attempt == 5:
                raise
            print(f"  {method} {url.split('?')[0].rsplit('/', 1)[-1]}: HTTP {e.code}, retrying")
        except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
            if attempt == 5:
                raise
            print(f"  {method}: {e}, retrying")
        time.sleep(3 * (attempt + 1))


def find_release(tag):
    # /releases/tags/<tag> returns published releases only; the listing has the drafts too
    for page in (1, 2, 3):
        rels = call(f"{API}/releases?per_page=100&page={page}")
        for r in rels:
            if r["tag_name"] == tag:
                return r
        if len(rels) < 100:
            break
    sys.exit(f"no release with tag {tag} (a draft needs a token with access)")


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    tag = sys.argv[1]
    rel = find_release(tag)
    assets = {a["name"]: a for a in rel["assets"]}
    platforms = {}
    for name, a in sorted(assets.items()):
        keys = next((k for suffix, k in KEYS.items() if name.endswith(suffix)), None)
        if not keys:
            continue
        sig = assets.get(name + ".sig")
        if not sig:
            sys.exit(f"{name} has no .sig in the release")
        signature = call(sig["url"], accept="application/octet-stream").decode().strip()
        # the address the file has once the release is published (a draft's own url says "untagged")
        url = f"https://github.com/{REPO}/releases/download/{tag}/{urllib.parse.quote(name)}"
        for k in keys:
            platforms[k] = {"url": url, "signature": signature}
    missing = [k for keys in KEYS.values() for k in keys if k not in platforms]
    if missing:
        sys.exit(f"release {tag} lacks assets for {missing}")
    manifest = {
        "version": tag.removeprefix("launcher-v"),
        "notes": rel.get("body") or "",
        "pub_date": datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z"),
        "platforms": platforms,
    }
    body = json.dumps(manifest, indent=2) + "\n"
    with open("latest.json", "w") as f:
        f.write(body)
    print(f"latest.json: {manifest['version']} with {sorted(platforms)}{' (draft)' if rel['draft'] else ''}")

    if "--upload" in sys.argv:
        # look again right before: a failed earlier try may have left a copy behind
        for a in call(rel["url"])["assets"]:
            if a["name"] == "latest.json":
                call(a["url"], method="DELETE")
        upload = rel["upload_url"].split("{", 1)[0] + "?name=latest.json"
        call(upload, method="POST", data=body.encode(), content_type="application/json")
        print("uploaded latest.json")
    if "--publish" in sys.argv:
        if rel["draft"]:
            call(rel["url"], method="PATCH", data=json.dumps({"draft": False}).encode(), content_type="application/json")
            print(f"published {tag}")
        else:
            print(f"{tag} was already published")


if __name__ == "__main__":
    main()
