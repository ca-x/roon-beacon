# Changelog

## 0.2.1 — 2026-10-31

- Add an in-app AirPlay receiver network-interface selector with visible LAN addresses, private persistent settings, and audio-only restarts. Core connections and phone playback controls are unchanged; no new deployment parameters are required.
- Keep Web settings usable if a previously selected audio interface disappears, and allow recovery by selecting another interface or Automatic.
- Distinguish a connected browser waiting for Roon audio from browser buffering. Add PCM-input/MP3-output counters and bounded receiver/encoder diagnostics to help locate audio failures.
- Translate Roon library navigation, search prompts/buttons, and known playback actions into Chinese without translating artist, album, or track metadata.
- Fix truncated theme labels, including “跟随系统”, using intrinsic control sizing and a wrapping header. Verify narrow screens and enlarged fonts.
- Preserve existing token authentication and same-origin protection, including the new audio network API.

Browser audio remains an experimental AirPlay → MP3 gateway, not native RAAT/Roon Bridge. Interface selection is a troubleshooting control, not a guarantee that every network's playback failure is fixed. Acceptance testing with the actual Roon Server and LazyCat host is still required.
