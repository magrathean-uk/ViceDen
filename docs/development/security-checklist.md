# Security review checklist

Use this checklist for relevant changes. It is not a completed audit or a certification. Vulnerability reporting belongs in [SECURITY.md](../../.github/SECURITY.md).

## Application and data

- [ ] Password hashing, token digests, and pepper handling preserve existing protections.
- [ ] Cookie-backed writes validate CSRF and production cookies use the intended flags.
- [ ] Member and founder authorization are tested separately for affected routes.
- [ ] Markdown remains sanitized and input limits cover the changed fields.
- [ ] SQL values use bound parameters; any dynamic SQL identifiers are controlled.
- [ ] Registration, invite redemption, and age-attestation changes preserve transaction boundaries.
- [ ] Classifier requests and logs disclose only the data needed for the operation.
- [ ] Token revocation and account deactivation have behavior tests where changed.

## HTTP and operations

- [ ] Security headers remain present on the relevant response paths.
- [ ] Proxy trust matches the actual ingress path and ignores untrusted forwarded headers.
- [ ] Rate-limit behavior is checked for both privileged and ordinary accounts.
- [ ] Deployed configuration satisfies boot policy without weakening it to make startup pass.
- [ ] Backup restore and migration rollback procedures match the installed database arrangement.
- [ ] Dependencies and release contents receive checks appropriate to the change.
- [ ] Secrets and private forum data are absent from issues, fixtures, screenshots, and logs shared for review.

Record what was exercised and what remains unverified. Do not infer an upstream key's validity from the local boot check, which checks configuration rather than contacting the provider.
