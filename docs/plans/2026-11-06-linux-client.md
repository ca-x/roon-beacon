# Linux desktop and transport, v0.3.0

## Approved scope

- Keep the existing React, Cupertino, Radix, TanStack Query and Lucide UI.
- Add a Linux-only Tauri desktop application sharing the existing Rust server.
- Ship a Debian package. Do not add macOS or Windows desktop targets.
- Receive AirPlay through Shairport Sync and play directly to ALSA. Roon Bridge
  is not required; this is not RAAT, DSD or a promise of high-resolution playback.
- Declare Shairport Sync and Avahi as Debian dependencies; do not silently install
  software or manage unrelated system services from the application.
- Use roon-rs typed client/events/transport, retaining Beacon's tested discovery,
  authorization subscription, secure token persistence and reconnection ownership.
- Expose playback modes, grouping, ungrouping, transfers, source controls, relative
  volume and pause-all through validated APIs and accessible English/Chinese UI.

## Design

Use ui-ux-pro-max, emil-design-eng, animate, apple-design and frontend-design.
Preserve the existing listening-room visual language. Put frequent playback
controls beside the artwork and infrequent routing/device operations in a Radix
dialog. Use 44px targets, visible labels, capability-based disabled states,
context-safe pending actions, keyboard focus restoration and reduced motion.
Use short opacity/transform transitions only for user-triggered overlays; do not
animate seek positions, list polling or keyboard actions.

## Implementation order

1. Typed SDK adapter and checked transport operations, protocol regression tests.
2. Direct ALSA receiver with bounded diagnostics and explicit lifecycle cleanup.
3. Shared Rust library and authenticated loopback desktop host.
4. Existing React UI: modes, output management and native audio onboarding.
5. Linux desktop packaging, desktop validation and Debian artifact CI.
6. Full verification, review, version synchronization and release publication.

## Verification

`cargo fmt --all -- --check`

`cargo test --workspace --all-targets`

`cargo clippy --workspace --all-targets -- -D warnings`

`RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps`

`npm --prefix web run check && npm --prefix web run build`

`cargo check --manifest-path desktop/Cargo.toml`

`cargo tauri build --bundles deb` from `desktop/`

Inspect the package metadata, payload, dependency declarations, launcher and
version. Protocol fixtures and synthetic audio prove local contracts, not real
Roon authorization or audible playback on the user's hardware. Report those
hardware-dependent acceptance checks separately.
