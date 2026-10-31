# Roon Beacon

English | [简体中文](README.zh-CN.md)

Roon Beacon publishes a remote Roon Core into your local LAN so Roon Remote and Control clients can discover and use it across network boundaries.

Version 0.2 also discovers local Roon Servers, provides a browser-based music remote, and includes an experimental AirPlay-to-browser audio gateway. It is not a native RAAT/Roon Bridge implementation.

## Web Player and Docker

Run the binary without a remote address for standalone discovery and playback control:

```bash
cargo run --release -- --data-dir ./data
```

Open `http://127.0.0.1:9339`, then enable **Roon Beacon** in **Roon Settings → Extensions**. Choose a Server and playback zone in the Web UI. Controls include play/pause, previous/next, seeking, output volume/mute, library navigation, search, and cover art. Unsupported controls remain unavailable rather than pretending to succeed.

The interface uses the supplied project logo, offers English/Chinese language switching, and supports light, dark, or system themes. Language and theme preferences are saved in the current browser.

For Docker on Linux, use the included `compose.yml`:

```bash
docker compose up -d
docker compose exec roon-beacon cat /data/web-access-token
```

Open `http://<docker-host-ip>:9339` and enter the generated token. The image is `ghcr.io/ca-x/roon-beacon:0.2.0`, with `linux/amd64` and `linux/arm64` support. `latest` tracks stable version tags; pin a version or digest for repeatable deployments. The container uses host networking for SOOD and AirPlay/mDNS discovery. Docker Desktop bridge networking is not an equivalent LAN multicast setup.

The `/data` volume contains private authorization tokens and the selected Server. Keep it across container updates. Optionally set `ROON_BEACON_WEB_TOKEN` to a private 16–256-character token using ASCII letters, digits, `-`, `_`, or `.`. On a non-loopback Web bind, authentication is always enabled: a token is generated when none is configured. Do not expose this service directly to the Internet; use a trusted LAN, VPN, or HTTPS reverse proxy. Reverse proxies must preserve the browser-visible `Host` for same-origin checks.

Unauthenticated loopback access accepts only `localhost` or loopback-IP Host names. When proxying a loopback listener under another hostname, configure an explicit Web token rather than bypassing this DNS-rebinding protection.

### Listen in the Browser

The Docker image includes the optional receiver and encoder. With the binary, install **Shairport Sync with the stdout backend**, **FFmpeg with libmp3lame**, and a working Avahi/mDNS service, then run:

```bash
./target/release/roon-beacon --enable-audio --audio-name "Roon Beacon Web"
```

1. In **Roon Settings → Audio**, enable the AirPlay device **Roon Beacon Web**.
2. Select that zone in Roon or the Web UI and start playing music.
3. Click **Join audio** in the Web UI to join the stream. Browser playback requires this explicit click; it does not autoplay.

```text
Roon Server → AirPlay → Shairport Sync → PCM → FFmpeg MP3 → browser audio
```

This is an **experimental, shared AirPlay output**, not RAAT or a separate Roon zone per browser. MP3 encoding is lossy, buffering adds latency, and multiple browsers are not synchronized. Leaving a browser stream does not pause the shared Roon zone. The receiver must be discoverable and reachable by the Roon Server; the existing SOOD reverse proxy does not forward AirPlay/mDNS across a VPN. No host sound card or `/dev/snd` mount is needed.

Audio availability and missing-runtime errors appear in the UI. Automated mock-Core and synthetic-audio tests do not replace acceptance testing with your actual Roon Server, network, and browser.

### New Options

```text
    --web-bind <ADDR>      Web listener (default 127.0.0.1:9339)
    --data-dir <PATH>      Private persistent state (default data)
    --web-token <TOKEN>    Optional explicit Web access token
    --enable-audio         Enable the experimental AirPlay receiver
    --audio-name <NAME>    AirPlay name (default Roon Beacon Web)
```

The existing `--remote-ip` bridge mode remains supported and requires `--local-ip`. Without `--remote-ip`, no bridge forwarding ports are opened. The settings panel still exposes the original bridge counters and runtime settings.

## What It Does

Roon discovers Cores through SOOD, a UDP multicast protocol on `239.255.90.90:9003`. Multicast usually stops at the local layer-2 network, so a Core behind VPN, Tailscale, or another routed segment may be reachable by IP but invisible to Roon Remote.

Roon Beacon runs on a machine inside the client LAN and exposes the remote Core as a local discovery and forwarding endpoint:

