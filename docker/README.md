# LAN container and experimental browser audio

On Linux, run `docker compose up -d --build`, or:

```sh
docker run -d --name roon-beacon --network host \
  --volume roon-beacon-data:/data \
  --env ROON_BEACON_WEB_TOKEN \
  ghcr.io/ca-x/roon-beacon:0.2.0
```

The optional `ROON_BEACON_WEB_TOKEN` supplies the Web access token. Without it,
the application generates durable credentials in `/data`. Keep this volume for
authorization and player state. The default command is
`--web-bind 0.0.0.0:9339 --data-dir /data --enable-audio`; no remote Core argument is
required for standalone LAN discovery. Set the token before starting, and use a
trusted LAN or TLS reverse proxy: plain HTTP does not encrypt authentication or audio.

Host networking is required for LAN SOOD and AirPlay multicast discovery; port
mapping alone is insufficient. Docker Desktop and routed/VPN networks may not
provide the required multicast behavior. Allow Web TCP 9339, mDNS UDP 5353,
SOOD UDP 9003, AirPlay RTSP TCP 5000 and the receiver's default UDP 6001–6010
on the appropriate trusted interfaces. No host D-Bus socket, host audio device,
privileged container, or additional Linux capability is required. A second host
mDNS stack can reduce reliability even though Avahi allows socket sharing.

The entrypoint runs a private system D-Bus and Avahi in the container filesystem,
waits for both to be ready, then drops the application to the packaged
`shairport-sync` user. Tini forwards signals and reaps orphaned children; the
entrypoint supervises and terminates its application, Avahi, and D-Bus children.
The healthcheck uses `wget` against the unauthenticated `/api/health`; it reports
process health, not Roon connectivity or audio availability.

## Audio integration

This is an experimental AirPlay receiver, **not native RAAT or Roon Bridge**.
Select its advertised output in Roon and explicitly join browser audio. The
shared MP3 output is lossy, delayed, and not synchronized across browsers.
Synthetic audio and mock tests do not verify real Roon interoperability.

The gateway invokes:

```text
shairport-sync -c /data/audio/shairport-sync.conf -o stdout -m avahi -a NAME
ffmpeg -hide_banner -loglevel error -nostdin -f s16le -ar 44100 -ac 2 \
  -probesize 32 -analyzeduration 0 -i pipe:0 -map 0:a:0 -c:a libmp3lame \
  -b:a 192k -reservoir 0 -write_xing 0 -id3v2_version 0 -write_id3v1 0 \
  -flush_packets 1 -f mp3 pipe:1
```

The receiver configuration selects `stdout`, Avahi, stereo playback, stderr
diagnostics, and disabled metadata. Debian Bookworm packages Shairport Sync
3.3.8 with `--with-stdout` and `--with-avahi`. Its stdout backend writes four
bytes per stereo sample, using the default 16-bit little-endian / 44,100 Hz
format; ALSA `output_format` settings are not stdout settings.

The encoder disables the MP3 bit reservoir and all startup tags. Every broadcast
message is one complete MPEG-1 Layer III frame so late listeners do not require
an initial Xing/ID3 header. The shared channel retains at most 64 frames
(approximately 1.7 seconds / 40 KiB), and there are at most 16 listeners. Lagged
listeners receive a terminated stream rather than skipped MP3 bytes. They must
rejoin explicitly. `bytes_streamed` counts encoded bytes produced once for the
shared output, not the sum of per-listener traffic.

`AudioGateway::start(enabled, name, data_dir)` always leaves missing-executable
and runtime receiver/encoder failures in `/api/audio/status`, without failing
the core application. Failures retry after five seconds, after both processes
are killed and reaped. Disabled audio starts no gateway processes. The router
owns its state and must be merged before the parent's cookie-authentication
middleware; it does not implement separate authentication. Call `shutdown()`
before stopping the Tokio runtime so cleanup can complete. Dropping the final
gateway/router handle also signals cleanup while the runtime remains alive.

## Publishing

The Docker workflow runs its own fmt, Clippy, tests (including FFmpeg), and docs
gate; it does not assume the separate binary-release workflow has finished.
Native `ubuntu-latest` amd64 and `ubuntu-24.04-arm` arm64 builds each run the
container smoke test before pushing image digests. Only after both pass are
digests merged and the registry manifest read back to assert both architectures.
Stable `v0.2.0` tags publish `0.2.0` and `latest`; prereleases publish their own
version only; `main` publishes `edge`. Arbitrary branches and pull requests do
not publish. Stable promotion refuses to overwrite an existing version tag, including on a workflow rerun; a rerun does not silently replace a published image rebuilt from mutable base images. Per-run native build tags remain separate. The repository owner must make the GHCR package public separately
when first publishing, if desired.

The smoke test first starts the real packaged receiver on private D-Bus/Avahi,
checks authenticated audio status, graceful stopping, and `/data` persistence.
It then substitutes a test-only receiver through `PATH` in a separate container,
feeding a synthetic 440 Hz PCM tone into the production encoder and HTTP route.
The authenticated MP3 download is probed and decoded inside the image, including
a nonzero-signal check. The fixture is not included in the production image.

## Primary references

- [Debian Bookworm build options](https://sources.debian.org/src/shairport-sync/3.3.8-1/debian/rules/)
- [Shairport Sync 3.3.8 stdout implementation](https://github.com/mikebrady/shairport-sync/blob/3.3.8/audio_stdout.c)
- [Shairport Sync 3.3.8 defaults and command options](https://github.com/mikebrady/shairport-sync/blob/3.3.8/shairport.c)
- [Shairport Sync 3.3.8 configuration](https://github.com/mikebrady/shairport-sync/blob/3.3.8/scripts/shairport-sync.conf)
- [FFmpeg MP3 muxer](https://ffmpeg.org/ffmpeg-formats.html#mp3)
- [FFmpeg 5.1 MP3 muxer documentation source](https://github.com/FFmpeg/FFmpeg/blob/n5.1.8/doc/muxers.texi#L1777)
- [FFmpeg libmp3lame encoder](https://ffmpeg.org/ffmpeg-codecs.html#libmp3lame-1)
- [Avahi daemon manual](https://github.com/avahi/avahi/blob/master/man/avahi-daemon.8.xml.in)
