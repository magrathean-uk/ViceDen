# Memory and resource safety

Resource controls need explicit bounds even in Rust. Review new long-lived state for size, expiry, concurrency, and shutdown behavior.

## Current controls

`config/default.toml` sets a 128 KiB request body limit, 15-second request timeout, and ten database connections. `src/routes/pagination.rs` defaults page size to 50 and caps it at 100. The moderation retry worker queries batches of up to 50 pending posts and uses a configurable concurrency semaphore.

These controls do not prove a global memory bound. Some handlers collect rows before paging, and process-local rate-limit maps need separate growth review. Worker handles live for the process lifetime; do not claim they are drained through explicit cancellation.

## Change rules

- Give caches and per-client registries both capacity and expiry limits.
- Prefer bounded queues and deliberate rejection or backpressure.
- Page in the database where possible instead of loading entire tables.
- Tie new background work to a defined lifecycle and cap concurrency.
- Keep timeouts on external calls and avoid retaining full model conversations.
- Never log passwords, tokens, invite codes, full post bodies, or age-attestation records.
- Keep backup retention and log rotation in the operator's storage plan.

## Review and measurement

Identify each new allocation, its largest expected size, and how it is released. Measure process memory, database connections, request latency, and moderation backlog under the relevant load. Record the workload and revision; do not infer capacity from configuration limits.

`src/services/retention.rs` is a design stub. A retention plan still needs implementation and verification before it can be described as an operating worker.
