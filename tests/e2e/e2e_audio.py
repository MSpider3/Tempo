"""Sound for scenarios that set `AUDIO = True`: a null sink in the session's private PipeWire,
and a recorder on its monitor. Nothing here touches any other sound server: every process
gets the session environment (private XDG_RUNTIME_DIR, so the private `pipewire-0`)."""

from __future__ import annotations

import array
import math
import signal
import subprocess
import time
import wave

from e2e_find import E2EError

SINK = "e2e-sink"
RATE = 48000


def make_sink(s):
    """A virtual output the (private) WirePlumber links Tempo to; it is also where we record."""
    props = f"{{ factory.name=support.null-audio-sink node.name={SINK} media.class=Audio/Sink object.linger=true audio.position=[ FL FR ] }}"
    done = subprocess.run(["pw-cli", "create-node", "adapter", props], env=s.env, capture_output=True, text=True, timeout=20, cwd=s.tmp)
    if done.returncode != 0 or "Error" in done.stderr or "Error" in done.stdout:
        raise E2EError(f"pw-cli could not create the null sink: {done.stderr or done.stdout}")


class Recording:
    """pw-record on the sink's monitor: 48 kHz, 2 channels, 16 bit. `stop()` returns [left, right] sample lists (-1..1)."""

    def __init__(self, s, name="audio"):
        self.path = s.tmp / f"{name}.wav"
        self.report = (s.report_dir / f"{name}.wav") if s.report_dir else None
        argv = ["pw-record", "--target", SINK, "-P", "stream.capture.sink=true", "--rate", str(RATE), "--channels", "2", "--format", "s16", str(self.path)]
        self.proc = subprocess.Popen(argv, env=s.env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, cwd=s.tmp)
        s.wait_for(lambda: self.path.exists() and self.path.stat().st_size > 0 or self.proc.poll() is not None, "the recorder to start")
        if self.proc.poll() is not None:
            raise E2EError(f"pw-record exited: {self.proc.stderr.read().decode(errors='replace')}")
        time.sleep(0.3)

    def stop(self):
        self.proc.send_signal(signal.SIGINT)  # finalises the WAV header
        try:
            self.proc.wait(10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(5)
        with wave.open(str(self.path)) as w:
            raw = array.array("h", w.readframes(w.getnframes()))
        if self.report:
            self.report.write_bytes(self.path.read_bytes())
        return [[v / 32768.0 for v in raw[c::2]] for c in (0, 1)]


def rms(x):
    return math.sqrt(sum(v * v for v in x) / len(x)) if x else 0.0


def tone_level(x, freq):
    """Amplitude of the `freq` component of `x` (Goertzel); 1.0 for a full-scale sine."""
    if not x:
        return 0.0
    w = 2 * math.pi * freq / RATE
    coeff, s1, s2 = 2 * math.cos(w), 0.0, 0.0
    for v in x:
        s1, s2 = v + coeff * s1 - s2, s1
    return 2 * math.sqrt(max(s1 * s1 + s2 * s2 - coeff * s1 * s2, 0.0)) / len(x)


def windows(x, seconds=0.25):
    """Consecutive slices of `seconds` each."""
    n = int(RATE * seconds)
    return [x[i:i + n] for i in range(0, len(x) - n + 1, n)]


def onset(x, threshold=0.01):
    """Index of the first sample louder than `threshold` (where the sound starts), or None."""
    return next((i for i, v in enumerate(x) if abs(v) > threshold), None)
