# Changelog

## 0.3.1 — 2026-11-07

- Declare the desktop's actual minimum glibc dependency (2.39) so APT rejects
  incompatible systems instead of installing an executable that cannot start.
- Document Ubuntu 24.04 / Debian 13 desktop compatibility and older-system
  source-build or Web/Docker alternatives.
- Fix the audio lifecycle test race: a spawned child is not proof its PID file
  exists yet. Synchronize PID publication and exercise deliberately delayed startup.

## 0.3.0 — 2026-11-06 (GitHub release candidate, unpublished)

- Add a Linux desktop application and amd64/arm64 Debian packages.
- Add direct ALSA AirPlay reception without installing Roon Bridge or using the
  browser MP3 pipeline. Debian packages declare Shairport Sync and Avahi.
- Reuse roon-rs RoonClient, RoonEvent, Core, Zone, Output and checked typed
  transport operations while preserving Beacon's authorization/reconnect loop.
- Add output subscriptions, playback modes, output grouping/ungrouping, playback
  transfer, device source/standby controls, relative seek/volume and pause-all.
- Preserve the current React component stack, themes, English/Chinese support,
  accessible controls and reduced-motion behavior.
- Keep the existing discovery bridge, Web service and experimental browser audio.
- Pin browser receiver PCM format to match FFmpeg and add a real RTSP/RTP audio
  validation tool. The previous user's silent deployment still needs field QA.

The desktop audio endpoint uses AirPlay, not RAAT. It does not promise
high-resolution, DSD or bit-perfect output. Hardware-dependent Roon authorization
and audible playback require a real Roon Server and audio device.

## 0.2.1 — 2026-10-31

- Add an in-app AirPlay receiver network-interface selector with visible LAN addresses, private persistent settings, and audio-only restarts. Core connections and phone playback controls are unchanged; no new deployment parameters are required.
- Keep Web settings usable if a previously selected audio interface disappears, and allow recovery by selecting another interface or Automatic.
- Distinguish a connected browser waiting for Roon audio from browser buffering. Add PCM-input/MP3-output counters and bounded receiver/encoder diagnostics to help locate audio failures.
- Translate Roon library navigation, search prompts/buttons, and known playback actions into Chinese without translating artist, album, or track metadata.
- Fix truncated theme labels, including “跟随系统”, using intrinsic control sizing and a wrapping header. Verify narrow screens and enlarged fonts.
- Preserve existing token authentication and same-origin protection, including the new audio network API.

Browser audio remains an experimental AirPlay → MP3 gateway, not native RAAT/Roon Bridge. Interface selection is a troubleshooting control, not a guarantee that every network's playback failure is fixed. Acceptance testing with the actual Roon Server and LazyCat host is still required.
