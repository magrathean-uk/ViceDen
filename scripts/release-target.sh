#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <cargo-target-dir>" >&2
    exit 2
fi

CARGO_TARGET_DIR="$1"
RELEASE_CARGO_TARGET="${VICEDEN_RELEASE_TARGET:-}"
HOST_OS="${VICEDEN_RELEASE_HOST_OS:-$(uname -s)}"
HOST_ARCH="${VICEDEN_RELEASE_HOST_ARCH:-$(uname -m)}"
HOST_IS_LINUX_X86_64=false
if [[ "$HOST_OS" == "Linux" && ( "$HOST_ARCH" == "x86_64" || "$HOST_ARCH" == "amd64" ) ]]; then
    HOST_IS_LINUX_X86_64=true
fi

if [[ -n "$RELEASE_CARGO_TARGET" ]]; then
    if [[ "$HOST_IS_LINUX_X86_64" != "true" ]]; then
        echo "release packaging requires a Linux x86_64 builder so the artifact can be boot-smoked" >&2
        echo "use an x86_64 Linux VM or CI runner for scripts/release.sh" >&2
        exit 2
    fi
    case "$RELEASE_CARGO_TARGET" in
        x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl)
            RELEASE_DIR="$CARGO_TARGET_DIR/$RELEASE_CARGO_TARGET/release"
            RELEASE_PLATFORM="linux-x86_64"
            ;;
        *)
            echo "unsupported VICEDEN_RELEASE_TARGET=$RELEASE_CARGO_TARGET" >&2
            echo "expected x86_64-unknown-linux-gnu or x86_64-unknown-linux-musl" >&2
            exit 2
            ;;
    esac
elif [[ "$HOST_IS_LINUX_X86_64" == "true" ]]; then
    RELEASE_DIR="$CARGO_TARGET_DIR/release"
    RELEASE_PLATFORM="linux-x86_64"
else
    echo "release packaging requires Linux x86_64" >&2
    echo "run scripts/release.sh on an x86_64 Linux VM or CI runner" >&2
    exit 2
fi

printf 'RELEASE_CARGO_TARGET=%q\n' "$RELEASE_CARGO_TARGET"
printf 'RELEASE_DIR=%q\n' "$RELEASE_DIR"
printf 'RELEASE_PLATFORM=%q\n' "$RELEASE_PLATFORM"
