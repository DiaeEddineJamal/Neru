# winget

Two packages, both built from the GitHub release:

| Identifier | What it installs | Installer |
| --- | --- | --- |
| `Luziv.Neru` | The desktop app, which also puts `neru` on PATH | `Neru_<ver>_x64-setup.exe` (NSIS, per user) |
| `Luziv.Neru.CLI` | Only the `neru` terminal CLI | `neru-cli-x86_64-pc-windows-msvc.zip` (portable `neru.exe`, alias `neru`) |

`manifests/` mirrors the layout of [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs) (`manifests/l/Luziv/...`). The `InstallerSha256` values are placeholders; fill them in from the release's `.sha256` files before submitting.

Install one or the other, not both: each provides a `neru` command.

## First submission (manual, once)

winget-pkgs only accepts automated updates for packages that already exist there, so 0.4.0 goes in by hand.

1. Publish the release, and get the hashes:

   ```powershell
   $v = '0.4.0'
   $base = "https://github.com/DiaeEddineJamal/Neru/releases/download/v$v"
   (Invoke-WebRequest "$base/neru-cli-x86_64-pc-windows-msvc.zip.sha256" -UseBasicParsing).Content
   # The NSIS setup has no .sha256 sibling; hash it locally:
   Invoke-WebRequest "$base/Neru_${v}_x64-setup.exe" -OutFile setup.exe -UseBasicParsing
   (Get-FileHash setup.exe -Algorithm SHA256).Hash
   ```

2. Put the hashes in the two `*.installer.yaml` files, then check them locally:

   ```powershell
   winget validate --manifest packaging\winget\manifests\l\Luziv\Neru\0.4.0
   winget validate --manifest packaging\winget\manifests\l\Luziv\Neru\CLI\0.4.0
   # Optional, in Windows Sandbox or a VM (needs `winget settings --enable LocalManifestFiles`):
   winget install --manifest packaging\winget\manifests\l\Luziv\Neru\CLI\0.4.0
   ```

3. Submit with [wingetcreate](https://github.com/microsoft/winget-create), using a GitHub token with `public_repo` scope:

   ```powershell
   winget install Microsoft.WingetCreate
   wingetcreate submit --token <github-token> packaging\winget\manifests\l\Luziv\Neru\0.4.0
   wingetcreate submit --token <github-token> packaging\winget\manifests\l\Luziv\Neru\CLI\0.4.0
   ```

   This opens one pull request per package on microsoft/winget-pkgs. Sign the Microsoft CLA on the first PR and answer any validation comments. Once both are merged, `winget install Luziv.Neru.CLI` works.

   (`wingetcreate new <installer-url>` builds the same manifests interactively, if you'd rather start from scratch.)

## Later versions (automated)

The `winget` job in `.github/workflows/release.yml` runs after each release is published and opens the update PRs for both packages with [winget-releaser](https://github.com/vedantmgoyal9/winget-releaser). It needs:

- A repository secret `WINGET_TOKEN`: a classic GitHub token with `public_repo` scope, for an account that has a fork of `microsoft/winget-pkgs` (the action pushes the branch there).

Without the secret the job is skipped. The manifests in this folder are only for the first submission; winget-pkgs is the source of truth after that.
