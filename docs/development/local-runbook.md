# Run locally

Use an existing Rust toolchain and a PostgreSQL database dedicated to development. Run commands from the repository root. The application applies migrations on startup, so do not point local experiments at a valuable database.

## Configuration

The loader reads `config/default.toml` and `VICEDEN_` environment overrides. Nested keys use two underscores. `.env.example` is a reference; the Rust loader does not automatically source `.env`.

Set `VICEDEN_DATABASE__URL` to your development connection URL, then select the deterministic classifier for local work:

```sh
export VICEDEN_CLASSIFIER__PROVIDER=stub
cargo run
```

The default address is `http://127.0.0.1:3000`. The stub allows ordinary text and has special trigger strings for tests; it provides no production moderation guarantee. To exercise Anthropic, use the configured provider and supply the key named by `classifier.api_key_env`.

The seeded local founder is `root` with password `root` unless overridden. Keep that configuration on loopback with disposable data. Set `VICEDEN_AUTH__FOUNDER_PASSWORD` to a private value for shared environments. Deployed policy also requires a token pepper and classifier key; see [deployment](debian-deploy-quickstart.md).

## Check the running application

```sh
curl -fsS http://127.0.0.1:3000/api/healthz
```

Sign in through `/login`, load a board, create a test thread and reply, open notifications, and create then revoke a test PAT from `/profile`. Use that PAT for API requests; a session cookie alone does not authenticate the API.

All API endpoints except health require a PAT. Founder-only operations are listed separately in the [API reference](../reference/api-v1-spec.md).

## Troubleshooting

- A database error at boot means the server cannot start. Check the connection URL, service, role permissions, and migration error.
- A port conflict means another process owns the configured bind address. Identify it before stopping anything.
- Configuration changes require a restart. Run from the repository root so the default configuration and static assets resolve.
- For a reset, use a disposable database only. [Migration replay](test-quality-gates.md#postgres-and-migrations) destroys the selected public schema.
