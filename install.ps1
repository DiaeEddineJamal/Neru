# Installs the `neru` terminal CLI on Windows.
#
#   irm https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.ps1 | iex
#
# Installs into %LOCALAPPDATA%\Neru\cli and adds that folder to your user PATH.
# Pin a version with $env:NERU_VERSION = '0.4.0' before piping, or run the file with -Version 0.4.0.
# Set $env:NERU_NO_MODIFY_PATH = '1' to leave PATH alone.
# Works in Windows PowerShell 5.1 and PowerShell 7.

param([string]$Version)

function Install-NeruCli {
    param([string]$Version)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    $repo = 'DiaeEddineJamal/Neru'

    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    } catch { }

    function Write-Step([string]$Text) { Write-Host "  $Text" -ForegroundColor DarkGray }

    # The mark, when the console can show it (kept out of the file so 5.1 reads it as plain ASCII).
    $mark = '*'
    try { if ([Console]::OutputEncoding.CodePage -eq 65001) { $mark = [string][char]0x273B } } catch { }

    Write-Host ''
    Write-Host "$mark Neru" -ForegroundColor Green
    Write-Host ''

    if (-not $Version) { $Version = $env:NERU_VERSION }
    if ($Version) { $Version = $Version.TrimStart('v', 'V') }

    # x64 builds only; Windows on ARM runs them under emulation.
    $arch = $env:PROCESSOR_ARCHITECTURE
    if ($env:PROCESSOR_ARCHITEW6432) { $arch = $env:PROCESSOR_ARCHITEW6432 }
    if ($arch -ne 'AMD64' -and $arch -ne 'ARM64') {
        throw "Unsupported architecture: $arch. Neru needs 64-bit Windows."
    }

    $asset = 'neru-cli-x86_64-pc-windows-msvc.zip'
    if ($env:NERU_DOWNLOAD_BASE) {
        # Mirrors and local testing: a folder URL that holds the asset and its .sha256.
        $base = $env:NERU_DOWNLOAD_BASE.TrimEnd('/')
    } elseif ($Version) {
        $base = "https://github.com/$repo/releases/download/v$Version"
    } else {
        $base = "https://github.com/$repo/releases/latest/download"
    }

    $root = Join-Path $env:LOCALAPPDATA 'Neru'
    $installDir = Join-Path $root 'cli'
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("neru-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null

    try {
        # Download and verify
        $label = if ($Version) { "$asset (v$Version)" } else { $asset }
        Write-Step "Downloading $label"
        $zip = Join-Path $tmp $asset
        $sumFile = "$zip.sha256"
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $zip
            Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset.sha256" -OutFile $sumFile
        } catch {
            throw "Could not download $base/$asset. $($_.Exception.Message)"
        }

        $expected = ((Get-Content -Raw $sumFile).Trim() -split '\s+')[0].ToLowerInvariant()
        $actual = (Get-FileHash -Algorithm SHA256 -Path $zip).Hash.ToLowerInvariant()
        if (-not $expected) { throw 'The checksum file is empty.' }
        if ($expected -ne $actual) { throw "Checksum mismatch for $asset (expected $expected, got $actual)." }
        Write-Step 'Checksum verified'

        # Extract, then swap it in
        $staging = Join-Path $tmp 'cli'
        Expand-Archive -Path $zip -DestinationPath $staging -Force
        if (-not (Test-Path (Join-Path $staging 'neru.exe'))) { throw 'The archive does not contain neru.exe.' }

        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
        # A running neru.exe cannot be overwritten, but it can be renamed out of the way.
        $exe = Join-Path $installDir 'neru.exe'
        $oldExe = "$exe.old"
        if (Test-Path $oldExe) { Remove-Item -Force $oldExe -ErrorAction SilentlyContinue }
        if (Test-Path $exe) {
            try { Remove-Item -Force $exe } catch { Move-Item -Force $exe $oldExe }
        }
        $skillsDir = Join-Path $installDir 'skills'
        if (Test-Path $skillsDir) { Remove-Item -Recurse -Force $skillsDir }
        Get-ChildItem -Force $staging | ForEach-Object {
            Move-Item -Force $_.FullName (Join-Path $installDir $_.Name)
        }
        Write-Step "Installed to $installDir"

        # PATH (user scope, plus this session). NERU_NO_MODIFY_PATH=1 leaves it alone.
        if ($env:NERU_NO_MODIFY_PATH -eq '1') {
            Write-Step "Skipped PATH; add $installDir yourself"
        } else {
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        $entries = @()
        if ($userPath) { $entries = $userPath -split ';' | Where-Object { $_ } }
        $onPath = $entries | Where-Object { $_.TrimEnd('\') -ieq $installDir.TrimEnd('\') }
        if (-not $onPath) {
            $newPath = (@($entries) + $installDir) -join ';'
            [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
            Write-Step "Added $installDir to your user PATH"
        }
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $installDir.TrimEnd('\') })) {
            $env:Path = "$env:Path;$installDir"
        }
        }

        Write-Host ''
        $shown = if ($Version) { "neru v$Version" } else { 'neru' }
        Write-Host "$mark " -ForegroundColor Green -NoNewline
        Write-Host "$shown is installed."
        Write-Host 'Run `neru` in any project folder. Other open terminals need a restart to see it.'
        Write-Host ''
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
}

try {
    Install-NeruCli -Version $Version
} catch {
    Write-Host "error: $($_.Exception.Message)" -ForegroundColor Red
    $global:LASTEXITCODE = 1
    # `exit` would close the window under `irm | iex`, so only exit when this file itself was run.
    $self = $MyInvocation.MyCommand.Path
    if ($self -and (Split-Path -Leaf $self) -like '*install*.ps1') { exit 1 }
}
