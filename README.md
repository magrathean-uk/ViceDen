<h1 align="center">ViceDen</h1>

<p align="center">An invite-based Rust web forum with moderation, backed by PostgreSQL.</p>

<p align="center">
  <a href="docs/index.md">Documentation</a>
</p>

> **Status:** an unmaintained snapshot of a personal project, published as-is
> without warranty. Sample boards and seed data are neutral placeholders.

## Overview

ViceDen is a Rust web forum backed by PostgreSQL. It provides server-rendered
HTML pages and a JSON API for boards, threads, replies, votes, mentions,
notifications, profiles, search, saved and hidden threads, subscriptions, and
moderation workflows.

The application is invite-based. Content is processed by a synchronous and
asynchronous moderation pipeline, with PostgreSQL storing users, sessions,
tokens, forum content, reports, moderation events, and preferences. The web
interface is dark-only and serves local CSS, JavaScript, and SVG assets.

## Requirements

- Rust and Cargo
- PostgreSQL reachable through `VICEDEN_DATABASE__URL`
- An Anthropic API key for the default classifier configuration
- Python 3.10+ and the pinned client dependency in `cli/requirements.txt` for `cli/viceden.py`

The application runs embedded SQLx migrations during startup. PostgreSQL is
the live persistence layer. The default configuration is in
[`config/default.toml`](config/default.toml), and board definitions are in
[`config/boards.toml`](config/boards.toml). Override configuration with the
corresponding `VICEDEN_` environment variables. Keep secrets out of the
repository.

## Local development

From the repository root, set a database URL for a disposable local database
and select the local test classifier before starting:

```sh
export VICEDEN_DATABASE__URL=postgres://user:password@127.0.0.1:5432/viceden
export VICEDEN_CLASSIFIER__PROVIDER=stub
cargo run
```

The stub is for development and does not provide production moderation. To use
the default Anthropic provider, supply the configured API key instead. See the
[local runbook](docs/development/local-runbook.md) for configuration and local login.

The default bind address is `127.0.0.1:3000`. The public health endpoint is
`GET /api/healthz`. Authentication, API, board, and moderation routes are
documented in [the user guide](docs/guides/user-guide.md).

Install the client CLI dependency into a virtual environment inside the
checkout (`.venv/` is ignored by Git), not into a system or user Python, and
run the script with that interpreter:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -r cli/requirements.txt
.venv/bin/python cli/viceden.py boards
```

For a development check, run:

```sh
./scripts/check.sh
```

This runs Cargo check, formatting verification, Clippy with warnings denied,
and the test suite. The migration replay script operates on a database whose
name contains `test` and drops its public schema, so use only a throwaway
database:

```sh
VICEDEN_DATABASE__URL=postgres://user:password@127.0.0.1:5432/viceden_test \
DATABASE_URL=postgres://user:password@127.0.0.1:5432/viceden_test \
  ./scripts/migrate-test.sh
```

Local operation details are in [the runbook](docs/development/runbook.md) and
[the local runbook](docs/development/local-runbook.md). Deployment packaging
is described in [the Debian deploy quickstart](docs/development/debian-deploy-quickstart.md).

## Repository map

- `src/` contains startup, routing, repositories, services, and domain code.
- `templates/` contains Askama server-rendered pages.
- `static/assets/` contains browser assets served at `/assets`.
- `migrations/` contains the embedded PostgreSQL schema history.
- `scripts/` contains checks, migration replay, release, build, and smoke helpers.
- `docs/` contains architecture, API, user, deployment, and operational notes.

Do not edit a shipped migration. Add a new migration for a schema change. Keep
the PostgreSQL-only persistence model, local-asset UI contract, and
no external telemetry boundary intact. Project-specific development rules are in
[`AGENTS.md`](AGENTS.md).

## Licence

ViceDen is released under the Apache License 2.0; see [`LICENSE`](LICENSE).
Copyright © 2026 MAGRATHEAN UK LTD. Third-party notices are in
[docs/legal/third-party-notices.md](docs/legal/third-party-notices.md), and trademark
notices are in [docs/legal/trademarks.md](docs/legal/trademarks.md).

<sub>© 2026 MAGRATHEAN UK LTD</sub>
