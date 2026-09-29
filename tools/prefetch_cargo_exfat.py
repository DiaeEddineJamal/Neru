"""Verified Cargo crate extraction for Windows exFAT, which rejects old tar mtimes.

Reads Cargo.lock, downloads each registry crate into the project-local Cargo cache,
verifies its SHA-256 checksum, and extracts regular files without restoring tar mtimes.
"""

from __future__ import annotations

import concurrent.futures
import hashlib
import io
import json
import os
from pathlib import Path
import tarfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
LOCK = ROOT / "app" / "src-tauri" / "Cargo.lock"
CARGO = ROOT / ".local" / "cache" / "cargo" / "registry"
REGISTRY = "index.crates.io-1949cf8c6b5b557f"
CACHE = CARGO / "cache" / REGISTRY
SOURCES = CARGO / "src" / REGISTRY


def process(package: dict[str, str]) -> tuple[str, str | None]:
    name, version, checksum = package["name"], package["version"], package["checksum"]
    label = f"{name}-{version}"
    archive = CACHE / f"{label}.crate"
    destination = SOURCES / label
    marker = destination / ".cargo-ok"
    if marker.exists() and marker.read_text(encoding="utf-8") == '{"v":1}':
        return label, None

    CACHE.mkdir(parents=True, exist_ok=True)
    if not archive.exists() or hashlib.sha256(archive.read_bytes()).hexdigest() != checksum:
        url = f"https://static.crates.io/crates/{name}/{label}.crate"
        request = urllib.request.Request(url, headers={"User-Agent": "Neru-Cargo-exFAT-prefetch/1"})
        with urllib.request.urlopen(request, timeout=60) as response:
            contents = response.read()
        if hashlib.sha256(contents).hexdigest() != checksum:
            raise ValueError(f"Checksum mismatch: {label}")
        temporary = archive.with_suffix(".download")
        temporary.write_bytes(contents)
        os.replace(temporary, archive)

    destination.mkdir(parents=True, exist_ok=True)
    prefix = label + "/"
    with tarfile.open(archive, "r:gz") as stream:
        for member in stream:
            if not member.name.startswith(prefix):
                raise ValueError(f"Unexpected archive member in {label}: {member.name}")
            relative = Path(member.name[len(prefix):])
            if not relative.parts or ".." in relative.parts or relative.is_absolute():
                raise ValueError(f"Unsafe archive member in {label}: {member.name}")
            target = destination / relative
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
            elif member.isfile():
                target.parent.mkdir(parents=True, exist_ok=True)
                source = stream.extractfile(member)
                if source is None:
                    raise ValueError(f"Unreadable archive member: {member.name}")
                with target.open("wb") as output:
                    while chunk := source.read(1024 * 1024):
                        output.write(chunk)
            else:
                raise ValueError(f"Unsupported archive member in {label}: {member.name}")
    marker.write_text('{"v":1}', encoding="utf-8")
    return label, "extracted"


def main() -> None:
    lock = tomllib.loads(LOCK.read_text(encoding="utf-8"))
    packages = [p for p in lock["package"] if p.get("source", "").startswith("registry+")]
    print(f"Prefetching {len(packages)} crates to {CARGO}", flush=True)
    failures: list[str] = []
    completed = 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futures = {pool.submit(process, package): package for package in packages}
        for future in concurrent.futures.as_completed(futures):
            package = futures[future]
            try:
                future.result()
            except Exception as error:
                failures.append(f"{package['name']}-{package['version']}: {error}")
            completed += 1
            if completed % 25 == 0 or completed == len(packages):
                print(f"{completed}/{len(packages)} crates, {len(failures)} failures", flush=True)
    if failures:
        print("\n".join(failures), flush=True)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
