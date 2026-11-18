# Third-party notices

The third-party packages ViceDen depends on, and their licences.

ViceDen is released under the Apache License 2.0. See [LICENSE](../../LICENSE).
Copyright © 2026 MAGRATHEAN UK LTD.

## Rust dependencies

### Rust — `Cargo.toml`

Direct dependencies of the server, with the version locked in `Cargo.lock` and the licence
stated in each crate's own manifest.

| Package | Locked version | Licence | Declared in |
|---------|----------------|---------|-------------|
| `axum` | 0.8.9 | MIT | `Cargo.toml` |
| `tokio` | 1.53.1 | MIT | `Cargo.toml` |
| `sqlx` | 0.9.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | `Cargo.toml` |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 | `Cargo.toml` |
| `askama` | 0.16.1 | MIT OR Apache-2.0 | `Cargo.toml` |
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 | `Cargo.toml` |
| `toml` | 1.1.6+spec-1.1.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `ammonia` | 4.2.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `pulldown-cmark` | 0.13.4 | MIT | `Cargo.toml` |
| `config` | 0.15.26 | MIT OR Apache-2.0 | `Cargo.toml` |
| `uuid` | 1.26.1 | Apache-2.0 OR MIT | `Cargo.toml` |
| `chrono` | 0.4.45 | MIT OR Apache-2.0 | `Cargo.toml` |
| `tower` | 0.5.3 | MIT | `Cargo.toml` |
| `tower-http` | 0.7.1 | MIT | `Cargo.toml` |
| `tracing` | 0.1.44 | MIT | `Cargo.toml` |
| `tracing-subscriber` | 0.3.23 | MIT | `Cargo.toml` |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | `Cargo.toml` |
| `sha2` | 0.11.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `hex` | 0.4.3 | MIT OR Apache-2.0 | `Cargo.toml` |
| `argon2` | 0.6.0 | MIT OR Apache-2.0 | `Cargo.toml` |
| `getrandom` | 0.4.3 | MIT OR Apache-2.0 | `Cargo.toml` |
| `reqwest` | 0.13.5 | MIT OR Apache-2.0 | `Cargo.toml` |
| `async-trait` | 0.1.92 | MIT OR Apache-2.0 | `Cargo.toml` |
| `ipnet` | 2.12.2 | MIT OR Apache-2.0 | `Cargo.toml` |

### MPL-2.0 crates linked into the server

The HTML sanitiser `ammonia` depends on two crates licensed under the Mozilla Public
License 2.0. Both are linked into the server binary, unmodified, from crates.io.

| Package | Locked version | Licence | Reached through |
|---------|----------------|---------|-----------------|
| `cssparser` | 0.38.0 | MPL-2.0 | `ammonia` |
| `dtoa-short` | 0.3.5 | MPL-2.0 | `cssparser` |

MPL-2.0 is a file-level copyleft licence. Its obligations arise only when the binary is
distributed to others. Magrathean runs the ViceDen server itself. Its release tarballs are
built for Magrathean's own hosts and are not distributed to others. If the binary is
distributed, the recipient must be told how to obtain the source code of these two
crates, and must receive a copy of the MPL-2.0 text. Any modification to their files must
be made available under MPL-2.0.

### Other licences in the locked dependency graph

The remaining transitive crates are under permissive licences: MIT, Apache-2.0, ISC,
BSD-2-Clause, BSD-3-Clause, Zlib, CC0-1.0, Unicode-3.0 and CDLA-Permissive-2.0
(`webpki-roots` and `webpki-root-certs`, the bundled root certificate list). `r-efi`
offers MIT OR Apache-2.0 OR LGPL-2.1-or-later; the MIT or Apache-2.0 option applies.

> **Note — commented-out dependency:** `webauthn-rs` is referenced in `Cargo.toml`
> as a future addition. It is not in `Cargo.lock` and is not built. Its licence is
> **MPL-2.0**. If it is enabled, the MPL-2.0 terms described above also apply to it.

## Python dependencies

### Command-line client — `cli/requirements.txt`

| Package | Pinned version | Licence | Declared in |
|---------|----------------|---------|-------------|
| `requests` | 2.34.2 | Apache-2.0 | `cli/requirements.txt` |

The client scripts in `cli/` install `requests` from PyPI. It is not bundled in this
repository.

---

## License Obligations Summary

| License | Action required |
|---------|----------------|
| Proprietary (this project) | No redistribution without written agreement — see `LICENSE` |
| MIT, ISC, BSD and similar permissive licences | Retain copyright notice and licence text when shipping the binary |
| Apache-2.0 (sole or chosen option) | Retain NOTICE file (if any) and licence text when shipping the binary |
| MPL-2.0 (`cssparser`, `dtoa-short`) | Only on distribution of the binary: include the MPL-2.0 text, say where the source is, and publish any modified MPL files under MPL-2.0 |

## Distribution record

The tables list the direct dependencies and the MPL-2.0 crates in `Cargo.lock` at the time
of writing. They are not a complete list of transitive dependencies, or a distribution
notice bundle. `Cargo.toml` and `Cargo.lock`, including the separate fuzz package, identify
the dependency sets to review. A distributor must review the exact locked dependency graph
and distributed artefact for required notices, licence texts, and attribution.
