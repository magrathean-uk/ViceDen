#!/usr/bin/env bash
# Verify a ViceDen pg_dump custom/plain backup by restoring it into a throwaway DB.

set -euo pipefail

BACKUP="${1:-}"
TARGET_URL="${VICEDEN_VERIFY_DATABASE__URL:-${VICEDEN_DATABASE__URL:-}}"

if [[ -z "$BACKUP" || ! -f "$BACKUP" ]]; then
  echo "usage: VICEDEN_VERIFY_DATABASE__URL=postgres://.../viceden_verify_test $0 /path/to/backup.dump[.gz]" >&2
  exit 64
fi

if [[ -z "$TARGET_URL" ]]; then
  echo "set VICEDEN_VERIFY_DATABASE__URL to a throwaway Postgres database URL" >&2
  exit 64
fi

DB_NAME="${TARGET_URL##*/}"
DB_NAME="${DB_NAME%%\?*}"
if [[ ! "$DB_NAME" =~ test|verify ]]; then
  echo "refusing to restore into database without 'test' or 'verify' in its name: $DB_NAME" >&2
  exit 65
fi

psql "$TARGET_URL" -v ON_ERROR_STOP=1 <<'SQL'
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
SQL

case "$BACKUP" in
  *.gz)
    if gunzip -c "$BACKUP" | head -c 5 | grep -q '^PGDMP'; then
      gunzip -c "$BACKUP" | pg_restore --clean --if-exists --no-owner --no-privileges -d "$TARGET_URL"
    else
      gunzip -c "$BACKUP" | psql "$TARGET_URL" -v ON_ERROR_STOP=1
    fi
    ;;
  *)
    if head -c 5 "$BACKUP" | grep -q '^PGDMP'; then
      pg_restore --clean --if-exists --no-owner --no-privileges -d "$TARGET_URL" "$BACKUP"
    else
      psql "$TARGET_URL" -v ON_ERROR_STOP=1 -f "$BACKUP"
    fi
    ;;
esac

psql "$TARGET_URL" -v ON_ERROR_STOP=1 -f scripts/data-integrity-check.sql >/dev/null
printf '%s\n' "$(date -u +%FT%TZ)" > "${BACKUP}.verified"
echo "backup restore verified in $DB_NAME"
