# Operations

This guide describes the checked-in operating tools. Installed services, host addresses, secrets, and current release state must be checked on the authorized deployment; source files alone do not establish live status.

## Release flow

The existing GitHub deploy workflow runs on pushes to `main` and manual dispatch. It builds release artifacts and updates the rolling `deploy-main` release. `scripts/poll_release.py` downloads release metadata and invokes `deploy.sh` when its release identifier changes. Neither script is a general-purpose installer for an unprepared host.

The release directory contains a runtime tarball, checksums, `manifest.json`, and a Debian package when the packaging tool is available. The poller consumes the runtime tarball and checksum file. See the [deployment guide](debian-deploy-quickstart.md).

## Configuration and secrets

Runtime settings come from `config/default.toml` and `VICEDEN_` environment variables. The packaged application unit reads `/etc/default/viceden`. Database backup and deployment-poller units have separate environment files named in their unit definitions.

Use restricted files or the deployment's existing secret manager. Do not put values, personal credential locations, database dumps, or private topology into repository documentation or public reports. A nonempty classifier key passing boot validation does not prove that the upstream service accepts it.

## Routine checks

```sh
sudo systemctl status viceden
sudo journalctl -u viceden -n 200 --no-pager
sudo journalctl -u viceden-deploy-poll.service -n 200 --no-pager
```

Check `/api/healthz` at the authorized application address. In deployed mode it probes PostgreSQL. Founder metrics report backup-marker state and ingress settings; they do not independently prove that a backup can recover all application behavior or that the network enforces those settings.

Review log and backup retention, free disk space, PostgreSQL connectivity, classifier errors, and pending moderation work. Keep the application database separate from other services.

## Maintenance and recovery

- Review direct dependencies periodically and prioritize relevant security fixes. Validate related dependency changes together with the affected tests.
- Exercise backups on an isolated database and record the source revision, backup identifier, restored schema, and application checks.
- Treat binary rollback and database rollback as separate operations. Never edit a shipped migration to force an old binary to start.
- Keep external telemetry out of the application. Use local logs and founder metrics, with sensitive data redacted when shared.

Use the [runbook](runbook.md) for incidents, backup verification, and rollback. Release publication, deployment, and destructive recovery remain deliberate operator actions.
