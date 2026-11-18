# JSON API reference

This reference describes the routes implemented in `src/routes/api.rs`. All endpoints live under `/api`; the path itself has no version segment.

## Authentication

- API clients authenticate with a Personal Access Token (PAT).
- Use `Authorization: Bearer <token>` or `x-api-token: <token>`.
- `GET /api/healthz` is the only anonymous endpoint.
- All other API routes require a member or founder PAT. Administrative routes additionally require the founder role.
- Web session cookies do not authenticate these API routes. PATs have no configurable per-route scopes.

## Anonymous endpoint

- `GET /api/healthz`

## Authenticated reads

- `GET /api/boards`
- `GET /api/boards/{slug}/threads`
- `GET /api/threads/{id}/posts`
- `GET /api/search`

## Authenticated member operations

- `POST /api/boards/{slug}/threads`
- `POST /api/boards/{slug}/subscription`
- `POST /api/threads/{id}/posts`
- `POST /api/threads/{id}/save`
- `POST /api/threads/{id}/hide`
- `POST /api/threads/{id}/report`
- `PATCH /api/posts/{id}`
- `POST /api/posts/{id}/vote`
- `POST /api/posts/{id}/report`
- `GET /api/me`
- `GET /api/me/saved`
- `GET /api/me/hidden`
- `POST /api/me/mutes/{id}`
- `DELETE /api/me/mutes/{id}`
- `POST /api/me/tags/{tag}/subscription`
- `DELETE /api/me/tags/{tag}/subscription`
- `PATCH /api/me/settings`
- `GET /api/me/notifications`
- `PATCH /api/me/notifications/{id}`
- `POST /api/me/notifications/read-all`
- `GET /api/me/pats`
- `POST /api/me/pats`
- `DELETE /api/me/pats/{id}`

## Founder operations

- `GET /api/admin/users`
- `POST /api/admin/users/{id}/lift-probation`
- `GET /api/admin/metrics`
- `GET /api/admin/invites`
- `POST /api/admin/invites`
- `GET /api/admin/moderation-events`
- `PATCH /api/admin/threads/{id}/state`
- `POST /api/admin/users/{id}/cauterise`
- `POST /api/admin/users/{id}/wipe`
- `POST /api/admin/users/{id}/delete`
- `GET /api/admin/reports`
- `POST /api/admin/reports/{id}/triage`
- `PATCH /api/admin/reports/{id}`
- `POST /api/admin/moderation/{target}/{id}`

Legacy `/api/users/...` names and unscoped `POST /api/threads` are not part of
the beta contract.

## Pagination

Thread lists, post lists, and search accept `page` and `limit`. Pages start at 1; the default limit is 50 and the maximum is 100. Thread/post list responses are JSON arrays. Search uses its own response object. Visibility and action permissions still depend on the authenticated account and moderation state.

## Core shapes

### `GET /api/boards`

```json
{
  "boards": [
    {
      "slug": "general",
      "name": "General",
      "ttl_days": 180,
      "description": "General discussion and community news."
    }
  ]
}
```

### `GET /api/boards/{slug}/threads`

Returns thread items with id, board slug, title, author, timestamps, score,
reply count, and whether the score is hidden.

Query:

- `sort=hot`
- `sort=new`

### `POST /api/boards/{slug}/threads`

```json
{
  "title": "Thread title",
  "body": "Thread body",
  "tags": ["discussion"]
}
```

`tags` is optional; use tags appropriate for the board.

### `GET /api/threads/{id}/posts`

Returns posts for the thread with post id, thread id, author, body, timestamp,
score, and current viewer vote.

### `POST /api/threads/{id}/posts`

```json
{
  "body": "Reply text"
}
```

### `POST /api/posts/{id}/vote`

```json
{
  "value": 1
}
```

Use `1` for upvote, `-1` for downvote, and repeat the same vote to clear it. Board voting policy and account restrictions still apply.

### `PATCH /api/posts/{id}`

Send `{"body":"Revised text"}`. The handler accepts the author or an account with founder/mod role, checks the body, and records the edit. This endpoint does not create a general moderator administration API.

### `POST /api/me/notifications/read-all`

No body.

### `POST /api/me/pats`

```json
{
  "name": "cli"
}
```

The response includes the raw PAT once. Store it client-side.

### `GET /api/admin/reports`

Founder only. Returns report queue items with thread/post targets, a
`priority_label`, desk `queue_state`, optional `assigned_to`, optional
`target_author`, and `audit_history`.

### `POST /api/admin/reports/{id}/triage`

```json
{
  "queue_state": "reviewing",
  "claim": true,
  "note": "desk owner"
}
```

Founder only. Updates report desk state and optional self-assignment.

### `POST /api/admin/users/{id}/cauterise`

```json
{
  "reason": "invite leak / spam ring"
}
```

Founder only. Returns subtree counts for deactivated users, removed threads/posts,
and revoked unused invites.

### Reports

Thread and post report requests use the same shape:

```json
{
  "reason": "spam",
  "details": "Optional context"
}
```

### `PATCH /api/me/settings`

```json
{
  "theme": "dark",
  "compactness": 1
}
```

### `PATCH /api/admin/reports/{id}`

```json
{
  "resolved": true,
  "note": "reviewed and closed"
}
```

### `POST /api/admin/moderation/{target}/{id}`

`target` must be `post` or `thread`.

```json
{
  "action": "remove",
  "reason": "spam",
  "note": "removed from founder desk"
}
```

## Error Codes

- `400 Bad Request`: invalid JSON, missing fields, or invalid target.
- `401 Unauthorized`: missing or invalid PAT.
- `403 Forbidden`: user is not allowed to perform the action.
- `404 Not Found`: requested board, thread, post, report, or PAT does not exist.
- `429 Too Many Requests`: rate limit exceeded.
- `500 Internal Server Error`: server or persistence error.
- `503 Service Unavailable`: database or service unavailable.

Handler errors commonly use `{"error":"message"}`. Framework extraction errors can have a different response shape. Do not include response bodies containing private data in public bug reports.
