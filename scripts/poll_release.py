#!/usr/bin/env python3
import json
import os
import subprocess
import urllib.request
from pathlib import Path


GITHUB_REPOSITORY = os.environ.get("GITHUB_REPOSITORY", "magrathean-uk/ViceDen")
RELEASE_TAG = os.environ.get("RELEASE_TAG", "deploy-main")
GITHUB_TOKEN = os.environ.get("GITHUB_TOKEN", "")
STATE_FILE = Path(os.environ.get("DEPLOY_STATE_FILE", "/opt/sites/viceden/runtime/last_release"))
DEPLOY_SCRIPT = os.environ.get("DEPLOY_SCRIPT", "/opt/sites/viceden/deploy.sh")
RUNTIME_ASSET_NAME = os.environ.get("RUNTIME_ASSET_NAME", "viceden-linux-x86_64.tar.gz")
CHECKSUM_ASSET_NAME = os.environ.get("CHECKSUM_ASSET_NAME", "SHA256SUMS")


def github_request(url: str) -> urllib.request.Request:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if GITHUB_TOKEN:
        headers["Authorization"] = f"Bearer {GITHUB_TOKEN}"
    return urllib.request.Request(url, headers=headers)


def fetch_release() -> dict:
    request = github_request(
        f"https://api.github.com/repos/{GITHUB_REPOSITORY}/releases/tags/{RELEASE_TAG}"
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def asset_url(asset: dict) -> str:
    if asset.get("url") and GITHUB_TOKEN:
        return asset["url"]
    return asset["browser_download_url"]


def build_manifest(release: dict) -> dict:
    assets = {asset["name"]: asset for asset in release["assets"]}
    runtime_asset = assets[RUNTIME_ASSET_NAME]
    checksum_asset = assets[CHECKSUM_ASSET_NAME]
    digest = runtime_asset.get("digest", "")
    release_id = ""
    if digest.startswith("sha256:"):
        release_id = digest.split("sha256:", 1)[1][:16]
    if not release_id:
        release_id = (release.get("target_commitish") or str(release["id"]))[:16]
    return {
        "release_id": release_id,
        "runtime_url": asset_url(runtime_asset),
        "runtime_name": runtime_asset["name"],
        "checksum_url": asset_url(checksum_asset),
    }


def read_current_release() -> str:
    if not STATE_FILE.exists():
        return ""
    return STATE_FILE.read_text(encoding="utf-8").strip()


def write_current_release(release_id: str) -> None:
    STATE_FILE.parent.mkdir(parents=True, exist_ok=True)
    STATE_FILE.write_text(f"{release_id}\n", encoding="utf-8")


def main() -> int:
    manifest = build_manifest(fetch_release())
    release_id = manifest["release_id"]

    if read_current_release() == release_id:
        return 0

    subprocess.run(
        [
            DEPLOY_SCRIPT,
            manifest["runtime_url"],
            manifest["runtime_name"],
            manifest["checksum_url"],
            release_id,
        ],
        check=True,
    )

    write_current_release(release_id)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
