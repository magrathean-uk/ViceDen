#!/usr/bin/env bash
# G1: migration replay regression test.
#
# Drops + recreates the Postgres schema, applies every viceden migration
# in order, then runs the integration tests against the resulting state.
# A clean run is the single-truth proof that the migration set is
# backward-compatible and shape-stable.
#
# Requires:
#   - VICEDEN_DATABASE__URL: postgres URL the caller is willing to wipe.
#     The script refuses if the URL doesn't contain "test" or "_test"
#     in the database name — anti-foot-gun against running against prod.
#   - psql, cargo, sqlx-cli (or `cargo run --bin viceden -- migrate`).
#
# Usage:
#   export VICEDEN_DATABASE__URL=postgres://viceden:viceden@127.0.0.1:5432/viceden_test
#   bash scripts/migrate-test.sh

set -euo pipefail

URL="${VICEDEN_DATABASE__URL:-}"
if [[ -z "$URL" ]]; then
  echo "VICEDEN_DATABASE__URL must be set (point at a throwaway Postgres)." >&2
  exit 64
fi

# Anti-foot-gun: refuse to wipe a DB that doesn't look like a test DB.
DB_NAME="${URL##*/}"
DB_NAME="${DB_NAME%%\?*}"
if [[ ! "$DB_NAME" =~ test ]]; then
  echo "Refusing to wipe a database that doesn't have 'test' in its name." >&2
  echo "Got DB name: $DB_NAME" >&2
  echo "Set VICEDEN_DATABASE__URL to a *_test database and retry." >&2
  exit 65
fi

echo "[migrate-test] Wiping schema in $DB_NAME ..."
psql "$URL" -v ON_ERROR_STOP=1 <<'SQL'
DROP SCHEMA public CASCADE;
CREATE SCHEMA public;
SQL

echo "[migrate-test] Replaying migrations from ./migrations ..."
# Use sqlx-cli if available, else replay the plain SQL migrations with psql.
if command -v sqlx >/dev/null 2>&1; then
  sqlx migrate run --source ./migrations
else
  echo "[migrate-test] sqlx-cli not found; replaying migrations with psql."
  for migration in ./migrations/*.sql; do
    echo "[migrate-test] applying ${migration}"
    psql "$URL" -v ON_ERROR_STOP=1 -f "$migration" >/dev/null
  done
fi

echo "[migrate-test] Running lib tests against fresh schema ..."
cargo test --lib

echo "[migrate-test] Running #[ignore] integration tests against fresh schema ..."
cargo test -- --ignored --test-threads=1

echo "[migrate-test] All migrations applied + tests green. ✓"
