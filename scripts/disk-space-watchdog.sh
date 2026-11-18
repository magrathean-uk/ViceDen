#!/usr/bin/env bash
# Local disk guard for ViceDen hosts. Exits non-zero at the critical threshold.

set -euo pipefail

PATH_TO_CHECK="${1:-/}"
WARN_AT="${VICEDEN_DISK_WARN_PERCENT:-80}"
CRIT_AT="${VICEDEN_DISK_CRIT_PERCENT:-95}"

USED_PERCENT="$(df -P "$PATH_TO_CHECK" | awk 'NR == 2 { gsub("%", "", $5); print $5 }')"

if [[ -z "$USED_PERCENT" ]]; then
  echo "could not read disk usage for $PATH_TO_CHECK" >&2
  exit 66
fi

if (( USED_PERCENT >= CRIT_AT )); then
  echo "critical: $PATH_TO_CHECK disk ${USED_PERCENT}% full (threshold ${CRIT_AT}%)" >&2
  exit 2
fi

if (( USED_PERCENT >= WARN_AT )); then
  echo "warning: $PATH_TO_CHECK disk ${USED_PERCENT}% full (threshold ${WARN_AT}%)" >&2
  exit 1
fi

echo "ok: $PATH_TO_CHECK disk ${USED_PERCENT}% full"
