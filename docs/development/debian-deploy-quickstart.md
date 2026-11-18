# Debian deployment guide

The repository supplies release packaging, a polling deployer, and systemd examples for a Linux x86_64 installation. This guide describes those files; it does not assert that any particular host is configured or healthy.

## Build and artifact flow

The existing `.github/workflows/deploy.yml` runs `./scripts/release.sh` on main pushes or manual dispatch and publishes a rolling prerelease named `deploy-main`.

The release script runs the local gate, builds the binary, packages runtime files, and invokes `scripts/smoke-release.sh`. It rejects a dirty checkout by default. Output includes `viceden-linux-x86_64.tar.gz`, `SHA256SUMS`, and `manifest.json`; `viceden_amd64.deb` is included when Debian packaging is available. These are script-defined outputs, not a claim that a release has passed.

The poller reads GitHub release metadata, selects the runtime tarball and checksum asset, invokes `deploy.sh`, and records the completed release identifier. A repository token is needed when its configured assets require authentication. Keep that token out of logs and repository files.

## Prepare an installation

1. Provision a dedicated ViceDen database and role on the intended PostgreSQL service. The compatibility `viceden-postgres.service` requires the host PostgreSQL 18 service; it does not create a database or run a container.
2. Review the paths and service account in `packaging/viceden.service`, the units under `ops/`, and `deploy.sh`. Their defaults must match the installation before activation.
3. Install a reviewed runtime artifact and make its configuration, assets, executable, and current-release path readable by the application user.
4. Prepare restricted environment files using `packaging/viceden.env.example` and the examples under `ops/`. Replace example values through the authorized secret-management process.
5. Configure the reverse proxy for the chosen origin. Keep the application and database private; do not open public ports as a shortcut. Review trusted-proxy CIDRs and forwarded headers against the actual ingress chain.
6. Verify startup and application behavior before enabling the deployment and backup timers.

Host addresses, accounts, and private credential locations belong in the installation's private operating record.

## Runtime settings

| Variable | Purpose |
| --- | --- |
| `VICEDEN_DATABASE__URL` | Dedicated application PostgreSQL connection |
| `VICEDEN_DATABASE__REQUIRE_CONNECTION` | Enable deployed boot checks and health probing; set to `true` for deployment |
| `VICEDEN_AUTH__FOUNDER_PASSWORD` | Non-default founder password |
| `VICEDEN_AUTH__TOKEN_PEPPER` | Secret satisfying the deployed token-pepper validator |
| `ANTHROPIC_API_KEY` | Default classifier key variable; configurable through `classifier.api_key_env` |
| `VICEDEN_SERVER__BIND` | Private application listener |
| `VICEDEN_SERVER__SECURE_COOKIES` | Set to `true` for HTTPS ingress |
| `VICEDEN_SERVER__PROXY__TRUSTED_CIDRS` | Trusted proxy peers |
| `VICEDEN_SERVER__PROXY__CLIENT_IP_HEADERS` | Headers accepted from those peers |
| `VICEDEN_BOARDS_FILE` | Board configuration path |
| `VICEDEN_OPS__BACKUP_DIR` | Directory inspected by founder backup metrics |

The classifier boot check checks that its configured key variable is nonempty. Verify actual provider access separately. The runtime loader reads the default TOML plus environment; it does not automatically load `.env`.

## Verification and recovery

Check service logs and `/api/healthz`, then exercise login, a controlled post and reply, report review, and PAT creation/revocation. `scripts/smoke.sh` creates test activity and requires an authorized target plus private founder credentials.

The release tarball includes this guide as `DEPLOY.md`. Repository-only helpers, including backup verification, must be available separately if the release script does not package them. Inspect actual contents before assuming a source path exists in an installed release.

Use the repository runbook for database restore and migration-aware rollback. The legacy `scripts/restore-postgres.sh` expects Docker and must not be treated as a host-PostgreSQL restore recipe. Coordinate automatic polling with recovery so it cannot immediately replace a rollback.
