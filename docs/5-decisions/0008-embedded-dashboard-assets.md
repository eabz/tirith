# ADR-0008: Dashboard assets are embedded in the binary from `src/`

**Status:** Accepted, 2026-09-15

## Context

The dashboard gained a logo (header image and favicon). The crates.io
package only ships `/src/**`, `Cargo.toml`, `README.md` and `LICENSE`
(see `include` in `Cargo.toml`), and the daemon is meant to run with
nothing on disk besides the binary. Documentation images live in
`docs/_static/images/`, which is not part of the published crate.

## Decision

Everything the dashboard serves is embedded at compile time with
`include_str!` / `include_bytes!` from files next to `src/dashboard.rs`.
The logo is a 192px PNG with transparent rounded corners at
`src/dashboard-logo.png`, cut from `docs/_static/images/logo.jpeg`, and
served at `/logo.png` with a one-day cache header. The page loads no
external fonts, scripts, or stylesheets.

## Alternatives

- **`include_bytes!` from `docs/_static/images/`.** Breaks the crates.io
  build unless `include` grows to cover `docs/`, which would also ship
  the documentation tree with the crate.
- **Serve files from disk at runtime.** Adds a runtime dependency on the
  repository checkout and a path-traversal surface for no benefit.
- **A CDN font or icon set.** The dashboard must work offline and on
  air-gapped machines.

## Consequences

- `src/dashboard-logo.png` is a source file: regenerate it from the
  master logo when the logo changes, and keep it small (it is currently
  under 20 KiB) because it is compiled into every binary.
- New dashboard assets follow the same rule: a file in `src/`, a route in
  `src/dashboard.rs`, a test in `tests/http_roundtrip.rs`.
