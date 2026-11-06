"""Send ALAC over real RTSP/RTP, capture the gateway MP3, and verify its tone."""

import argparse
import array
import asyncio
import ipaddress
import json
import math
from pathlib import Path
import struct
import subprocess
import tempfile
import threading
import urllib.request
import wave

import pyatv
from pyatv.conf import AppleTV, ManualService
from pyatv.const import Protocol
from pyatv.protocols.raop.protocols.airplayv1 import AirPlayV1
from pyatv.support import rtsp


async def send_alac(self, transport, rtp_header, audio):
    bits = len(audio) * 8
    payload = (((0x100001 << bits) | int.from_bytes(audio, "big")) << 3) | 7
    padding = (-(23 + bits + 3)) % 8
    packet = rtp_header + (payload << padding).to_bytes((23 + bits + 3 + padding) // 8, "big")
    transport.sendto(packet)
    return self.context.rtpseq, packet


async def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", default="http://127.0.0.1:9339")
    parser.add_argument("--receiver-host", default="127.0.0.1")
    parser.add_argument("--receiver-port", type=int, default=5000)
    args = parser.parse_args()
    base = args.server.rstrip("/")
    with urllib.request.urlopen(base + "/api/audio/status", timeout=5) as response:
        before = json.load(response)
    assert before["mode"] == "browser" and before["available"], before
    captured = bytearray()
    errors = []
    ready = threading.Event()
    stop = threading.Event()

    def capture():
        try:
            with urllib.request.urlopen(base + "/api/audio/stream", timeout=10) as response:
                assert response.headers.get_content_type() == "audio/mpeg"
                ready.set()
                while not stop.is_set():
                    chunk = response.read(4096)
                    if not chunk:
                        break
                    captured.extend(chunk)
        except TimeoutError:
            if not stop.is_set():
                errors.append("No MP3 arrived within ten seconds")
        except Exception as failure:
            errors.append(str(failure))

    AirPlayV1.send_audio_packet = send_alac
    rtsp.ANNOUNCE_PAYLOAD = rtsp.ANNOUNCE_PAYLOAD.replace("L16/44100/2", "AppleLossless")
    with tempfile.TemporaryDirectory(prefix="beacon-raop-") as directory:
        path = Path(directory) / "tone.wav"
        frames = bytearray()
        for index in range(44100 * 15):
            sample = int(8000 * math.sin(2 * math.pi * 440 * index / 44100))
            frames.extend(struct.pack("<hh", sample, sample))
        with wave.open(str(path), "wb") as output:
            output.setparams((2, 2, 44100, 0, "NONE", "not compressed"))
            output.writeframes(frames)
        listener = threading.Thread(target=capture, daemon=True)
        listener.start()
        assert await asyncio.to_thread(ready.wait, 5), errors
        config = AppleTV(ipaddress.IPv4Address(args.receiver_host), "Beacon QA")
        config.add_service(ManualService("001122334455", Protocol.RAOP, args.receiver_port, {
            "cn": "1", "et": "0,1", "sr": "44100", "ss": "16", "ch": "2",
        }))
        device = await pyatv.connect(config, asyncio.get_running_loop())
        try:
            await device.audio.set_volume(80)
            await device.stream.stream_file(str(path))
        finally:
            await asyncio.gather(*device.close())
            stop.set()
            await asyncio.to_thread(listener.join, 12)
    assert not listener.is_alive() and not errors, errors
    assert len(captured) > 200000, len(captured)
    decoded = subprocess.run([
        "ffmpeg", "-hide_banner", "-loglevel", "error", "-i", "pipe:0", "-f", "s16le",
        "-acodec", "pcm_s16le", "-ar", "44100", "-ac", "2", "pipe:1",
    ], input=captured, capture_output=True, check=True, timeout=20)
    samples = array.array("h", decoded.stdout)
    if struct.pack("=H", 1) != b"\x01\x00":
        samples.byteswap()
    channel = samples[44100 * 3 * 2:44100 * 8 * 2:2]
    assert len(channel) == 44100 * 5, len(channel)
    rms = math.sqrt(sum(sample * sample for sample in channel) / len(channel))
    frequency = sum(left <= 0 < right for left, right in zip(channel, channel[1:])) / 5
    with urllib.request.urlopen(base + "/api/audio/status", timeout=5) as response:
        after = json.load(response)
    assert after["pcm_bytes_received"] > before["pcm_bytes_received"], after
    assert after["bytes_streamed"] > before["bytes_streamed"], after
    assert rms > 10 and abs(frequency - 440) < 3, (rms, frequency)
    print(json.dumps({"result": "PASS", "mp3_bytes": len(captured), "rms": rms, "frequency_hz": frequency}))


if __name__ == "__main__":
    asyncio.run(main())
