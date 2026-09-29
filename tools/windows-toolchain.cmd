@echo off
setlocal
call "C:\Program Files\Microsoft Visual Studio\18\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
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
npm run tauri -- build -b nsis
exit /b %errorlevel%
:installers
rem Update files are signed with the key kept in .local\keys, which is never committed.
if exist "D:\Neru\.local\keys\neru-updater.key" set /p TAURI_SIGNING_PRIVATE_KEY=<"D:\Neru\.local\keys\neru-updater.key"
if exist "D:\Neru\.local\keys\neru-updater.password" set /p TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<"D:\Neru\.local\keys\neru-updater.password"
npm run tauri -- build -b nsis,msi
exit /b %errorlevel%
