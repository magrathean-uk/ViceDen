#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "usage: $0 <viceden-backup.dump.gz>" >&2
    exit 2
fi

BACKUP_FILE="$1"
ENV_FILE="${VICEDEN_POSTGRES_ENV_FILE:-/etc/viceden-postgres.env}"
CONTAINER="${VICEDEN_POSTGRES_CONTAINER:-viceden-postgres}"
DB_NAME="${VICEDEN_POSTGRES_DB:-viceden}"
DB_USER="${VICEDEN_POSTGRES_USER:-viceden}"
CONFIRM="${CONFIRM_RESTORE:-}"

if [[ ! -f "$BACKUP_FILE" ]]; then
    echo "backup not found: $BACKUP_FILE" >&2
    exit 2
fi

if [[ -f "$ENV_FILE" ]]; then
    set -a
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    set +a
fi

: "${VICEDEN_POSTGRES_PASSWORD:?missing VICEDEN_POSTGRES_PASSWORD}"

if [[ "$CONFIRM" != "viceden" ]]; then
    echo "set CONFIRM_RESTORE=viceden to restore into $DB_NAME" >&2
    exit 2
fi

command -v docker >/dev/null 2>&1 || {
    echo "docker is required" >&2
    exit 2
}
command -v gzip >/dev/null 2>&1 || {
    echo "gzip is required" >&2
    exit 2
}

echo "warning: destructive restore; this replaces the target database"

cat "$BACKUP_FILE" | gzip -dc | docker exec -i \
    -e PGPASSWORD="$VICEDEN_POSTGRES_PASSWORD" \
    "$CONTAINER" \
    pg_restore \
        -h 127.0.0.1 \
        -p 15432 \
        -U "$DB_USER" \
        -d "$DB_NAME" \
        --clean \
        --if-exists \
        --no-owner \
        --no-acl

echo "restored $BACKUP_FILE into $DB_NAME"