```text
Roon Remote  -- SOOD/TCP -->  Roon Beacon on local LAN  -- TCP/UDP -->  Remote Roon Core
```

It handles three jobs:

1. SOOD responses: listens for local discovery queries and prefers live byte-for-byte responses from the real Core.
2. TCP forwarding: listens on the local LAN IP for key Roon ports and forwards traffic to the remote Core.
3. Optional reverse SOOD proxy: forwards local RAAT Bridge announcements back to the remote Core.

The binary also embeds a small Web UI at `http://127.0.0.1:9339` with English and Chinese language switching.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/roon-beacon`.

The UI uses React, TypeScript, Vite, and React Cupertino UI components. Generated frontend assets are committed and embedded in the binary, so building or running the Rust binary does not require Node. After editing frontend sources under `web/`, regenerate them before the Cargo build:

```bash
npm --prefix web ci
npm --prefix web run check
npm --prefix web run build
cargo build --release
```

CI checks that committed assets match a fresh frontend build. Docker performs both builds in separate Node and Rust builder stages; the runtime image has no Node server.

For browser contract regressions, install `agent-browser` and its Chromium browser, then run `python3 tests/frontend_qa.py`. The suite uses an isolated mock server, not your Roon Server.

## Quick Start

```bash
./target/release/roon-beacon -r <remote-core-host> -l <local-lan-ip>
```

Example:

```bash
./target/release/roon-beacon -r 100.107.146.8 -l 192.168.1.10
```

Use `-r / --remote-ip` for the remote Core address and `-l / --local-ip` for the LAN address visible to phones, tablets, or computers running Roon Remote. Do not use a VPN or overlay IP for `--local-ip`.

Open the embedded Web UI:

```text
http://127.0.0.1:9339
```

The UI shows status counters and lets you update runtime settings that do not require rebinding sockets, such as fallback display name, active announcement interval, live SOOD timeout, and reverse proxy settings.

## Options

```text
-r, --remote-ip <HOST>        Remote Core host, IP, or DNS name
-l, --local-ip  <IP>          Local LAN IP visible to Roon Remote clients
-n, --name      <NAME>        Fallback Core display name
    --prefer    <v4|v6|any>   Address family preference for DNS resolution
    --broadcast-interval <N>  Active SOOD announcement interval, seconds
    --use-broadcast           Use UDP broadcast instead of multicast
    --retry-secs <N>          TCP upstream retry interval, seconds
    --skip-ports <P1,P2>      Skip local TCP forwarding ports
    --no-probe                Skip startup SOOD template probing
    --probe-timeout <N>       Startup probe timeout, seconds
    --reverse-proxy           Forward local RAAT announcements to the Core
    --proxy-ip <IP>           Reachable proxy IP used for reverse announcements
```

## Common Setups

### NAS + Tailscale

Run Roon Server and Tailscale on the NAS, then run Roon Beacon on a machine in the client LAN:

```bash
./target/release/roon-beacon -r 100.107.146.8 -l 192.168.1.10
```

Phones on the same Wi-Fi as the Beacon host do not need Tailscale.

### Public DNS or Port Mapping

```bash
./target/release/roon-beacon -r nas.example.com -l 192.168.1.10 --prefer v4
```

Use `--prefer v4` when a private or unreachable IPv6 address is returned first.

### Multicast Blocked by Router

```bash
./target/release/roon-beacon -r 100.107.146.8 -l 192.168.1.10 --use-broadcast
```

This sends active announcements to `255.255.255.255:9003`.

## Troubleshooting

If Roon Remote cannot find the Core, confirm that:

- the Beacon host and Roon Remote are on the same LAN or Wi-Fi;
- the log shows TCP listeners for at least `9330`;
- the log shows incoming SOOD queries from the client LAN;
- no other local Roon Bridge or RAATServer process owns the required port.

If probing fails, verify that the remote Core is running and UDP `9003` is reachable. You can start with `--no-probe`, but live probing gives Roon Remote the most accurate `service_id`, `unique_id`, and version fields.

If playback is unstable, inspect the actual RAATServer-to-Core TCP path. Control-plane bridging cannot fix packet loss, reordering, or bandwidth limits on the audio stream itself.

## Releases

GitHub Actions builds release-ready binaries for Linux, macOS, and Windows. Tagged releases upload archives with the compiled `roon-beacon` executable.

## License

GPL-3.0-only. See `LICENSE`. Third-party runtime components retain their respective licenses.
