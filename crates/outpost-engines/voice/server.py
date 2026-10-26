"""Outpost voice service: speech to text (faster-whisper) and text to speech (Kokoro).

Written for Outpost (MIT). Outpost starts it on 127.0.0.1 only; the gateway is the
one place that talks to it. Everything runs on the CPU, so it never competes with
chat or pictures for the graphics card.

  GET  /health            {"ok": true, ...}
  GET  /voices            {"voices": ["af_heart", ...]}
  POST /stt?language=xx&hotwords=Kat,zk4t   body: audio in any common format -> {"text", "language", "duration"}
  POST /tts               {"text", "voice", "speed"} -> audio/wav

`python server.py --prefetch` downloads the speech-recognition model and exits.
"""

import io
import json
import os
import sys
import threading
import wave
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

PORT = int(os.environ.get("OUTPOST_VOICE_PORT", "8190"))
MODELS = os.environ.get("OUTPOST_VOICE_MODELS", os.path.join(os.path.dirname(__file__), "models"))
STT_MODEL = os.environ.get("OUTPOST_STT_MODEL", "small")
KOKORO_DIR = os.path.join(MODELS, "kokoro")
WHISPER_DIR = os.path.join(MODELS, "whisper")
MAX_BODY = 30 * 1024 * 1024

_stt = None
_tts = None
_stt_lock = threading.Lock()
_tts_lock = threading.Lock()


def to_samples(body):
    """Any common audio format as 16 kHz mono float32, the way Whisper wants it.

    Decoded here rather than by faster-whisper, whose decoder passes an argument
    (metadata_errors) that newer PyAV releases removed.
    """
    import numpy as np

    if body[:4] == b"RIFF" and body[8:12] == b"WAVE":
        try:
            with wave.open(io.BytesIO(body)) as w:
                if w.getsampwidth() == 2 and w.getnchannels() == 1 and w.getframerate() == 16000:
                    return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0
        except wave.Error:
            pass
    import av

    resampler = av.AudioResampler(format="s16", layout="mono", rate=16000)
    chunks = []
    with av.open(io.BytesIO(body), mode="r") as container:
        for frame in container.decode(audio=0):
            frame.pts = None
            for out in resampler.resample(frame):
                chunks.append(out.to_ndarray().reshape(-1))
        for out in resampler.resample(None):
            chunks.append(out.to_ndarray().reshape(-1))
    if not chunks:
        return np.zeros(0, dtype=np.float32)
    return np.concatenate(chunks).astype(np.float32) / 32768.0


def stt_model():
    global _stt
    with _stt_lock:
        if _stt is None:
            from faster_whisper import WhisperModel

            threads = max(1, (os.cpu_count() or 4) - 2)
            _stt = WhisperModel(
                STT_MODEL, device="cpu", compute_type="int8", cpu_threads=threads, download_root=WHISPER_DIR
            )
        return _stt


def tts_model():
    global _tts
    with _tts_lock:
        if _tts is None:
            from kokoro_onnx import Kokoro

            _tts = Kokoro(os.path.join(KOKORO_DIR, "kokoro-v1.0.onnx"), os.path.join(KOKORO_DIR, "voices-v1.0.bin"))
        return _tts


# Kokoro voice names start with a letter for their language.
LANGS = {
    "a": "en-us",
    "b": "en-gb",
    "e": "es",
    "f": "fr-fr",
    "h": "hi",
    "i": "it",
    "j": "ja",
    "p": "pt-br",
    "z": "cmn",
}


def to_wav(samples, rate):
    import numpy as np

    pcm = (np.clip(samples, -1.0, 1.0) * 32767).astype("<i2").tobytes()
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(pcm)
    return out.getvalue()


class Handler(BaseHTTPRequestHandler):
    server_version = "OutpostVoice/1"

    def log_message(self, fmt, *args):  # quiet: the gateway logs requests
        pass

    def send_json(self, status, body):
        data = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def read_body(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0 or length > MAX_BODY:
            return None
        return self.rfile.read(length)

    def do_GET(self):
        path = urlparse(self.path).path
        if path == "/health":
            self.send_json(200, {"ok": True, "stt_model": STT_MODEL, "stt_loaded": _stt is not None, "tts_loaded": _tts is not None})
        elif path == "/voices":
            try:
                self.send_json(200, {"voices": tts_model().get_voices()})
            except Exception as e:  # noqa: BLE001 - report any load failure to the gateway
                self.send_json(500, {"error": f"Couldn't load the voices: {e}"})
        else:
            self.send_json(404, {"error": "not found"})

    def do_POST(self):
        url = urlparse(self.path)
        body = self.read_body()
        if body is None:
            self.send_json(400, {"error": "Send a body of up to 30 MB."})
            return
        try:
            if url.path == "/stt":
                query = parse_qs(url.query)
                language = (query.get("language") or [None])[0] or None
                samples = to_samples(body)
                if samples.size < 1600:  # under 0.1 s
                    self.send_json(200, {"text": "", "language": language or "", "duration": samples.size / 16000})
                    return
                # Names Whisper wouldn't guess (characters, personas), so "Kat" isn't written "Cat".
                hotwords = ((query.get("hotwords") or [None])[0] or "").strip()[:400] or None
                segments, info = stt_model().transcribe(
                    samples, language=language, vad_filter=True, beam_size=1, hotwords=hotwords
                )
                text = " ".join(s.text.strip() for s in segments).strip()
                self.send_json(200, {"text": text, "language": info.language, "duration": info.duration})
            elif url.path == "/tts":
                req = json.loads(body.decode("utf-8"))
                text = (req.get("text") or "").strip()
                if not text:
                    self.send_json(400, {"error": "Nothing to say."})
                    return
                voice = req.get("voice") or "af_heart"
                speed = float(req.get("speed") or 1.0)
                lang = LANGS.get(voice[:1], "en-us")
                samples, rate = tts_model().create(text, voice=voice, speed=max(0.5, min(2.0, speed)), lang=lang)
                wav = to_wav(samples, rate)
                self.send_response(200)
                self.send_header("Content-Type", "audio/wav")
                self.send_header("Content-Length", str(len(wav)))
                self.end_headers()
                self.wfile.write(wav)
            else:
                self.send_json(404, {"error": "not found"})
        except ValueError as e:
            self.send_json(400, {"error": str(e)})
        except Exception as e:  # noqa: BLE001 - the gateway shows this to the person
            self.send_json(500, {"error": f"{type(e).__name__}: {e}"})


def main():
    if "--prefetch" in sys.argv:
        stt_model()
        tts_model()
        print("Voice models ready.")
        return
    server = ThreadingHTTPServer(("127.0.0.1", PORT), Handler)
    print(f"Outpost voice listening on 127.0.0.1:{PORT}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
