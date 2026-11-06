# Browser audio validation

Browser audio is an experimental AirPlay → Shairport Sync stdout → FFmpeg MP3
→ HTTP → HTML audio pipeline. It does not turn every existing Roon zone into a
browser stream: enable and select **Roon Beacon Web** in Roon Settings → Audio,
then press **Join audio** in Beacon. The two volume controls are independent.

## Reproducible receiver test

On an isolated test instance, start Beacon with `--enable-audio`. It needs
Shairport Sync with stdout support, FFmpeg and a working Avahi service. Do not
point this test at a speaker: it sends a 15-second 440 Hz tone.

```sh
uv run --with pyatv==0.18.0 tests/raop_audio_qa.py \
  --server http://127.0.0.1:9339 \
  --receiver-host 127.0.0.1 --receiver-port 5000
```

The isolated instance must use loopback HTTP without authentication. The test
uses pyatv's RTSP/RTP session with uncompressed ALAC frames, because classic
Shairport 3.3.8 expects ALAC rather than pyatv's default L16 payload. It checks
actual receiver PCM counters, HTTP MP3 bytes, decoded amplitude and frequency.
This is not a substitute for Roon, mDNS across the LAN, or audible hardware QA.

## Evidence on 2026-11-06

- Shairport Sync 3.3.8, real RTSP/RTP packets, FFmpeg and Beacon HTTP produced
  393,717 bytes of MP3 from a 15-second test signal. The decoded signal measured
  440 Hz, with nonzero amplitude.
- Chromium joined through the actual UI. During a longer signal it reported
  `paused=false`, `readyState=3`, and a running AudioContext. A Web Audio FFT
  measured a 441.43 Hz peak (8192-point FFT, 44.1 kHz sample rate).
- The local receiver used tinysvcmdns and an isolated RTSP port because the test
  host lacked Avahi. This does not verify production Avahi discovery or Roon.
- New Shairport AirPlay 2 stdout defaults differ from classic AirPlay. Beacon
  explicitly requests S16_LE / 44100 Hz / stereo, matching its FFmpeg input.
- The user's previously silent Web deployment remains unverified. No claim is
  made that its underlying network, authorization, routing or audio issue is fixed.

## Locating a silent stream

Open **Receiver diagnostics** while Roon is actively playing the Beacon output:

1. PCM stays at zero: check the selected Roon output, Roon's error, AirPlay
   enablement, chosen LAN interface, multicast visibility and receiver errors.
2. PCM increases but MP3 does not: inspect encoder diagnostics and FFmpeg.
3. Both increase but the browser stays buffering: inspect the stream request,
   reverse-proxy buffering, browser playback errors and whether it is muted.
4. Playback time advances but no sound is audible: check Roon output volume,
   Beacon browser volume, tab mute and system output routing.

The receiver uses TCP 5000 and UDP ports starting at 6001. Existing AirPlay
services or a firewall can conflict. Desktop audio uses its separate TCP 5002
and UDP 6101–6110 range and bypasses the browser MP3 stream.
