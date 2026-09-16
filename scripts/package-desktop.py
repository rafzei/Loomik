#!/usr/bin/env python3
"""Create and verify standalone macOS/Windows distribution archives."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def sha256(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def verify_binary(path, arch):
    """Fail packaging when a build depends on the developer's libraries."""
    if sys.platform == "darwin":
        actual = subprocess.check_output(["lipo", "-archs", str(path)], text=True).strip()
        if actual != arch:
            raise RuntimeError(f"Wrong architecture for {path.name}: {actual}, expected {arch}")
        lines = subprocess.check_output(["otool", "-L", str(path)], text=True).splitlines()[1:]
        for line in lines:
            library = line.strip().split(" (", 1)[0]
            if library == "@rpath/libswift_Concurrency.dylib":
                load = subprocess.check_output(["otool", "-l", str(path)], text=True)
                if "path /usr/lib/swift " in load:
                    continue
            if not library.startswith(("/usr/lib/", "/System/Library/")):
                raise RuntimeError(f"Non-system dependency in {path.name}: {library}")
    else:
        # dumpbin is supplied by the MSVC developer environment in the CI job.
        text = subprocess.check_output(["dumpbin", "/dependents", str(path)], text=True)
        system = {"kernel32.dll", "user32.dll", "advapi32.dll", "ole32.dll", "oleaut32.dll",
                  "shell32.dll", "ws2_32.dll", "bcrypt.dll", "ntdll.dll", "gdi32.dll",
                  "winmm.dll", "secur32.dll", "crypt32.dll", "msvcrt.dll", "ucrtbase.dll",
                  "imm32.dll", "comdlg32.dll", "shlwapi.dll", "version.dll", "setupapi.dll",
                  "dwmapi.dll", "dxgi.dll", "d3d11.dll", "d3d12.dll", "d3dcompiler_47.dll",
                  "opengl32.dll", "propsys.dll", "cfgmgr32.dll", "powrprof.dll", "avrt.dll",
                  "runtimeobject.dll", "combase.dll", "windowsapp.dll", "winspool.drv",
                  "mf.dll", "mfplat.dll", "mfreadwrite.dll", "mfuuid.dll", "strmiids.dll"}
        for line in text.splitlines():
            library = line.strip().lower()
            if library.endswith((".dll", ".drv")) and not (
                library in system or library.startswith(("api-ms-win-", "ext-ms-win-"))
            ):
                raise RuntimeError(f"Unbundled/unknown Windows dependency in {path.name}: {library}")


