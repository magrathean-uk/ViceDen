# User guide

ViceDen is an invite-based text forum. Use the web interface to register, read boards, join discussions, and manage your account. Available actions depend on your account state, board rules, and moderation.

## Start and navigate

Open an invitation link to `/register?code=...`. The form checks invite availability and records age self-attestation. A revoked, expired, or exhausted invite cannot be redeemed. Sign in at `/login` after registration.

The main views are `/` for home, `/b/{slug}` for a board, `/t/{id}` for a thread, `/search`, `/saved`, `/hidden`, `/notifications`, and `/profile`. `/guide`, `/rules`, and `/privacy` provide in-app information.

## Read, post, and search

Boards contain threads and replies. You can subscribe to boards, save or hide threads, vote where board policy permits, and report content. Content can be pending, visible, quarantined, or removed through moderation. The configured classifier may receive submitted text for processing.

Search accepts text and an optional board suffix, such as `privacy : /general`. The board name after the colon must be a configured slug. Search scopes include `all`, `threads`, and `posts`.

Threads have expiry timestamps, but this checkout does not run an automatic retention sweeper. Do not rely on an expiry date as a promise of physical deletion.

## Account and notifications

Profile includes your recent threads, saved threads, board subscriptions, notification preferences, and personal access token controls. The interface is dark-only. The notification bell opens mentions, replies, and moderation-related updates.

A PAT authenticates API requests as your account. Store the raw token when created, keep it private, and revoke it when no longer needed. Founder PATs can access administrative routes. Tokens do not have selectable per-route scopes.

## Founder tools

Founder pages include `/admin/users`, `/admin/reports`, `/admin/invites`, `/admin/moderation-events`, and `/admin/metrics`. Report tools support queue state, assignment, review notes, and moderation history. Cauterising an account affects its invite subtree, including content and unused invitations; inspect the impact before submitting.

## API access

Only health is anonymous:

```sh
curl -fsS http://127.0.0.1:3000/api/healthz
```

All other API routes require a PAT in `Authorization: Bearer ...` or `x-api-token`. A web session cookie is not enough. See the [API reference](../reference/api-v1-spec.md) for member and founder routes and request bodies.

For local setup use the [local runbook](../development/local-runbook.md). For help preparing a report, see [Support](../../.github/SUPPORT.md).
