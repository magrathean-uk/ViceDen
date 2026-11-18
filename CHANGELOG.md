# Changelog

All user-visible changes. Newest first. The format groups commits by
feature area, not by commit-by-commit log; for the full git history use
`git log --oneline`.

## Unreleased — 2026-05-09 hardening loop

A 6-hour autonomous hardening pass focused on perf, reliability,
security, observability, and developer-experience polish. No
user-facing feature changes; the unhinged-Reddit aesthetic is intact.

### Phase 11 strangle (read paths complete)

Every render handler now reads exclusively from Postgres. The
in-memory `ForumStore` is no longer touched on the hot path.

- **A1–A5** — Repository layer gains the methods every handler
  needed: `UserRepo` (handle batch lookup, count, list_paged,
  summary_with_counts, latest_activity, list_summaries_for_admin),
  `ThreadRepo` (count, find_with_preview), `PostRepo` (count,
  participant_count, first_post_for_thread), `VoteRepo`
  (user_vote_for_post + batched scores_for_thread / user_votes_for_thread),
  `PreferencesRepo` (is_thread_saved/hidden, is_board_subscribed),
  new `TagRepo` (list_for_thread, top_tags, set_for_thread,
  tags_for_threads — batched), new `ThreadStateRepo` (state_for,
  states_for_batch, upsert), `NotificationRepo.count_unread_global`.
  Every method ships with `#[ignore]` integration tests.
- **A6.1 view_metrics** — 6 PG count queries.
- **A6.2 view_users** — admin user list via single batched join.
- **A6.3 view_user_detail** — summary + activity + per-board threads.
- **A6.4 view_reports** — open reports + handle batch lookup.
- **A6.5 view_thread** — heaviest. 14 ForumStore calls → 10 repo
  calls; per-post N+1 vote fan-out folded into 2 batch queries.
- **A6.6 view_board** — sort-aware previews via
  `db::list_board_thread_previews` (new / top / hot / active /
  unanswered) + visibility intersection + batched author / state /
  saved-set + tags batch.
- **A6.7 search** — `db::search_forum` (already PG-only) + batched
  handle lookup + per-thread tag cache.
- **A6.8 index** — front-page fan-out per board with the same
  batched preview-render pipeline. Closes the A8 mod_state overlay
  TODO that the legacy front page left open.

### Performance

- View_thread renders eliminate the 2N vote-related round-trips
  (50-post page goes from 100 vote queries to 2).
- view_index / view_board / view_user_detail tag fan-out folded into
  one batched query per render (28-thread index: 28 → 1; 100-thread
  user-detail: 100 → 1).

### Security

- **F1 CSP** — drop `'unsafe-inline'` from style-src; add
  `img-src 'self' data:` and `connect-src 'self'`.
- **F2 HSTS** — preload directive on secure-transport responses.
- **F3 Permissions-Policy** — denies camera, microphone, geolocation,
  payment, USB, FLoC.
- **F8 open-redirect** — verified clean. Every `Redirect::to(...)`
  with user input routes through `services::redirects::safe_return_to`.
- **F10 Argon2** — params explicitly pinned at OWASP 2024 floor
  (m=19456 KiB, t=2, p=1) via custom hasher fn + regression test.
- **F11 cookie flags** — integration-test asserts HttpOnly +
  SameSite=Lax + Path=/ + Max-Age + Secure on the issued session
  cookie.
- **F12 invite race** — N=20 concurrent redemptions on a single-use
  code admit exactly one winner (regression test).
- **F14 PAT scope matrix** — current model documented; v2 sketch.
- **F15 founder-bypass audit** — every admin handler bails to 403
  before privileged work. No regressions.
- **F16 age-attestation immutability** — source-grep test prevents
  any future UPDATE statement on the table; integration test
  proves UNIQUE constraint rejects re-insert.
- **F18 gitleaks** — config + clean baseline scan across all 136
  commits.
- **F19 cargo audit + cargo deny** — sqlx default-features off
  (drops mysql/sqlite + their transitive `rsa 0.9` Marvin-attack
  vulnerability from the build path); deny.toml documents the two
  remaining lockfile-resident-but-unused advisories.

### Observability + ops

- **B1** — `scripts/data-integrity-check.sql`: 17-query read-only
  audit (orphans, missing age attestations, expired sessions,
  duplicate handles, etc.).
- **G1** — `make migrate-test`: replay all migrations on a fresh DB
  + run the lib + integration tests against the resulting schema.
- **H1** — direct dependency major-version review and quarterly bump policy.
- **H2** — `cargo machete` clean (no unused deps).
- **I12** — `RUNBOOK.md` gains a 6-scenario incident-response
  section (classifier outage, PG full, rate-limit bypass, invite
  leak, age-attestation compromise, API-key leak).
- **I15** — `ARCHITECTURE.md`: ASCII topology + per-layer summary
  for the next operator.
- **I16** — `OPERATIONS.md`: deploy / rollback / logs / secrets
  procedures.

### Build + tooling

- `cargo fmt` sweep across the workspace (pre-existing rustfmt
  drift).
- Library tests: 32 → 33 (Argon2 floor regression test).
- All gates green: `cargo fmt --check`, `cargo clippy --all-targets
  -- -D warnings`, `cargo test --lib`.

### Pre-existing failures (not introduced by this loop)

- `register_page_redirects_to_login` and
  `register_post_redirects_to_login`: assertion mismatch on
  `/register` flow predates the loop.
- `shell_templates_link_static_css_without_inline_style_blocks`:
  `data-density` attribute assertion fails on clean main; design
  contract test out of scope.

## 2026-05-08 — M1+M2 milestone (invite beta + concurrency unlock)

The original public-beta build. Brought the forum from
single-tenant prototype to invite-only beta with:

- Invite-only signup (DOB attestation, 18+ check)
- AI moderation (Anthropic Claude Haiku 4.5 + StubClassifier)
- Postgres MVCC (replaces the global `Arc<Mutex<ForumStore>>`)
- Rate limiting (token-bucket with founder bypass)
- Trusted-CIDR client-IP resolver
- Cauterise / hide / save / report flows
- Boot-policy gates (founder-password override, ANTHROPIC_API_KEY)
- Local `tracing` logs and founder-gated internal metrics
