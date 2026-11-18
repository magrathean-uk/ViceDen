# Storage contract

PostgreSQL is required for every normal server startup. `src/main.rs` calls `db::connect_pool`, which connects and applies embedded SQLx migrations before serving requests.

## Runtime state

Durable forum data is read and written through PostgreSQL repositories. Board definitions come from `config/boards.toml` and are synchronized at startup. `ForumStore`, JSON snapshots, and older runtime-state helpers remain for compatibility; new runtime features must not depend on a JSON fallback.

`database.require_connection` controls deployed boot checks and health probing. Setting it to false does not make the database optional. The health response still uses the label `optional_postgres` in that configuration; treat it as a legacy label, not a storage guarantee.

## Changes and recovery

Add a migration for schema changes. Never edit a migration already shipped. Keep related writes in a transaction and return errors when persistence fails. Do not infer board configuration from saved snapshots.

Migration `0010_invite_age_classifier.sql` removes the old invite redemption columns after backfill. Reverting to code that needs those columns requires a compatible database restore. A binary rollback alone cannot undo a schema change.

See [migration guidance](../../migrations/README.md) and the [runbook](../development/runbook.md) for validation and recovery.

## Retention and assets

Threads carry expiry timestamps. `src/services/retention.rs` is currently a design stub, and startup does not schedule a retention worker. Do not promise automatic deletion based on that file or legacy `ForumStore` helpers.

`static/` contains deployable application assets only. Do not place uploads, credentials, backups, or user data there.
