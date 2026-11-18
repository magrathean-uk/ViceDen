# Test and quality gates

Choose checks that exercise the changed behavior. Commands below are defined by the repository; their presence is not evidence that they passed for a particular revision.

## Local gate

```sh
./scripts/check.sh
```

The script runs `cargo check`, `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, then `cargo test`. It optionally sources `../build-env.sh`; `VICEDEN_BUILD_ENV` overrides that location. The helper is not required. Make targets have their own `BUILD_ENV` override.

Focused examples:

```sh
cargo test --test health_route
cargo test --test boards_loading
cargo test --test design_contract
cargo test --test api_docs_contract
```

## Postgres and migrations

Default tests omit tests marked `#[ignore]`. Database-backed tests need a dedicated PostgreSQL database. CI uses PostgreSQL 18 and runs ignored tests serially.

Migration replay **drops and recreates the public schema**. Select a disposable database whose name contains `test`. Set `VICEDEN_DATABASE__URL` to that database and align SQLx's variable before running:

```sh
export DATABASE_URL="$VICEDEN_DATABASE__URL"
./scripts/migrate-test.sh
```

The script uses `psql` to reset the schema, then either SQLx CLI or `psql` to apply migrations. If SQLx is installed it reads `DATABASE_URL`; mismatched variables can target different databases. Replay is followed by lib tests and ignored tests with `--test-threads=1`.

## Release and smoke checks

```sh
./scripts/release.sh
./scripts/build-linux-musl.sh
```

These are separate workflows. Release packaging requires Linux x86_64 because it boots the package. The musl helper cross-compiles a binary but requires a developer-specific absolute environment helper. It is not portable as shipped and does not establish Linux package acceptance. See [cross-compilation](cross-compile-strategy.md).

`./scripts/smoke-release.sh` takes an existing tarball path. `./scripts/smoke.sh` uses `BASE_URL` and `FOUNDER_PASSWORD` against a running instance and creates test activity. Run it only against an instance authorized for that exercise.

## Existing GitHub workflows

| Workflow | Defined behavior |
| --- | --- |
| `check.yml` | PR/manual execution of `scripts/check.sh` |
| `ci.yml` | Format, lint, tests, and ignored PostgreSQL tests on main pushes and PRs |
| `cargo-deny.yml` | License, ban, and source checks; its title does not mean an advisory scan runs |
| `deploy.yml` | Main push/manual release build and publication to rolling `deploy-main` |

No live CI result is implied by this table. Workflow changes require separate review.

## Acceptance

Record the command, revision, result, and any blocker. UI changes need [browser checks](responsive-smoke.md) in addition to source contracts. Schema changes need migration and recovery evidence. Dependency or secret-handling changes may justify installed tools such as `cargo deny check` or `gitleaks detect --config .gitleaks.toml`; do not claim those tools ran when only their configuration was inspected.
