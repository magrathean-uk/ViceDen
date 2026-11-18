# Product decisions and implementation boundaries

This file records project direction. Earlier versions mixed planned features with shipped behavior. Use the current source and linked references for what the application actually supports.

## Existing boundaries

- Text discussion with configured boards and invite-based registration.
- Username/password authentication, cookie-backed web sessions, and PAT API access.
- Dark-only UI, local assets, and no external telemetry.
- PostgreSQL as the live persistence layer.
- Founder-controlled administrative operations and classifier-assisted moderation.
- Dedicated application database and role. Do not reuse another service's database.
- Hosted ingress stays behind the configured reverse proxy. Do not open public application or database ports as a deployment shortcut.

Changes to product policy or these operating boundaries need an explicit owner decision.

## Plans that are not current guarantees

Passkeys, payment processing, a fixed subscription price, automatic trial entitlements, proof-of-work signup, and a complete administrative CLI are not implemented commitments established by this checkout. Do not present them as available features or commercial terms.

The API permits post editing, so the former claim that users can never edit posts is inaccurate. The running binary does not start a retention sweeper, so a configured thread lifetime is not a deletion guarantee. Invitation capacity and posting rules must be read from their current handlers and configuration rather than from the original launch plan.

## References

- [User guide](../guides/user-guide.md)
- [API reference](../reference/api-v1-spec.md)
- [CLI status](../reference/admin-cli-spec.md)
- [Storage contract](../architecture/storage-contract.md)
- [Licensing](../legal/third-party-notices.md)
