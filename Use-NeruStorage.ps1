# Dot-source this file before running Neru build and development commands:
# . D:\Neru\Use-NeruStorage.ps1

$neruRoot = $PSScriptRoot
$neruLocal = Join-Path $neruRoot '.local'
$neruPaths = @{
    Temp = Join-Path $neruLocal 'tmp'
    Cache = Join-Path $neruLocal 'cache'
    Npm = Join-Path $neruLocal 'cache\npm'
    Pnpm = Join-Path $neruLocal 'cache\pnpm'
    Corepack = Join-Path $neruLocal 'cache\corepack'
    Pip = Join-Path $neruLocal 'cache\pip'
    Uv = Join-Path $neruLocal 'cache\uv'
    Cargo = Join-Path $neruLocal 'cache\cargo'
    Rustup = Join-Path $neruLocal 'cache\rustup'
    RustTarget = Join-Path $neruLocal 'build\rust'
    Playwright = Join-Path $neruLocal 'cache\playwright'
    Electron = Join-Path $neruLocal 'cache\electron'
    PythonBytecode = Join-Path $neruLocal 'cache\python-bytecode'
    Gradle = Join-Path $neruLocal 'cache\gradle'
    Data = Join-Path $neruLocal 'data'
}

foreach ($path in $neruPaths.Values) {
    New-Item -ItemType Directory -Force -Path $path | Out-Null
}

$env:TEMP = $neruPaths.Temp
$env:TMP = $neruPaths.Temp
$env:TMPDIR = $neruPaths.Temp
$env:XDG_CACHE_HOME = $neruPaths.Cache
$env:NPM_CONFIG_CACHE = $neruPaths.Npm
$env:COREPACK_HOME = $neruPaths.Corepack
$env:PNPM_HOME = $neruPaths.Pnpm
$env:PNPM_STORE_DIR = $neruPaths.Pnpm
$env:PIP_CACHE_DIR = $neruPaths.Pip
$env:UV_CACHE_DIR = $neruPaths.Uv
$env:CARGO_HOME = $neruPaths.Cargo
$env:RUSTUP_HOME = $neruPaths.Rustup
$env:CARGO_TARGET_DIR = $neruPaths.RustTarget
$env:PLAYWRIGHT_BROWSERS_PATH = $neruPaths.Playwright
$env:ELECTRON_CACHE = $neruPaths.Electron
$env:PYTHONPYCACHEPREFIX = $neruPaths.PythonBytecode
$env:GRADLE_USER_HOME = $neruPaths.Gradle
$env:NERU_DATA_DIR = $neruPaths.Data

Write-Host "Neru project storage is scoped to $neruLocal"
