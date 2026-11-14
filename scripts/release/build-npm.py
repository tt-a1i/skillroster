#!/usr/bin/env python3
"""Stage npm packages from four local, checksum-verified Release archives."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n", encoding="utf-8")


def archive_binary(archive, member):
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as package:
            matches = [info for info in package.infolist() if info.filename == member]
            if len(matches) != 1 or matches[0].is_dir() or stat.S_ISLNK(matches[0].external_attr >> 16):
                raise ValueError(f"missing or non-regular binary: {member}")
            return package.read(matches[0])
    with tarfile.open(archive, "r:gz") as package:
        matches = [info for info in package.getmembers() if info.name == member]
        if len(matches) != 1 or not matches[0].isfile():
            raise ValueError(f"missing or non-regular binary: {member}")
        return package.extractfile(matches[0]).read()


def build(version, artifacts, output):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?", version):
        raise ValueError("version must be SemVer without build metadata")
    candidate = re.search(r'^version = "([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
    if version != candidate and not version.startswith(candidate + "-"):
        raise ValueError(f"version must match source candidate {candidate} or its prerelease")
    if output.exists() or output.is_symlink():
        raise ValueError("refusing to overwrite the output directory")
    platforms = json.loads((ROOT / "npm/platforms.json").read_text())
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="skillroster-npm-", dir=output.parent) as temporary:
        stage = Path(temporary) / "packages"
        stage.mkdir()
        main = stage / "skillroster"
        shutil.copytree(ROOT / "npm/bin", main / "bin")
        (main / "bin/skillroster.cjs").chmod(0o755)
        shutil.copyfile(ROOT / "npm/platforms.json", main / "platforms.json")
        manifest = json.loads((ROOT / "npm/package.json").read_text())
        manifest["version"] = version
        manifest["optionalDependencies"] = {name: version for name in manifest["optionalDependencies"]}
        write_json(main / "package.json", manifest)
        for platform, target in platforms.items():
            name = f"skillroster-{version}-{target}"
            archive_name = name + (".zip" if platform.startswith("win32-") else ".tar.gz")
            archive = artifacts / archive_name
            expected = (artifacts / (archive_name + ".sha256")).read_text().strip()
            match = re.fullmatch(r"([0-9a-fA-F]{64}) [ *]" + re.escape(archive_name), expected)
            with archive.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest() if hasattr(hashlib, "file_digest") else _digest(source)
            if match is None or digest != match[1].lower():
                raise ValueError(f"archive SHA-256 mismatch: {archive_name}")
            binary_name = "skillroster.exe" if platform.startswith("win32-") else "skillroster"
            binary = archive_binary(archive, f"{name}/{binary_name}")
            package = stage / platform
            (package / "bin").mkdir(parents=True)
            destination = package / "bin" / binary_name
            destination.write_bytes(binary)
            destination.chmod(0o755)
            binary_digest = hashlib.sha256(binary).hexdigest()
            (package / "bin" / (binary_name + ".sha256")).write_text(binary_digest + "\n")
            native = json.loads((ROOT / "npm/platforms" / platform / "package.json").read_text())
            native["version"] = version
            write_json(package / "package.json", native)
            write_json(package / "provenance.json", {"target": target, "archive": archive_name, "archive_sha256": digest, "binary_sha256": binary_digest})
        for package in stage.iterdir():
            shutil.copyfile(ROOT / "LICENSE", package / "LICENSE")
            shutil.copyfile(ROOT / "npm/README.md", package / "README.md")
        os.rename(stage, output)


def _digest(source):
    digest = hashlib.sha256()
    for chunk in iter(lambda: source.read(1024 * 1024), b""):
        digest.update(chunk)
    return digest.hexdigest()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        build(args.version, args.artifacts.resolve(), args.output.absolute())
    except (OSError, ValueError, tarfile.TarError, zipfile.BadZipFile) as error:
        parser.exit(1, f"npm staging failed: {error}\n")
    print(f"Staged five verified npm packages at {args.output}")
