"""Unpack Microsoft Windows SDK NuGet packages into the Neru D drive workspace."""

from pathlib import Path
import zipfile

ROOT = Path(r"D:\Neru\.local\sdk")
for filename, folder in (("sdk-common.nupkg", "common"), ("sdk-x64.nupkg", "x64")):
    destination = ROOT / folder
    destination.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(ROOT / filename) as archive:
        for entry in archive.infolist():
            relative = Path(entry.filename)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"Unsafe SDK path: {entry.filename}")
            target = destination / relative
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
            else:
                target.parent.mkdir(parents=True, exist_ok=True)
                with archive.open(entry) as source, target.open("wb") as output:
                    while chunk := source.read(1024 * 1024):
                        output.write(chunk)
    print(f"Unpacked {filename} to {destination}", flush=True)
