#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

BUILD_ENV="${VICEDEN_BUILD_ENV:-$ROOT_DIR/../build-env.sh}"
if [[ -f "$BUILD_ENV" ]]; then
    # shellcheck disable=SC1090
    . "$BUILD_ENV"
fi

cargo check
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
