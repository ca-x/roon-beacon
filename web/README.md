# Embedded React frontend

Node is build-time only. From the repository root, run `npm --prefix web ci`,
`npm --prefix web run check`, and `npm --prefix web run build` using Node 24.
Vite emits a flat `app.js`, `styles.css`, and `index.html`, plus the original
branding PNGs from `public/`, into `crates/roon-beacon/src/web/`. Rust embeds
these files; there is no separate frontend server, CDN, telemetry, or runtime
environment secret.

## Components and data

- React 18.3.1 owns component state and DOM reconciliation.
- Published React Cupertino UI Button and TextField 2.0.5 are reused throughout
  transport controls, navigation, settings, prompts, and authentication. Their
  implicit SCSS imports are compiled with Sass. The packages incorrectly pin
  shared 2.0.1 with missing subpath exports; the npm override selects published
  shared 2.0.5, which has those exports. No aliases or vendor modifications are
  used. Do not import the broken `styles.css` package export.
- Radix Dialog provides modal semantics, keyboard focus trapping, and dismissal.
- Native ranges and selects preserve accessible browser input semantics. The
  Cupertino slider's decorative progress/thumb are not used for Roon transport.
- TanStack Query polls the player every two seconds and auth/runtime/audio every
  six seconds, passes cancellation signals, disables window-focus refetches, and
  scopes player responses to the current Core/auth revision.
- Ovio NowPlaying is intentionally not used: a controller must not create its
  own playback clock, seek requests, or automatic next-track behavior.
- English/Chinese and light/dark/system preferences persist locally. An inline
  head script resolves the initial theme before first paint. No token persists
  outside the server-issued HttpOnly cookie.

## Interaction invariants

Idle slider focus must never suspend polling. Only an active gesture or pending
commit protects its draft. Track, Core, zone, output, and volume bounds/type
changes invalidate that draft. A release commits its native value before clearing
the draft. Deduplication applies within a gesture, not to future gestures at the
same target. Detached controls and stale queued commits cannot address a removed
output or a replacement track. Live parent context validation also covers native
change events fired while React removes a focused output.

Browse uses a distinct random session for each document/tab, including ordinary
HTTP LAN origins where `crypto.randomUUID` is unavailable. Roon-provided action
lists and prompts are authoritative; `message` is displayed verbatim as text and
`is_error` selects an alert, never a success notification.

Audio Join invokes `HTMLMediaElement.play()` synchronously in the user's gesture;
Leave stops playback and removes the source. Browser loudness never changes Roon
output volume. Old play-promise rejections cannot cancel a newer listening attempt.

## Browser verification

`python3 tests/frontend_qa.py` serves only the generated assets and HTTP fixtures.
It uses its own named agent-browser session and closes it afterward. The suite
exercises real mouse and keyboard ranges, polling/focus/context regressions,
long unbroken titles, browsing, authentication, audio gestures, language/theme
persistence, responsive widths, and light/dark accessibility. It neither accesses
a user's Core nor exposes application internals as test globals.

See `THIRD_PARTY_NOTICES.md` for runtime package licenses, and
`licenses/react-cupertino-ui.txt` for Cupertino's original MIT notice.
