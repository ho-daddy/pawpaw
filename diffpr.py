#!/usr/bin/env python3
"""Download PR before/after full source code for review."""

import sys
import os
import shutil
import json
import tarfile
import io
import urllib.request


def fetch_json(url):
    req = urllib.request.Request(url, headers={"Accept": "application/vnd.github.v3+json"})
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read())


def download_and_extract(tarball_url, dest_dir):
    """Download tarball and extract contents into dest_dir."""
    req = urllib.request.Request(tarball_url, headers={"Accept": "application/vnd.github.v3+json"})
    with urllib.request.urlopen(req) as resp:
        data = resp.read()

    os.makedirs(dest_dir, exist_ok=True)

    with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as tar:
        # tarball has a top-level directory like "owner-repo-sha/"
        # strip it so files go directly into dest_dir
        prefix = None
        for member in tar.getmembers():
            if prefix is None:
                prefix = member.name.split("/")[0]
            # Strip the top-level directory
            if member.name.startswith(prefix + "/"):
                member.name = member.name[len(prefix) + 1:]
            else:
                member.name = member.name[len(prefix):]
            if member.name == "" or member.name.startswith("/") or ".." in member.name.split("/"):
                continue
            tar.extract(member, dest_dir)


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <pr_number> <output_dir>")
        sys.exit(1)

    pr_number = sys.argv[1]
    output_dir = sys.argv[2]

    owner = "kstost"
    repo = "cokacdir"

    # Fetch PR info
    pr_url = f"https://api.github.com/repos/{owner}/{repo}/pulls/{pr_number}"
    print(f"Fetching PR #{pr_number} info...")
    pr_info = fetch_json(pr_url)

    base_sha = pr_info["base"]["sha"]
    head_sha = pr_info["head"]["sha"]
    print(f"Base SHA: {base_sha[:10]}  Head SHA: {head_sha[:10]}")

    before_dir = os.path.join(output_dir, "before")
    after_dir = os.path.join(output_dir, "after")

    # Clean existing directories
    for d in [before_dir, after_dir]:
        if os.path.exists(d):
            shutil.rmtree(d)

    # Download and extract
    tarball_base = f"https://api.github.com/repos/{owner}/{repo}/tarball/{base_sha}"
    tarball_head = f"https://api.github.com/repos/{owner}/{repo}/tarball/{head_sha}"

    print("Downloading before (base)...")
    download_and_extract(tarball_base, before_dir)

    print("Downloading after (head)...")
    download_and_extract(tarball_head, after_dir)

    print(f"Done. Files saved to {before_dir}/ and {after_dir}/")


if __name__ == "__main__":
    main()
