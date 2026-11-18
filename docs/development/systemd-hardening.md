# Systemd hardening

The application unit is `packaging/viceden.service`. It runs as the `viceden` user, reads its environment from `/etc/default/viceden`, and uses the release directory as its working directory.

The shipped flags include:

```ini
NoNewPrivileges=true
PrivateTmp=true
ProtectHome=true
ProtectSystem=strict
ProtectControlGroups=true
ProtectKernelModules=true
ProtectKernelTunables=true
RestrictSUIDSGID=true
UMask=0077
```

The unit declares writable data/log paths and requires `viceden-postgres.service`. That service is a compatibility wrapper around the host PostgreSQL service; it does not provision a database or start a container.

Review the actual installation paths, environment-file permissions, database service dependency, backup access, and reverse-proxy logging before enabling units. Test stricter flags against startup, static assets, PostgreSQL, classifier calls, and restart behavior. Journald receives application output through `SyslogIdentifier=viceden`.

These are shipped settings, not evidence that a particular host has installed or enforced them. See [deployment](debian-deploy-quickstart.md).
