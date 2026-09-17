"""Promote verified main-branch Actions artifacts to an immutable GitHub Release.

Preparation uses a read-only token. Only publication needs contents:write.
Downloaded application code is never executed by either step.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib


REPO = "rafzei/Loomik"
PLATFORMS = {
    "macos-arm64": ("macos-15", "macOS-arm64", ("zip", "dmg")),
    "macos-x86_64": ("macos-15-intel", "macOS-x86_64", ("zip", "dmg")),
    "windows-x64": ("windows-2025", "Windows-x64", ("zip",)),
    "ubuntu-amd64": ("ubuntu-24.04", "Ubuntu-24.04-amd64", ("deb",)),
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def gh(*args):
    return subprocess.run(["gh", *args], check=True, text=True, capture_output=True).stdout


def api(endpoint, *, method="GET", data=None, optional=False):
    args = ["gh", "api", "--method", method, f"repos/{REPO}/{endpoint}"]
    if data is not None:
        args += ["--input", "-"]
    result = subprocess.run(args, input=json.dumps(data) if data is not None else None,
                            text=True, capture_output=True)
    if optional and result.returncode and "(HTTP 404)" in result.stderr:
        return None
    if result.returncode:
        raise RuntimeError(result.stderr.strip())
    return json.loads(result.stdout) if result.stdout.strip() else None


def repo_file(path, sha):
    result = api(f"contents/{path}?ref={sha}")
    return base64.b64decode(result["content"]).decode("utf-8")


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def asset_names(version):
    require(re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version), "Expected a stable X.Y.Z version")
    packages, files = set(), set()
    for _, suffix, extensions in PLATFORMS.values():
        stem = f"Loomik-{version}-{suffix}"
        packages.update(f"{stem}.{ext}" for ext in extensions)
        if extensions == ("deb",):
            files.add(f"{stem}.deb.sha256")
        else:
            files.update((f"{stem}.json", f"{stem}.sha256"))
    return packages, files | packages


def validate_run(run_id):
    require(re.fullmatch(r"[1-9][0-9]*", str(run_id)), "Run ID must be a positive integer")
    run = api(f"actions/runs/{run_id}")
    require(run["repository"]["full_name"] == REPO
            and run["head_repository"]["full_name"] == REPO,
            "Build must originate from this repository, never a fork")
    require(run["event"] == "push" and run["head_branch"] == "main",
            "Only push builds from main can be published")
    require(run["path"] == ".github/workflows/portable-builds.yml",
            "Unexpected source workflow")
    require(run["status"] == "completed" and run["conclusion"] == "success",
            "Source build must have completed successfully")
    require(re.fullmatch(r"[0-9a-f]{40}", run["head_sha"]), "Invalid source commit")
    comparison = api(f"compare/{run['head_sha']}...main")
    require(comparison["status"] in ("ahead", "identical"),
            "Source commit must still belong to main")
    jobs = api(f"actions/runs/{run_id}/attempts/{run['run_attempt']}/jobs?per_page=100")
    expected = {f"Build/package {platform} (runner {runner})"
                for platform, (runner, _, _) in PLATFORMS.items()}
    require(jobs["total_count"] == len(expected)
            and len(jobs["jobs"]) == len(expected)
            and {job["name"] for job in jobs["jobs"]} == expected
            and all(job["conclusion"] == "success" for job in jobs["jobs"]),
            "All four native build/package jobs must pass")
    return run


def verify_packages(source, version):
    packages, expected_files = asset_names(version)
    files = {}
    for path in source.rglob("*"):
        require(not path.is_symlink(), f"Symlink in artifacts: {path}")
        if path.is_file():
            require(path.name not in files, f"Duplicate artifact file: {path.name}")
            files[path.name] = path
    require(set(files) == expected_files,
            f"Unexpected artifact contents: {sorted(set(files) ^ expected_files)}")
    verified = {}
    for name, path in files.items():
        if name.endswith(".sha256"):
            for line in path.read_text().splitlines():
                expected, package = line.split(None, 1)
                package = package.removeprefix("*")
                require(package in packages and package not in verified,
                        f"Invalid checksum entry: {package}")
                require(re.fullmatch(r"[0-9a-f]{64}", expected), "Invalid SHA-256")
                require(sha256(files[package]) == expected, f"Checksum mismatch: {package}")
                verified[package] = expected
    require(set(verified) == packages, "Missing package checksums")
    for platform, (_, suffix, extensions) in PLATFORMS.items():
        if extensions == ("deb",):
            continue
        stem = f"Loomik-{version}-{suffix}"
        report = json.loads(files[f"{stem}.json"].read_text())
        require(report["version"] == version and report["platform"] == platform,
                f"Wrong package report: {platform}")
        check = report["package_check"]
        require(check["status"] == "passed", f"Package check failed: {platform}")
        require(len(check["checks"]) == 3
                and {item["format"] for item in check["checks"]} == {"mp4", "mov", "mkv"}
                and all(item["decoded_frames"] == 30 and 0.05 < item["audio_peak"] < 0.3
                        for item in check["checks"]), f"Incomplete media checks: {platform}")
        require(len(report["files"]) == len(extensions)
                and {item["name"] for item in report["files"]}
                == {f"{stem}.{ext}" for ext in extensions}, f"Wrong report files: {platform}")
        for item in report["files"]:
            require(item["sha256"] == verified[item["name"]]
                    and item["bytes"] == files[item["name"]].stat().st_size,
                    f"Package report mismatch: {item['name']}")
    return files, verified


def file_manifest(directory):
    result = {}
    for path in directory.iterdir():
        require(path.is_file() and not path.is_symlink(), "Assets must be regular files")
        result[path.name] = {"size": path.stat().st_size, "digest": f"sha256:{sha256(path)}"}
    return result


def prepare(run_id, directory):
    run = validate_run(run_id)
    sha = run["head_sha"]
    version = tomllib.loads(repo_file("Cargo.toml", sha))["package"]["version"]
    asset_names(version)  # Validate before constructing repository paths.
    notes = repo_file(f"docs/releases/{version}.md", sha)
    artifacts = api(f"actions/runs/{run_id}/artifacts?per_page=100")
    expected = {f"loomik-{platform}-build" for platform in PLATFORMS}
    require(artifacts["total_count"] == len(expected)
            and len(artifacts["artifacts"]) == len(expected)
            and {item["name"] for item in artifacts["artifacts"]} == expected
            and all(not item["expired"] and item["workflow_run"]["head_sha"] == sha
                    for item in artifacts["artifacts"]), "Missing or invalid build artifacts")
    with tempfile.TemporaryDirectory() as downloaded:
        args = ["run", "download", str(run_id), "--repo", REPO, "--dir", downloaded]
        for name in sorted(expected):
            args += ["--name", name]
        gh(*args)
        files, checksums = verify_packages(Path(downloaded), version)
        assets = directory / "assets"
        assets.mkdir(parents=True, exist_ok=False)
        for name, path in files.items():
            shutil.copyfile(path, assets / name)
        (assets / "SHA256SUMS").write_text("".join(
            f"{checksums[name]}  {name}\n" for name in sorted(checksums)))
    metadata = {"version": version, "sha": sha, "run_id": str(run_id),
                "run_attempt": run["run_attempt"], "assets": file_manifest(assets)}
    (directory / "release.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (directory / "notes.md").write_text(
        notes + f"\n\nBuilt from `{sha}` in [verified Actions run {run_id}]"
        f"(https://github.com/{REPO}/actions/runs/{run_id}).\n"
        "The attached packages are the exact tested artifacts; SHA256SUMS covers all six packages.\n")
    print(f"Verified {len(metadata['assets'])} assets for v{version} from {sha}")


def verify_tag(tag, sha, *, optional=False):
    reference = api(f"git/ref/tags/{tag}", optional=optional)
    if reference is None:
        return
    obj = reference["object"]
    for _ in range(10):
        if obj["type"] != "tag":
            break
        obj = api(f"git/tags/{obj['sha']}")["object"]
    require(obj["type"] == "commit" and obj["sha"] == sha,
            "Release tag already points to a different commit")


def verify_uploaded(release, expected, *, allow_missing=False):
    uploaded = {}
    for item in release["assets"]:
        name = item["name"]
        require(name in expected and name not in uploaded, f"Unexpected remote asset: {name}")
        require(item["state"] == "uploaded"
                and item["size"] == expected[name]["size"]
                and item["digest"] == expected[name]["digest"],
                f"Remote asset differs; refusing to overwrite: {name}")
        uploaded[name] = item
    require(allow_missing or set(uploaded) == set(expected), "Release assets are incomplete")
    return set(expected) - set(uploaded)


def publish(directory):
    metadata = json.loads((directory / "release.json").read_text())
    version, sha = metadata["version"], metadata["sha"]
    _, names = asset_names(version)
    assets = directory / "assets"
    manifest = file_manifest(assets)
    require(set(manifest) == names | {"SHA256SUMS"} and manifest == metadata["assets"],
            "Staged assets changed after verification")
    run = validate_run(metadata["run_id"])
    require(run["head_sha"] == sha and run["run_attempt"] == metadata["run_attempt"],
            "Source run changed after verification")
    require(tomllib.loads(repo_file("Cargo.toml", sha))["package"]["version"] == version,
            "Version does not match source commit")
    tag = f"v{version}"
    verify_tag(tag, sha, optional=True)
    release = api(f"releases/tags/{tag}", optional=True)
    if release is None:
        release = api("releases", method="POST", data={
            "tag_name": tag, "target_commitish": sha, "name": f"Loomik {version}",
            "body": (directory / "notes.md").read_text(), "draft": True, "prerelease": False,
        })
    require(not release["prerelease"] and release["tag_name"] == tag,
            "Unexpected release identity")
    if release["draft"]:
        require(release["target_commitish"] == sha, "Draft targets a different commit")
        missing = verify_uploaded(release, manifest, allow_missing=True)
        if missing:
            gh("release", "upload", tag, "--repo", REPO,
               *(str(assets / name) for name in sorted(missing)))
        release = api(f"releases/{release['id']}")
        verify_uploaded(release, manifest)
        verify_tag(tag, sha, optional=True)
        release = api(f"releases/{release['id']}", method="PATCH",
                      data={"draft": False, "make_latest": "true"})
    else:
        # A rerun verifies the existing release without modifying its assets/notes.
        verify_uploaded(release, manifest)
    require(not release["draft"] and release["published_at"], "Release is not public")
    verify_tag(tag, sha)
    verify_uploaded(api(f"releases/{release['id']}"), manifest)
    url = release["html_url"]
    print(f"Published and verified {len(manifest)} assets: {url}")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as summary:
            summary.write(f"[Loomik {version}]({url}): {len(manifest)} public assets verified.\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--run-id", required=True)
    prepare_parser.add_argument("--directory", type=Path, required=True)
    publish_parser = commands.add_parser("publish")
    publish_parser.add_argument("--directory", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "prepare":
        prepare(args.run_id, args.directory)
    else:
        publish(args.directory)


if __name__ == "__main__":
    main()
