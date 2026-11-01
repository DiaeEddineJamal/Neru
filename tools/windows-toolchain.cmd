@echo off
setlocal
rem Find the C++ tools in any Visual Studio edition (Community, Build Tools, ...), wherever it is installed.
set "VSINSTALLER=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer"
set "PATH=%VSINSTALLER%;%PATH%"
set "VCVARS="
if exist "%VSINSTALLER%\vswhere.exe" for /f "usebackq delims=" %%i in (`"%VSINSTALLER%\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find VC\Auxiliary\Build\vcvars64.bat`) do set "VCVARS=%%i"
for %%r in ("%ProgramFiles%" "%ProgramFiles(x86)%") do for %%e in (Community Professional Enterprise BuildTools) do if not defined VCVARS if exist "%%~r\Microsoft Visual Studio\18\%%e\VC\Auxiliary\Build\vcvars64.bat" set "VCVARS=%%~r\Microsoft Visual Studio\18\%%e\VC\Auxiliary\Build\vcvars64.bat"
if not defined VCVARS (
  echo Visual Studio C++ build tools were not found. Install "Desktop development with C++" or the VS Build Tools.
  exit /b 1
)
call "%VCVARS%" >nul
if errorlevel 1 exit /b 1
set "SDKROOT=D:\Neru\.local\sdk"
set "SDKVER=10.0.28000.0"
set "LIB=%LIB%;%SDKROOT%\x64\c\um\x64;%SDKROOT%\x64\c\ucrt\x64"
set "INCLUDE=%INCLUDE%;%SDKROOT%\common\c\Include\%SDKVER%\um;%SDKROOT%\common\c\Include\%SDKVER%\shared;%SDKROOT%\common\c\Include\%SDKVER%\ucrt;%SDKROOT%\common\c\Include\%SDKVER%\winrt"
set "PATH=D:\Neru\.local\cache\rustup\toolchains\stable-x86_64-pc-windows-msvc\bin;%SDKROOT%\common\c\bin\%SDKVER%\x64;%PATH%"
set "RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc"
cd /d D:\Neru\app
if "%~1"=="check" goto check
if "%~1"=="check-online" goto check-online
if "%~1"=="test" goto test
if "%~1"=="cli" goto cli
if "%~1"=="fmt" goto fmt
if "%~1"=="build-exe" goto build-exe
if "%~1"=="build" goto build
if "%~1"=="installers" goto installers
npm run desktop
exit /b %errorlevel%
:check
cargo check --manifest-path "D:\Neru\app\src-tauri\Cargo.toml" --offline
exit /b %errorlevel%
:check-online
cargo check --manifest-path "D:\Neru\app\src-tauri\Cargo.toml"
exit /b %errorlevel%
:cli
cargo build --manifest-path "D:\Neru\app\src-tauri\Cargo.toml" --offline --bin neru-cli %2
exit /b %errorlevel%
:test
cargo test --manifest-path "D:\Neru\app\src-tauri\Cargo.toml" --offline
exit /b %errorlevel%
:fmt
cargo fmt --manifest-path "D:\Neru\app\src-tauri\Cargo.toml"
exit /b %errorlevel%
:build-exe
cargo build --manifest-path "D:\Neru\app\src-tauri\Cargo.toml" --release --offline
exit /b %errorlevel%
:build
npm run tauri -- build -b nsis --config src-tauri/tauri.bundle.conf.json
exit /b %errorlevel%
:installers
rem Update files are signed with the key kept in .local\keys, which is never committed.
if exist "D:\Neru\.local\keys\neru-updater.key" set /p TAURI_SIGNING_PRIVATE_KEY=<"D:\Neru\.local\keys\neru-updater.key"
if exist "D:\Neru\.local\keys\neru-updater.password" set /p TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<"D:\Neru\.local\keys\neru-updater.password"
npm run tauri -- build -b nsis,msi --config src-tauri/tauri.bundle.conf.json
exit /b %errorlevel%
