# Roon Beacon

English | [简体中文](README.zh-CN.md)

Roon Beacon publishes a remote Roon Core into your local LAN so Roon Remote and Control clients can discover and use it across network boundaries.

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

MIT.
