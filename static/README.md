# Static assets

The files under `static/assets/` are browser assets served by the application
at `/assets`. The current set is:

- `viceden-app.css`, `viceden-auth.css`, and `viceden-public.css` for page styles
- `viceden-motion.css` for UI motion styles
- `viceden-ui.js` for the small client-side enhancement layer
- `viceden-favicon.svg` for the browser icon

Keep this directory limited to small, first-party assets required by the web
interface. Do not place user uploads, secrets, generated build output, or
external telemetry here. Preserve the dark-only, local-asset, and no-inline-
style contracts enforced by the design tests. Templates should reference
assets through the application’s `/assets` path.

The asset directory is mounted by the Rust router with a static file service;
asset changes therefore affect the rendered web interface and should be
reviewed with the relevant templates and design checks.
