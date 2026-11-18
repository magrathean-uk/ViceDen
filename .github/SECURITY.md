# Security Policy — ViceDen

## Scope

ViceDen is a server-rendered Rust forum with PostgreSQL-backed accounts,
content, moderation records, sessions, personal access tokens, and settings.
The application also sends submitted text to its configured moderation
classifier in production and records classification audit data.

## Security boundaries

- Authentication and authorization must protect founder-only and user-scoped
  routes, sessions, and personal access tokens.
- State-changing authenticated web routes must retain their CSRF protection.
- Deployment must use a non-default founder password, a token pepper, and a
  configured classifier key when the database connection requirement is on.
- Client IP headers are trusted only from configured proxy CIDRs. Rate limits,
  request size limits, and request timeouts are abuse-control boundaries.
- User-supplied rich text and return paths must remain safely handled. The
  shipped security headers and local-asset policy are part of the web boundary.
- Schema changes must preserve the PostgreSQL migration history and data
  integrity.

## Reporting vulnerabilities

The project lists `contact@magrathean.uk` as its security contact. Use the
subject `SECURITY: ViceDen` and include the affected version or commit,
platform, topology, reproduction steps, impact, and redacted evidence.

Do not publish credentials, private keys, database dumps, signing certificates,
or exploit details.

## Scope & Safe Harbour

MAGRATHEAN UK LTD will not pursue a good-faith researcher for security disclosures that:
- Target non-production test systems or researcher-owned environments;
- Avoid persistence, destructive changes, denial of service, and access to personal or customer data;
- Report promptly and permit reasonable time for remediation;
- Do not condition non-disclosure on financial compensation.

## Excluded Conduct

No safe harbour covers phishing, credential stuffing, accessing private production infrastructure, large-scale scanning, denial of service, or unlawful conduct.
