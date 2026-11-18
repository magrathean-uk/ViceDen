# Architecture

ViceDen is a Rust web application with server-rendered Askama templates, an Axum JSON API, and PostgreSQL persistence. Static assets are served locally. The optional Anthropic classifier is an external content-processing service, not telemetry.

## Request and storage flow

`src/main.rs` loads `config/default.toml` and environment overrides, opens the database, runs embedded migrations, syncs boards, seeds bootstrap users, applies boot policy, and starts the router. Startup fails if PostgreSQL is unavailable.

| Area | Source | Responsibility |
| --- | --- | --- |
| Router | `src/app.rs` | Web/API mounting, security headers, body limit, timeout |
| Web | `src/routes/web.rs`, `templates/` | Cookie sessions, forms, founder tools |
| API | `src/routes/api.rs` | PAT authentication and JSON operations |
| Persistence | `src/repository/`, `src/db.rs` | PostgreSQL queries and transactions |
| Moderation | `src/services/moderation_pipeline.rs` | Classification and moderation state |
| Workers | `src/services/moderation_workers.rs` | Pending-post retries and daily sample audit |
| UI assets | `static/assets/` | Local CSS and JavaScript |

`AppState` carries a required database pool, repositories, board configuration, caches, and the moderation pipeline. `ForumStore` and `data/forum_state.json` remain compatibility surfaces; they are not a running-server fallback. See the [storage contract](storage-contract.md).

## Authentication and access

Web sessions use cookies and form CSRF checks. Cookie security depends on `server.secure_cookies`; local defaults do not set Secure. Passwords use Argon2. Session and PAT repositories store token digests, with a pepper required by deployed boot policy.

Only `/api/healthz` is anonymous. Other API routes require a PAT; administrative routes additionally check the founder role. A browser session cookie is not a substitute for a PAT. Tokens have no configurable per-route scopes. See the [API reference](../reference/api-v1-spec.md).

Registration redeems an invite and records age self-attestation. This is not independent age verification. Founder actions can deactivate an invite subtree, remove content, and revoke unused invitations.

## Moderation and data leaving the server

The configured provider is Anthropic by default. The pipeline sends content for classification and records results in PostgreSQL. Calls can finish synchronously or leave content pending for retry. The pending worker limits concurrent classifications with a semaphore; a separate worker samples visible posts for reclassification. The `stub` provider is deterministic test behavior and is unsuitable as a production moderation service.

Markdown is rendered through `src/services/safe_html.rs`. Keep sanitization separate from classification: a classification verdict does not make HTML safe.

## Resource and operational boundaries

Configuration defaults set a 128 KiB body limit, 15-second request timeout, and ten database connections. These are configured controls, not evidence of load capacity. Rate-limit buckets are process-local, and trusted-proxy configuration controls which client-IP headers are accepted.

Local tracing and founder metrics provide diagnostics. Do not add external telemetry. The retention service file currently contains design notes only; the running binary does not start a retention sweeper. An expiry timestamp is not proof of physical deletion.

Release packaging and deployment are described in [Operations](../development/operations.md). Browser acceptance and live-service verification are separate from source or unit-test checks.
