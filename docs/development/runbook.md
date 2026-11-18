# Runbook

Use the [local runbook](local-runbook.md) for development, [quality gates](test-quality-gates.md) for checks, and [deployment guide](debian-deploy-quickstart.md) for release installation.

## Triage

1. Check health at the affected instance's `/api/healthz` and inspect the response status.
2. Read application and PostgreSQL logs for the same time window.
3. Check disk space, database connectivity, recent configuration changes, and the installed release identifier.
4. Contain the affected behavior before changing data or rolling back.
5. Record the timeline and recovery evidence without copying private forum data or credentials into public reports.

For the supplied service names:

```sh
sudo systemctl status viceden
sudo journalctl -u viceden -n 200 --no-pager
sudo journalctl -u viceden-deploy-poll.service -n 200 --no-pager
```

## Classifier failure

Inspect moderation state and classifier errors. Pending-post retries are handled by `src/services/moderation_workers.rs`; retry exhaustion can quarantine posts. The binary has no `reclassify-pending` subcommand. The provider setting is `VICEDEN_CLASSIFIER__PROVIDER`, not `BACKEND`.

Do not switch to the test stub as an undocumented outage workaround. It allows ordinary content and is not equivalent moderation. Restore provider access, verify a controlled submission, and inspect remaining pending/quarantined content through authorized tools.

## Database or disk failure

Stop avoidable write-heavy work when capacity is exhausted. Inspect space usage and PostgreSQL errors before deleting anything. Preserve backups, incident evidence, invite records, and age attestations. Avoid speculative cleanup SQL or a full-table rewrite during a disk incident.

The server needs PostgreSQL even when `database.require_connection` is false. Do not expect a JSON failover. The [storage contract](../architecture/storage-contract.md) explains this boundary.

## Credential, invite, or abuse incident

Revoke affected tokens or invitations through the relevant authorized tools and rotate leaked upstream keys or database credentials in their actual secret store. Existing accounts are not automatically removed merely because an invitation is revoked. Review the invite subtree before a cauterise action.

For rate-limit problems, inspect proxy CIDRs, accepted client-IP headers, the ordinary-user path, and the founder bypass. Do not assume a restart fixes the cause. For suspected data compromise, restrict access and preserve evidence before restoring or overwriting records. Use the [security reporting policy](../../.github/SECURITY.md) for disclosure.

## Backups and restore verification

`scripts/backup-postgres.sh` creates compressed custom-format PostgreSQL dumps and expires backups according to its retention setting. The shipped timer triggers the backup service. Its database and filesystem settings must match the installed host.

`scripts/verify-backup.sh` restores to `VICEDEN_VERIFY_DATABASE__URL` (falling back to `VICEDEN_DATABASE__URL`). It **drops and recreates the target public schema** and requires the database name to contain `test` or `verify`. Set the explicit verification variable to an isolated disposable database before use:

```sh
./scripts/verify-backup.sh "$BACKUP_FILE"
```

Set `BACKUP_FILE` to the backup being checked and run from the repository root because the script reads `scripts/data-integrity-check.sql`. It writes a `.verified` marker next to the backup after restore and SQL checks. That marker is not proof of a complete application recovery exercise. Inspect audit output and exercise the restored app before accepting recovery readiness.

`scripts/restore-postgres.sh` targets a legacy Docker database layout, whereas current service files use host PostgreSQL. Do not use that helper on the host layout. Prepare a host-specific restore plan using the correct database, role, port, backup, and recovery objective, and rehearse it on an isolated database first.

## Rollback

Pause automatic deployment while choosing a recovery release. Confirm the old binary can read the current schema and identify any writes that would be lost by a database restore. Take and preserve a backup before a destructive operation.

Migration `0010_invite_age_classifier.sql` drops legacy invite columns after backfill. Rolling back before that migration requires a compatible database restore. A release symlink change alone is insufficient.

After recovery, verify startup, health, login, a controlled thread/reply, moderation, and PAT revocation. Record what was tested and what remains uncertain before resuming automatic deployment.
