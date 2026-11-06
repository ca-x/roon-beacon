# Roon Beacon for Linux

Linux-only desktop application built with Tauri 2 and the existing React,
Cupertino, Radix and TanStack Query frontend. No Electron runtime or Roon Bridge
is required. The desktop process embeds the same Rust player as the Web service.

## Install

Download the `.deb` matching your machine (`amd64` or `arm64`) from the release,
then run `sudo apt install ./roon-beacon*.deb`. Use APT rather than `dpkg -i` so
that Shairport Sync, Avahi and system audio dependencies are resolved.

Launch **Roon Beacon** from the applications menu. In Roon:

1. Enable **Roon Beacon** under **Settings → Extensions**.
2. Enable **Roon Beacon Desktop** under **Settings → Audio → AirPlay**.
3. Select that output and start playback. Music plays through the default ALSA
   device; PipeWire/PulseAudio routing depends on the system's ALSA configuration.

This is an AirPlay endpoint, not RAAT. It avoids the browser MP3 conversion but
does not promise high-resolution, DSD, bit-perfect playback or synchronized
multiroom audio. A running receiver is not evidence that Roon is sending audio.
The UI reports process availability and diagnostics, not unmeasured audibility.

The Debian package declares `shairport-sync`, `avahi-daemon` and ALSA dependencies.
Those are system packages, not statically bundled engines. Avahi must be running
and the audio device must be accessible in the logged-in user's session. Beacon
does not install, configure or manage unrelated system services itself.

The application owns a separate Shairport process using TCP port 5002 and UDP
ports 6101–6110, a private
configuration and the name **Roon Beacon Desktop**, so it does not stop or reuse
the distribution's Shairport service. It stops and reaps its own receiver when
the window closes. Network interface selection restarts only that receiver.

## Security and data

The embedded HTTP service uses a dynamically assigned loopback port and a random
per-launch authentication token. The token is passed in the initial URL fragment,
removed from browser history and exchanged for an HttpOnly session cookie. The
window blocks navigation outside its own loopback origin; remote pages have no
Tauri command permissions. Core authorization state is atomically saved in the
application data directory with private permissions.

## Build

On Debian/Ubuntu install the Tauri 2 Linux build prerequisites, including
`libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` and
`patchelf`. Then, from the repository root:

```sh
npm --prefix web ci
npm --prefix desktop ci
npm --prefix desktop run build
```

Packages appear in `desktop/target/release/bundle/deb/`. The build uses the
repository's pinned Cargo and npm lockfiles. Linux CI builds native amd64 and
arm64 packages, inspects metadata and payloads, and smoke-runs the extracted
desktop executable in a fresh virtual desktop session. Real Roon authorization
and audible hardware playback still require on-device acceptance.