def verify_install(executable, output):
    env = os.environ.copy()
    env.pop("LOOMIK_FFMPEG", None)
    env.pop("LOOMIK_FFPROBE", None)
    if sys.platform == "darwin":
        env["PATH"] = "/usr/bin:/bin"
    else:
        windows = Path(env["SystemRoot"])
        env["PATH"] = os.pathsep.join([str(windows / "System32"), str(windows)])
    run(executable, "--verify-package", output, env=env, cwd=output.parent)
    result = json.loads((output / "package-check.json").read_text())
    if result["status"] != "passed":
        raise RuntimeError("Packaged application verification failed")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--media-tools", type=Path)
    parser.add_argument("--no-dmg", action="store_true", help="Create only ZIP on macOS")
    args = parser.parse_args()
    os.chdir(ROOT)
    mac = sys.platform == "darwin"
    if not mac and sys.platform != "win32":
        parser.error("Use the native macOS/Windows host. Ubuntu uses bundle-linux.sh.")
    arch = platform.machine() if mac else "x64"
    if mac and arch not in ("arm64", "x86_64"):
        parser.error(f"Unsupported architecture: {arch}")
    key = f"macos-{arch}" if mac else "windows-x64"
    media = (args.media_tools or ROOT / "target/media-tools" / key).resolve()
    suffix = "" if mac else ".exe"
    for name in ("ffmpeg", "ffprobe"):
        tool = media / "bin" / (name + suffix)
        if not tool.is_file():
            parser.error(f"Missing {tool}; first run bash scripts/build-media-tools.sh")
        verify_binary(tool, arch)
    for item in ("sources", "licenses", "NOTICE.txt", "build-info.txt"):
        if not (media / item).exists():
            parser.error(f"Missing media distribution material: {item}")
    if not args.skip_build:
        command = ["cargo", "build", "--release", "--locked"]
        if not mac:
            command += ["--target", "x86_64-pc-windows-msvc"]
        run(*command)
    executable = ROOT / ("target/release/loomik" if mac else "target/x86_64-pc-windows-msvc/release/loomik.exe")
    verify_binary(executable, arch)
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    destination = ROOT / "target/release/bundle"
    destination.mkdir(parents=True, exist_ok=True)
    name = f"Loomik-{version}-{'macOS-' + arch if mac else 'Windows-x64'}"
    # Fresh staging prevents old DLLs or old tools leaking into a release.
    with tempfile.TemporaryDirectory(prefix="package-", dir=destination) as temp:
        stage = Path(temp)
        app = stage / ("Loomik.app" if mac else "Loomik")
        binary_dir = app / "Contents/MacOS" if mac else app
        resources = app / "Contents/Resources" if mac else app
        binary_dir.mkdir(parents=True)
        resources.mkdir(parents=True, exist_ok=True)
        packaged_exe = binary_dir / ("loomik" if mac else "Loomik.exe")
        shutil.copy2(executable, packaged_exe)
        for tool in ("ffmpeg", "ffprobe"):
            shutil.copy2(media / "bin" / (tool + suffix), binary_dir / (tool + suffix))
        media_docs = resources / "media-tools"
        shutil.copytree(media, media_docs, ignore=shutil.ignore_patterns("bin"))
        shutil.copy2(ROOT / "LICENSE", resources / "LICENSE.txt")
        run(sys.executable, ROOT / "scripts/rust-notices.py", resources / "rust-notices")
        shutil.copy2(ROOT / "docs/media-tools.md", media_docs / "BUILDING.md")
        shutil.copy2(ROOT / "packaging" / ("INSTALL-macOS.txt" if mac else "INSTALL-Windows.txt"), resources / "INSTALL.txt")
        if mac:
            info = plistlib.loads((ROOT / "packaging/Info.plist").read_bytes())
            info["CFBundleShortVersionString"] = version
            info["CFBundleVersion"] = version
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
            shutil.copy2(ROOT / "packaging/AppIcon.icns", resources / "AppIcon.icns")
            for tool in ("ffmpeg", "ffprobe"):
                run("codesign", "--force", "--sign", "-", binary_dir / tool)
            run("codesign", "--force", "--sign", "-", "--identifier", "com.local-loom.recorder",
                "--requirements", '=designated => identifier "com.local-loom.recorder"', app)
            run("codesign", "--verify", "--deep", "--strict", app)
        archive = destination / (name + ".zip")
        if mac:
            run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, archive)
        else:
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zip:
                for file in sorted(app.rglob("*")):
                    if file.is_file():
                        zip.write(file, file.relative_to(stage))
        # Check the extracted archive, including paths containing spaces. Verify
        # that Finder/Explorer downloads retain all files and executable modes.
        extracted = stage / "Install check with spaces"
        extracted.mkdir()
        if mac:
            run("ditto", "-x", "-k", archive, extracted)
            installed = extracted / "Loomik.app/Contents/MacOS/loomik"
            run("codesign", "--verify", "--deep", "--strict", extracted / "Loomik.app")
        else:
            with zipfile.ZipFile(archive) as zip:
                zip.extractall(extracted)
            installed = extracted / "Loomik/Loomik.exe"
        evidence = verify_install(installed, extracted / "verification")
        artifacts = [archive]
        if mac and not args.no_dmg:
            image_root = stage / "disk-image"
            image_root.mkdir()
            shutil.copytree(app, image_root / "Loomik.app", symlinks=True)
            (image_root / "Applications").symlink_to("/Applications")
            shutil.copy2(ROOT / "packaging/INSTALL-macOS.txt", image_root / "INSTALL.txt")
            dmg = destination / (name + ".dmg")
            run("hdiutil", "create", "-volname", "Loomik", "-srcfolder", image_root,
                "-format", "UDZO", "-ov", dmg)
            run("hdiutil", "verify", dmg)
            artifacts.append(dmg)
        report = {"version": version, "platform": key, "signing": "ad-hoc" if mac else "unsigned",
                  "notarized": False, "package_check": evidence,
                  "files": [{"name": p.name, "sha256": sha256(p), "bytes": p.stat().st_size} for p in artifacts]}
        (destination / (name + ".json")).write_text(json.dumps(report, indent=2) + "\n")
        (destination / (name + ".sha256")).write_text("".join(f"{sha256(p)}  {p.name}\n" for p in artifacts))
        for artifact in artifacts:
            print(artifact)


if __name__ == "__main__":
    main()
