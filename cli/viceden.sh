#!/bin/bash
# Simple wrapper for the VICEDEN Python CLI.
#
# This script detects whether Python is available and invokes
# `cli/viceden.py` with the provided arguments.  It is provided for
# convenience on systems where `python3` is installed but the
# shebang line is not resolved automatically (e.g. some Docker
# containers).  Users can instead call `python3 cli/viceden.py` directly.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if command -v python3 >/dev/null 2>&1; then
    exec python3 "$SCRIPT_DIR/viceden.py" "$@"
else
    echo "Python 3 is required to run this script." >&2
    exit 1
fi