# Contributing to ViceDen

## Before you change code

Keep each change focused on the responsible service, route, template, or
configuration. Preserve the product name `ViceDen`, existing migration history,
and unrelated work in the checkout. Do not commit local build artefacts,
secrets, or generated files.

PostgreSQL is the live persistence layer. Use a throwaway database whose name
contains `test` for migration checks. Never edit a shipped migration. Add a new
migration instead. For a replay, follow
[test and quality gates](../docs/development/test-quality-gates.md) and
align `DATABASE_URL` with the disposable `VICEDEN_DATABASE__URL` when SQLx CLI
is available.

## Validation

Run the narrowest useful check from the repository root:

```sh
./scripts/check.sh
cargo test --test health_route
./scripts/migrate-test.sh
```

`./scripts/check.sh` is the full local gate. It runs Cargo checking, formatting,
Clippy, and tests. The migration check recreates the target schema, so it is
only safe for a disposable test database. For a UI change, keep the design
contract passing. For a release or deployment change, use the relevant evidence
described in `docs/development/runbook.md`.

## Dependencies and external material

Keep dependency additions deliberate and compatible with `deny.toml`. Do not
add third-party code, assets, or generated material without recording its
provenance and reviewing its licence obligations. The repository does not yet
state public contribution licensing or assignment terms; obtain an owner
decision before accepting an external contribution that requires either.

## Clean development

ViceDen follows [Clean Development](https://github.com/magrathean-uk/clean-development)
and is configured for it in `.clean-development.json`. The mandatory rules are
in the "Clean development" section of [AGENTS.md](../AGENTS.md): run builds,
tests and installs through
`clean-development run --session session-only -- <command>`, keep the routed
environment variables intact, and do not add installers or scripts that write
tool state under `~`.
