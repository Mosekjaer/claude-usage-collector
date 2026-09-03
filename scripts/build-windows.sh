#!/usr/bin/env bash
# Local cross-compile for Windows (dev shortcut; releases are built by CI on windows-latest).
# Needs: sudo apt install mingw-w64 && rustup target add x86_64-pc-windows-gnu
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target x86_64-pc-windows-gnu
ls -la target/x86_64-pc-windows-gnu/release/claude-usage-collector.exe
