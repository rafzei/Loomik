#!/usr/bin/env python3
"""Collect third-party Rust notices for the native dependency graph."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess


def collect(destination):
    host = next(line.removeprefix("host: ") for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host: "))
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", host], text=True))
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [metadata["resolve"]["root"]]
    included = set()
    while pending:
        package = pending.pop()
        if package in included:
            continue
        included.add(package)
        pending.extend(dependency["pkg"] for dependency in nodes[package]["deps"])
    destination.mkdir(parents=True, exist_ok=True)
    index = ["# Loomik Rust dependency notices", "", f"Native target: `{host}`", "",
             "This includes runtime, build and test dependencies resolved by Cargo.", ""]
    patterns = ("license*", "licence*", "copying*", "copyright*", "notice*", "unlicense*")
    from fnmatch import fnmatch
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in included or package["id"] == metadata["resolve"]["root"]:
            continue
        root = Path(package["manifest_path"]).parent
        folder = destination / f"{package['name']}-{package['version']}"
        folder.mkdir()
        files = [path for path in root.rglob("*") if path.is_file() and
                 any(fnmatch(path.name.lower(), pattern) for pattern in patterns)]
        if package.get("license_file"):
            files.append(root / package["license_file"])
        for file in sorted(set(files)):
            relative = file.relative_to(root)
            target = folder / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(file, target)
        details = {key: package.get(key) for key in ("name", "version", "license", "authors", "repository", "homepage")}
        (folder / "package.json").write_text(json.dumps(details, indent=2) + "\n")
        index.append(f"- {package['name']} {package['version']}: {package.get('license') or 'see included license file'}")
    (destination / "README.md").write_text("\n".join(index) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path)
    collect(parser.parse_args().destination)
