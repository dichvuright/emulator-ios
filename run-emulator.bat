@echo off
title SweetEmu Rust - iOS 16.7.16 Native Desktop Emulator
cd /d "%~dp0"

if "%~1"=="--python" (
    shift
    python tools/sweet-emu.py %1 %2 %3 %4 %5 %6 %7 %8 %9
    goto :eof
)

if not exist "sweet-rs\target\release\sweet-emu.exe" (
    echo [*] Building SweetEmu Rust Native Engine...
    cargo build --manifest-path "sweet-rs\Cargo.toml" --release
)

"sweet-rs\target\release\sweet-emu.exe" %*
