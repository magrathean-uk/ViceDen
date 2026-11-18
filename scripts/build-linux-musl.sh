#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"


TARGET="x86_64-unknown-linux-musl"
rustup target add "$TARGET"
cargo build --release --locked --target "$TARGET"

CARGO_TARGET_DIR="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
echo "Built $CARGO_TARGET_DIR/$TARGET/release/viceden"
