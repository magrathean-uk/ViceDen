#!/usr/bin/env bash
set -euo pipefail

ENV_FILE="${VICEDEN_POSTGRES_ENV_FILE:-/etc/viceden-postgres.env}"
BACKUP_DIR="${VICEDEN_BACKUP_DIR:-/opt/sites/viceden/backups}"
KEEP_DAYS="${VICEDEN_BACKUP_KEEP_DAYS:-7}"
PGHOST="${VICEDEN_POSTGRES_HOST:-127.0.0.1}"
PGPORT="${VICEDEN_POSTGRES_PORT:-5433}"

if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
fi

DB_NAME="${POSTGRES_DB:-viceden}"
DB_USER="${POSTGRES_USER:-viceden}"
DB_PASSWORD="${VICEDEN_POSTGRES_PASSWORD:-${POSTGRES_PASSWORD:-}}"
: "${DB_PASSWORD:?missing VICEDEN_POSTGRES_PASSWORD or POSTGRES_PASSWORD}"

mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
OUTPUT="$BACKUP_DIR/viceden-$STAMP.dump.gz"

PGPASSWORD="$DB_PASSWORD" pg_dump \
    -h "$PGHOST" \
    -p "$PGPORT" \
    -U "$DB_USER" \
    -d "$DB_NAME" \
    -Fc | gzip -9 > "$OUTPUT"

chmod 600 "$OUTPUT"
find "$BACKUP_DIR" -type f -name 'viceden-*.dump.gz' -mtime +"$KEEP_DAYS" -delete

echo "wrote $OUTPUT"
