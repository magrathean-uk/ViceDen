# Database migrations

This directory contains the SQLx migrations embedded into the ViceDen binary.
The current source tree contains migrations `0001` through `0017`; SQLx applies
them in filename order and records applied versions in PostgreSQL.

Never edit a migration that has shipped or been applied. Add the next numbered
`.sql` file for a schema change and update the relevant application code and
operational notes together. Migration `0010_invite_age_classifier.sql` changes
invite redemption storage destructively, so rolling back to a release before
it requires a database restore.

## Replay a disposable database

The supported repository check is:

```sh
VICEDEN_DATABASE__URL=postgres://user:password@127.0.0.1:5432/viceden_test \
DATABASE_URL=postgres://user:password@127.0.0.1:5432/viceden_test \
  ./scripts/migrate-test.sh
```

The script refuses database names without `test`, drops and recreates the
target `public` schema, replays every file, then runs the library tests and
ignored integration tests. Use a throwaway PostgreSQL database only. It uses
`sqlx migrate run` when `sqlx` is available and otherwise replays the SQL files
with `psql`. Set both variables when using the command above because the
application and the SQLx CLI read different variable names.

For ordinary application startup, migrations are embedded and run by the
server against the configured PostgreSQL database. See [the runbook](../docs/development/runbook.md)
and [`../AGENTS.md`](../AGENTS.md) for repository-wide migration rules.
