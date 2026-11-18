# ViceDen project context

ViceDen is a server-rendered Rust forum backed by PostgreSQL. Documentation includes `README.md` for the product overview, `docs/architecture/overview.md` for architecture, `docs/development/runbook.md` for operations and `.github/CONTRIBUTING.md` for contribution information.

## Architecture

- PostgreSQL is the live persistence layer. `data/forum_state.json` and `ForumStore` are compatibility surfaces rather than a runtime fallback.
- Shipped migrations record schema history. A rollback before `0010_invite_age_classifier.sql` requires a database restore.
- SQLx repository code uses runtime `query` and `query_as`. Migration replay details and the `DATABASE_URL` / `VICEDEN_DATABASE__URL` relationship are documented in [test and quality gates](docs/development/test-quality-gates.md).
- `tests/design_contract.rs` covers the dark UI, local assets and CSS conventions. Board slugs also appear in `tests/boards_loading.rs`.
- Proxy CIDRs, forwarded-client-IP headers, cookie security, token pepper, request limits, rate limits and moderation configuration affect security behaviour.
- `target/`, `fuzz/target/`, `fuzz/corpus/`, `fuzz/artifacts/`, `.codebase-memory/`, `.cargo/`, IDE folders, `.env` and `sqlx-data.json` contain generated output or local state.

## Command reference

`./scripts/check.sh` runs `cargo check`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test` from the repository root.

`./scripts/migrate-test.sh` is documented for a test-named PostgreSQL database and recreates its schema.
