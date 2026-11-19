#!/usr/bin/env python3
"""Tempo end-to-end harness: the real app on a real, private, headless Wayland compositor.

Read docs/TESTING_E2E.md first. In short:

    with Session(project="fixture.tempo", report_dir="out") as s:
        clip = s.clip(track="V1", index=0)
        s.click(*center(clip["rect"]))
        s.key("ctrl+t")
        s.wait_for(lambda: s.transitions(), "a transition appears")
        s.screenshot("after")

Every Session starts its own D-Bus session bus, PipeWire + WirePlumber (no hardware),
`mutter --headless --wayland --no-x11` and Tempo, all inside a fresh temp directory
with its own HOME and XDG dirs. Input goes through mutter's RemoteDesktop API on that
private bus; screenshots come from mutter's ScreenCast stream on that private PipeWire.
Nothing talks to the user's desktop, sound server or session bus.

Command line:  python3 tests/e2e/tempo_e2e.py run --project P --report DIR [scenario ...]
"""

from __future__ import annotations

import argparse
import ctypes
import importlib.util
import json
import os
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import traceback
import zlib
from pathlib import Path

import gi

gi.require_version("Gst", "1.0")
from gi.repository import Gio, GLib, Gst  # noqa: E402

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DEFAULT_APP = Path(os.environ.get("CARGO_TARGET_DIR", REPO / "target")) / "e2e-bin" / "tempo"  # copied there by scripts/e2e.sh

BTN = {"left": 272, "right": 273, "middle": 274}  # linux/input-event-codes.h
MODIFIERS = {"ctrl": 0xFFE3, "control": 0xFFE3, "shift": 0xFFE1, "alt": 0xFFE9, "super": 0xFFEB, "meta": 0xFFEB}
KEYSYMS = {
    "backspace": 0xFF08, "tab": 0xFF09, "return": 0xFF0D, "enter": 0xFF0D, "escape": 0xFF1B, "esc": 0xFF1B,
    "delete": 0xFFFF, "home": 0xFF50, "left": 0xFF51, "up": 0xFF52, "right": 0xFF53, "down": 0xFF54,
    "page_up": 0xFF55, "pageup": 0xFF55, "page_down": 0xFF56, "pagedown": 0xFF56, "end": 0xFF57,
    "insert": 0xFF63, "space": 0x20, "plus": 0x2B, "minus": 0x2D, "comma": 0x2C, "period": 0x2E,
    "bracketleft": 0x5B, "bracketright": 0x5D, "slash": 0x2F, "backslash": 0x5C, "grave": 0x60,
    **{f"f{i}": 0xFFBE + i - 1 for i in range(1, 13)},
}
STEP_DELAY = 0.03  # between low-level input events, like a quick human


class E2EError(AssertionError):
    """A failed expectation or a harness step that did not happen in time."""


# ---- Small helpers ---------------------------------------------------------------------


def center(rect):
    """Middle of an [x, y, w, h] rectangle."""
    x, y, w, h = rect
    return (x + w / 2.0, y + h / 2.0)


def keysym(name: str) -> int:
    """X keysym of a key name ("BackSpace", "Up", "t", "[", "F5")."""
    if len(name) == 1:
        code = ord(name)
        return code if 0x20 <= code <= 0x7E else 0x01000000 | code
    key = name.lower()
    if key in KEYSYMS:
        return KEYSYMS[key]
    if key in MODIFIERS:
        return MODIFIERS[key]
    raise E2EError(f"unknown key name {name!r}")


def short_tmp() -> str:
    # Wayland socket paths must stay under 108 bytes, so use /tmp, not a deep TMPDIR.
    return os.environ.get("TEMPO_E2E_TMP", "/tmp")


def _die_with_parent():
    # Children get SIGKILL if the harness itself dies, so nothing is left running.
    try:
        ctypes.CDLL("libc.so.6", use_errno=True).prctl(1, signal.SIGKILL)  # PR_SET_PDEATHSIG
    except OSError:
        pass


class Image:
    """A screenshot: RGB pixels in screen coordinates."""

    def __init__(self, width: int, height: int, stride: int, data: bytes):
        self.width, self.height, self.stride, self.data = width, height, stride, data

    def pixel(self, x, y):
        x, y = int(x), int(y)
        if not (0 <= x < self.width and 0 <= y < self.height):
            raise E2EError(f"pixel {x},{y} is outside the {self.width}x{self.height} screen")
        i = y * self.stride + x * 4
        return tuple(self.data[i : i + 3])

    def mean(self, rect, step=2):
        """Average (r, g, b) inside [x, y, w, h], sampling every `step` pixels."""
        x0, y0, w, h = (int(v) for v in rect)
        total, n = [0, 0, 0], 0
        for y in range(max(y0, 0), min(y0 + h, self.height), step):
            row = y * self.stride
            for x in range(max(x0, 0), min(x0 + w, self.width), step):
                i = row + x * 4
                total[0] += self.data[i]
                total[1] += self.data[i + 1]
                total[2] += self.data[i + 2]
                n += 1
        if n == 0:
            raise E2EError(f"empty rectangle {rect}")
        return tuple(round(t / n, 1) for t in total)

    def brightness(self, rect=None):
        r, g, b = self.mean(rect or [0, 0, self.width, self.height], step=4)
        return (r + g + b) / 3.0

    def save(self, path):
        rows = []
        for y in range(self.height):
            line = memoryview(self.data)[y * self.stride : y * self.stride + self.width * 4]
            rgb = bytearray(self.width * 3)
            rgb[0::3], rgb[1::3], rgb[2::3] = line[0::4], line[1::4], line[2::4]
            rows.append(b"\0" + bytes(rgb))
        raw = b"".join(rows)

        def chunk(kind, body):
            return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)

        png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", self.width, self.height, 8, 2, 0, 0, 0))
        png += chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b"")
        Path(path).write_bytes(png)
        return path


# ---- The app's control socket ------------------------------------------------------------


class Control:
    """Line-based JSON channel to the app (TEMPO_TEST_CONTROL, see crates/tempo-ui/src/app.rs)."""

    def __init__(self, path: str, timeout=15.0):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(timeout)
        self.sock.connect(path)
        self.buf = b""

    def call(self, line: str):
        self.sock.sendall(line.encode() + b"\n")
        while b"\n" not in self.buf:
            more = self.sock.recv(1 << 20)
            if not more:
                raise E2EError(f"the app closed the control socket during {line!r}")
            self.buf += more
        reply, self.buf = self.buf.split(b"\n", 1)
        answer = json.loads(reply)
        if not answer.get("ok"):
            raise E2EError(f"control {line!r}: {answer.get('error')}")
        return answer.get("result")

    def close(self):
        self.sock.close()


# ---- Session -------------------------------------------------------------------------------


class Session:
    """Compositor + app + input + screenshots for one test. Use as a context manager."""

    def __init__(self, project=None, report_dir=None, size=(1600, 1000), app=None, extra_env=None, start_timeout=60.0):
        self.project = Path(project).resolve() if project else None
        self.report_dir = Path(report_dir) if report_dir else None
        self.size = size
        self.app_bin = Path(app or os.environ.get("TEMPO_E2E_APP", DEFAULT_APP)).resolve()
        self.extra_env = extra_env or {}
        self.start_timeout = start_timeout
        self.procs: list[tuple[str, subprocess.Popen]] = []
        self.logs: dict[str, Path] = {}
        self.control = None
        self.bus = None
        self.pipeline = None
        self.rd_session = None
        self._frame = None
        self._frames = 0
        self._frame_at = 0.0
        self._lock = threading.Lock()
        self.shots = []
        self.notes = {}  # measurements a scenario wants in its result.json (see note())

    # -- lifecycle

    def __enter__(self):
        try:
            self.start()
        except BaseException:
            self.close()
            raise
        return self

    def __exit__(self, *exc):
        self.close()
        return False

    def start(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="te2e-", dir=short_tmp()))
        os.chmod(self.tmp, 0o700)
        self.run_dir = self.tmp / "run"
        for d in ["run", "home", "config", "data", "cache", "state", "logs"]:
            (self.tmp / d).mkdir(mode=0o700)
        self.display = f"tempo-e2e-{os.getpid()}"
        self.env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "LANG": "C.UTF-8",
            "HOME": str(self.tmp / "home"),
            "XDG_RUNTIME_DIR": str(self.run_dir),
            "XDG_CONFIG_HOME": str(self.tmp / "config"),
            "XDG_DATA_HOME": str(self.tmp / "data"),
            "XDG_CACHE_HOME": str(self.tmp / "cache"),
            "XDG_STATE_HOME": str(self.tmp / "state"),
            "GSETTINGS_BACKEND": "memory",
            "GIO_USE_VFS": "local",
            "NO_AT_BRIDGE": "1",
            "GTK_A11Y": "none",
        }
        self._start_bus()
        self._spawn("pipewire", ["pipewire"])
        self._wait_path(self.run_dir / "pipewire-0", "PipeWire socket")
        # The "policy" profile links streams but opens no sound cards, cameras or Bluetooth.
        self._spawn("wireplumber", ["wireplumber", "--profile", "policy"])
        w, h = self.size
        self._spawn("mutter", ["mutter", "--headless", "--wayland", "--no-x11", "--wayland-display", self.display, "--virtual-monitor", f"{w}x{h}"])
        self._wait_path(self.run_dir / self.display, "Wayland socket")
        self._wait(lambda: self._has_name("org.gnome.Mutter.ScreenCast") and self._has_name("org.gnome.Mutter.RemoteDesktop"), 20, "mutter's D-Bus services")
        self._start_remote_desktop()
        self._start_app()

    def _start_bus(self):
        address = f"unix:path={self.run_dir}/bus"
        # A session bus with no service directories: nothing (portals, gvfs, ...) is
        # started on demand behind the harness's back.
        config = self.tmp / "bus.conf"
        config.write_text(
            "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n"
            " \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n"
            f"<busconfig><type>session</type><listen>{address}</listen><auth>EXTERNAL</auth>"
            "<policy context=\"default\"><allow send_destination=\"*\" eavesdrop=\"true\"/>"
            "<allow eavesdrop=\"true\"/><allow own=\"*\"/></policy></busconfig>\n"
        )
        self._spawn("dbus", ["dbus-daemon", f"--config-file={config}", "--nofork", "--nopidfile"])
        self._wait_path(self.run_dir / "bus", "D-Bus socket")
        self.env["DBUS_SESSION_BUS_ADDRESS"] = address
        flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
        self.bus = Gio.DBusConnection.new_for_address_sync(address, flags, None, None)

    def _spawn(self, name, argv, env=None):
        log = self.tmp / "logs" / f"{name}.log"
        self.logs[name] = log
        out = open(log, "wb")
        proc = subprocess.Popen(argv, env=env or self.env, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, cwd=self.tmp, preexec_fn=_die_with_parent)
        out.close()
        self.procs.append((name, proc))
        return proc

    def _start_app(self):
        if not self.app_bin.exists():
            raise E2EError(f"no app binary at {self.app_bin}; run scripts/e2e.sh once, or pass --app")
        env = dict(self.env)
        env.update({
            "WAYLAND_DISPLAY": self.display,
            "GDK_BACKEND": "wayland",
            "TEMPO_TEST_CONTROL": str(self.run_dir / "control.sock"),
            "TEMPO_TEST_FULLSCREEN": "1",
            "TEMPO_NO_AUDIO": "1",
            "TEMPO_NO_PROXY": "1",
            "RUST_LOG": os.environ.get("RUST_LOG", "info"),
        })
        if self.project:
            copy = self.tmp / "home" / self.project.name
            shutil.copy(self.project, copy)
            env["TEMPO_OPEN"] = str(copy)
            self.project_copy = copy
        env.update(self.extra_env)
        self.app = self._spawn("app", [str(self.app_bin)], env)
        self._wait_path(self.run_dir / "control.sock", "the app's control socket", self.start_timeout)
        self.control = Control(str(self.run_dir / "control.sock"))
        self.control.call("ping")
        w, h = self.size
        self._wait(lambda: self.geometry()["window"][2:] == [w, h] and self.geometry()["fullscreen"], 20, "the window to be fullscreen")
        if self.project:
            self._wait(lambda: self.state()["project_loaded"] and self.geometry()["timeline"], self.start_timeout, "the project to open")
        # Give the window pointer focus with a little motion.
        self.move(w / 2, h / 2)
        self.move(w / 2 + 1, h / 2 + 1)
        self.wait_idle()
        self.settle()

    def close(self):
        if self.report_dir:
            self.report_dir.mkdir(parents=True, exist_ok=True)
        if self.control:
            try:
                self.control.call("quit")
            except Exception:
                pass
            self.control.close()
            self.control = None
        if self.pipeline:
            self.pipeline.set_state(Gst.State.NULL)
            self.pipeline = None
            self._pw_sock.close()
        if self.rd_session and self.bus:
            try:
                self._call("org.gnome.Mutter.RemoteDesktop", self.rd_session, "org.gnome.Mutter.RemoteDesktop.Session", "Stop", None)
            except Exception:
                pass
            self.rd_session = None
        # App first, bus last; each gets SIGTERM, then SIGKILL after 5 s.
        order = ["app", "mutter", "wireplumber", "pipewire", "dbus"]
        for name, proc in sorted(self.procs, key=lambda p: order.index(p[0]) if p[0] in order else -1):
            if proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(5)
        self.procs = []
        if self.bus:
            try:
                self.bus.close_sync(None)
            except GLib.Error:
                pass  # the bus daemon is already gone
            self.bus = None
        if getattr(self, "tmp", None):
            if self.report_dir:
                for name, log in self.logs.items():
                    if log.exists():
                        shutil.copy(log, self.report_dir / f"{name}.log")
            shutil.rmtree(self.tmp, ignore_errors=True)
            self.tmp = None

    # -- waiting

    def _wait(self, condition, timeout, what, interval=0.1):
        deadline = time.monotonic() + timeout
        last_error = None
        while time.monotonic() < deadline:
            self._check_alive()
            try:
                value = condition()
                if value:
                    return value
            except (E2EError, OSError, KeyError, TypeError, IndexError, GLib.Error) as e:
                last_error = e
            time.sleep(interval)
        raise E2EError(f"timed out after {timeout}s waiting for {what}" + (f" (last error: {last_error})" if last_error else ""))

    def _wait_path(self, path, what, timeout=20):
        self._wait(lambda: Path(path).exists(), timeout, what)

    def _check_alive(self):
        for name, proc in self.procs:
            if proc.poll() is not None:
                tail = self.logs[name].read_text(errors="replace")[-1500:] if name in self.logs else ""
                raise E2EError(f"{name} exited with {proc.returncode}:\n{tail}")

    def wait_for(self, condition, what="the condition", timeout=5.0, interval=0.1):
        """Poll `condition()` until it returns something truthy; return it or fail."""
        return self._wait(condition, timeout, what, interval)

    def wait_idle(self):
        """Return once the app's main loop has nothing pending and a frame was drawn."""
        self.control.call("wait-idle")

    def settle(self, quiet=0.25, timeout=5.0):
        """Wait until the screen has not changed for `quiet` seconds (or `timeout`)."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            with self._lock:
                since = time.monotonic() - self._frame_at
            if since >= quiet:
                return
            time.sleep(0.05)

    # -- D-Bus, RemoteDesktop, ScreenCast

    def _has_name(self, name):
        reply = self.bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "NameHasOwner", GLib.Variant("(s)", (name,)), GLib.VariantType("(b)"), 0, 2000, None)
        return reply.unpack()[0]

    def _call(self, dest, path, iface, method, args, reply_type=None):
        return self.bus.call_sync(dest, path, iface, method, args, GLib.VariantType(reply_type) if reply_type else None, 0, 10000, None)

    def _start_remote_desktop(self):
        rd = "org.gnome.Mutter.RemoteDesktop"
        sc = "org.gnome.Mutter.ScreenCast"
        self.rd_session = self._call(rd, "/org/gnome/Mutter/RemoteDesktop", rd, "CreateSession", None, "(o)").unpack()[0]
        props = self._call(rd, self.rd_session, "org.freedesktop.DBus.Properties", "Get", GLib.Variant("(ss)", (rd + ".Session", "SessionId")), "(v)")
        session_id = props.unpack()[0]
        options = {"remote-desktop-session-id": GLib.Variant("s", session_id)}
        sc_session = self._call(sc, "/org/gnome/Mutter/ScreenCast", sc, "CreateSession", GLib.Variant("(a{sv})", (options,)), "(o)").unpack()[0]
        # cursor-mode 0: the pointer is not drawn, so pixel checks see only the app.
        self.stream = self._call(sc, sc_session, sc + ".Session", "RecordMonitor", GLib.Variant("(sa{sv})", ("", {"cursor-mode": GLib.Variant("u", 0)})), "(o)").unpack()[0]
        node = []
        sub = self.bus.signal_subscribe(None, sc + ".Stream", "PipeWireStreamAdded", self.stream, None, 0, lambda *a: node.append(a[5].unpack()[0]))
        self._call(rd, self.rd_session, rd + ".Session", "Start", None)
        ctx = GLib.MainContext.default()
        self._wait(lambda: (ctx.iteration(False), node)[1], 10, "the ScreenCast PipeWire stream")
        self.bus.signal_unsubscribe(sub)
        self._start_capture(node[0])
        # Headless mutter has no input devices until the first virtual event creates them,
        # and a client binds the new pointer/keyboard only after that: make both now,
        # before the app starts, so no real test event is lost.
        self.move(1, 1)
        self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS["shift"], True)
        self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS["shift"], False)

    def _start_capture(self, node_id):
        Gst.init(None)
        # Connect to the private PipeWire by file descriptor: never through the environment.
        pw = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        pw.connect(str(self.run_dir / "pipewire-0"))
        self._pw_sock = pw
        self.pipeline = Gst.parse_launch(
            f"pipewiresrc fd={pw.fileno()} target-object={node_id} always-copy=true ! videoconvert ! video/x-raw,format=RGBx ! appsink name=sink emit-signals=true max-buffers=1 drop=true sync=false"
        )
        sink = self.pipeline.get_by_name("sink")
        sink.connect("new-sample", self._on_sample)
        self.pipeline.set_state(Gst.State.PLAYING)
        self._wait(lambda: self._frame is not None, 15, "the first screen frame")

    def _on_sample(self, sink):
        sample = sink.emit("pull-sample")
        caps = sample.get_caps().get_structure(0)
        w, h = caps.get_value("width"), caps.get_value("height")
        buf = sample.get_buffer()
        ok, info = buf.map(Gst.MapFlags.READ)
        if ok:
            data = bytes(info.data)
            buf.unmap(info)
            with self._lock:
                self._frame = Image(w, h, len(data) // h, data)
                self._frames += 1
                self._frame_at = time.monotonic()
        return Gst.FlowReturn.OK

    def _rd(self, method, signature, *args):
        self._call("org.gnome.Mutter.RemoteDesktop", self.rd_session, "org.gnome.Mutter.RemoteDesktop.Session", method, GLib.Variant(signature, args))
        time.sleep(STEP_DELAY)

    # -- input (real events through the compositor)

    def move(self, x, y):
        self._rd("NotifyPointerMotionAbsolute", "(sdd)", self.stream, float(x), float(y))

    def mouse_down(self, button="left"):
        self._rd("NotifyPointerButton", "(ib)", BTN[button], True)

    def mouse_up(self, button="left"):
        self._rd("NotifyPointerButton", "(ib)", BTN[button], False)

    def click(self, x, y, button="left", count=1, mods=()):
        """Click at screen x,y. `mods` are held during the click, e.g. ("ctrl",)."""
        for m in mods:
            self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS[m], True)
        self.move(x, y)
        for _ in range(count):
            self.mouse_down(button)
            self.mouse_up(button)
        for m in reversed(mods):
            self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS[m], False)
        self.wait_idle()

    def double_click(self, x, y):
        self.click(x, y, count=2)

    def right_click(self, x, y):
        self.click(x, y, button="right")

    def drag(self, x1, y1, x2, y2, steps=25, duration=0.6, hold=0.2, mods=(), via=(), before_release=None):
        """Press at x1,y1, move in `steps` per leg to x2,y2 (through the `via` points, to
        steer around places that would refuse a drop), wait `hold` s, call
        `before_release()` if given (e.g. to take a screenshot mid-drag), release. Works
        for in-widget drags and for drag-and-drop between widgets."""
        for m in mods:
            self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS[m], True)
        self.move(x1, y1)
        self.mouse_down()
        # A first small move past GTK's drag threshold, then the path.
        points = [(x1, y1), *via, (x2, y2)]
        nx, ny = points[1]
        self.move(x1 + (6 if nx >= x1 else -6), y1 + (6 if ny >= y1 else -6))
        for (ax, ay), (bx, by) in zip(points, points[1:]):
            for i in range(1, steps + 1):
                t = i / steps
                self.move(ax + (bx - ax) * t, ay + (by - ay) * t)
                time.sleep(max(duration / steps - STEP_DELAY, 0))
        time.sleep(hold)
        if before_release:
            before_release()
        self.move(x2, y2)
        self.mouse_up()
        for m in reversed(mods):
            self._rd("NotifyKeyboardKeysym", "(ub)", MODIFIERS[m], False)
        self.wait_idle()

    def scroll(self, x, y, steps=1, horizontal=False):
        """Mouse wheel at x,y: positive steps scroll down (or right)."""
        self.move(x, y)
        self._rd("NotifyPointerAxisDiscrete", "(ui)", 1 if horizontal else 0, int(steps))
        self.wait_idle()

    def key(self, combo: str):
        """Press and release a key with modifiers: "ctrl+t", "shift+[", "alt+t", "BackSpace", "Up"."""
        # "ctrl++" is Ctrl with the + key.
        *mods, name = combo[:-2].split("+") + ["+"] if combo.endswith("++") else combo.split("+")
        held = [MODIFIERS[m.lower()] for m in mods]
        for k in held:
            self._rd("NotifyKeyboardKeysym", "(ub)", k, True)
        sym = keysym(name)
        self._rd("NotifyKeyboardKeysym", "(ub)", sym, True)
        self._rd("NotifyKeyboardKeysym", "(ub)", sym, False)
        for k in reversed(held):
            self._rd("NotifyKeyboardKeysym", "(ub)", k, False)
        self.wait_idle()

    def type_text(self, text: str):
        """Type text character by character (the compositor adds Shift where needed)."""
        for ch in text:
            sym = keysym(ch)
            self._rd("NotifyKeyboardKeysym", "(ub)", sym, True)
            self._rd("NotifyKeyboardKeysym", "(ub)", sym, False)
        self.wait_idle()

    # -- looking

    def pid(self, name):
        """Process id of a part of the session: "app", "mutter", "pipewire", "wireplumber", "dbus"."""
        return next(p.pid for n, p in self.procs if n == name)

    def note(self, key, value):
        """Record a measurement; it is written to the scenario's result.json."""
        self.notes[key] = value

    def screenshot(self, name=None) -> Image:
        """What the compositor shows now. With `name`, also saved as <report>/<name>.png."""
        self.wait_idle()
        self.settle()
        with self._lock:
            image = self._frame
        if name and self.report_dir:
            self.report_dir.mkdir(parents=True, exist_ok=True)
            path = self.report_dir / f"{name}.png"
            image.save(path)
            self.shots.append(str(path))
        return image

    def state(self):
        """Project and editing state (see `test_state` in crates/tempo-ui/src/app.rs)."""
        return self.control.call("state")

    def geometry(self):
        """Screen rectangles of widgets and timeline items (see `test_geometry`)."""
        return self.control.call("geometry")

    def action(self, step: str):
        """Setup only: run a GAction or TEMPO_ACTIONS step inside the app. Do not use it
        for what a test is meant to check; drive that with real input."""
        self.control.call(f"action {step}")
        self.wait_idle()

    # -- finding things

    def widgets(self, text=None, type=None, tooltip=None, css=None, contains=None, geometry=None):
        """Mapped widgets matching all the given filters (exact `text`, substring `contains`)."""
        found = []
        for w in (geometry or self.geometry())["widgets"]:
            if text is not None and w.get("text") != text:
                continue
            if contains is not None and contains not in (w.get("text") or ""):
                continue
            if type is not None and w["type"] != type:
                continue
            if tooltip is not None and tooltip not in (w.get("tooltip") or ""):
                continue
            if css is not None and css not in w["css"]:
                continue
            if w["rect"][2] <= 0 or w["rect"][3] <= 0:
                continue
            found.append(w)
        return found

    def widget(self, timeout=5.0, **filters):
        """The first widget matching `filters` (see `widgets`), waiting for it to appear."""
        return self.wait_for(lambda: next(iter(self.widgets(**filters)), None), f"a widget {filters}", timeout)

    def row_widgets(self, label: str, type: str, geometry=None):
        """The `type` widgets on the same row as the label `label` (Inspector, dialogs), left
        to right: right of the label, vertical middle within the label's height."""
        geo = geometry or self.geometry()
        for lab in self.widgets(text=label, type="GtkLabel", geometry=geo):
            lx, ly, lw, lh = lab["rect"]
            cy = ly + lh / 2
            found = [w for w in self.widgets(type=type, geometry=geo) if w["rect"][0] >= lx + lw - 2 and abs(w["rect"][1] + w["rect"][3] / 2 - cy) <= max(lh, 14)]
            if found:
                return sorted(found, key=lambda w: w["rect"][0])
        return []

    def row_widget(self, label: str, type: str, index=0, timeout=5.0):
        """The index-th `type` widget on the row of `label` (see `row_widgets`), scrolled
        into view, waiting for it to appear."""
        def find():
            found = self.row_widgets(label, type)
            return found[index] if len(found) > index else None
        self.wait_for(find, f"a {type} on the row of {label!r}", timeout)
        return self.scroll_into_view(find)

    def scroll_into_view(self, find, attempts=20):
        """Scroll the wheel over the enclosing GtkScrolledWindow until `find()`'s widget is
        fully inside it; return the widget as it is then."""
        for _ in range(attempts):
            geo = self.geometry()
            w = find()
            sw = self.ancestor(next((g for g in geo["widgets"] if g["id"] == w["id"] and g["type"] == w["type"]), w), "GtkScrolledWindow", geo)
            if sw is None:
                return w
            sx, sy, swid, sh = sw["rect"]
            x, y, wid, h = w["rect"]
            if y >= sy and y + h <= sy + sh:
                return w
            self.scroll(sx + swid / 2, sy + sh / 2, 2 if y + h > sy + sh else -2)
            self.settle(quiet=0.15)
        raise E2EError(f"could not scroll {w.get('type')} {w.get('text')!r} into view")

    def ancestor(self, widget, type, geometry):
        """The nearest enclosing widget of `type` (e.g. the button around a label)."""
        items = geometry["widgets"]
        parent = widget.get("parent")
        while parent is not None:
            if items[parent]["type"] == type:
                return items[parent]
            parent = items[parent].get("parent")
        return None

    def descendants(self, widget, type=None, geometry=None):
        """Widgets inside `widget` (optionally only of `type`)."""
        geo = geometry or self.geometry()
        inside, out = {widget["id"]}, []
        for w in geo["widgets"]:
            if w.get("parent") in inside:
                inside.add(w["id"])
                if type is None or w["type"] == type:
                    out.append(w)
        return out

    def fill(self, widget, text, enter=True):
        """Click into a text field (an Entry, a SpinButton's text, ...), select all, type
        `text`, and press Return. Clicks the GtkText part, never a +/- button."""
        if widget["type"] != "GtkText":
            texts = self.descendants(widget, "GtkText")
            widget = texts[0] if texts else widget
        self.click(*center(widget["rect"]))
        self.key("ctrl+a")
        self.type_text(text)
        if enter:
            self.key("Return")

    def open_panel(self, label):
        """Click a top-bar panel toggle ("Media Pool", "Effects", "Inspector") unless it is on."""
        def toggle():
            geo = self.geometry()
            for lab in self.widgets(text=label, type="GtkLabel", geometry=geo):
                button = self.ancestor(lab, "GtkToggleButton", geo)
                if button:
                    return button
            raise E2EError(f"no panel toggle labelled {label!r}")

        if not toggle()["active"]:
            self.click(*center(toggle()["rect"]))
            self.wait_for(lambda: toggle()["active"], f"the {label} panel to open")
            self.settle()

    def timeline(self):
        return self.state()["project"]["timeline"]

    def tracks(self):
        return self.timeline()["tracks"]

    def track_name(self, track):
        return ("V" if track["kind"] == "Video" else "A") + str(track["kind_index"])

    def track(self, name):
        return next((t for t in self.tracks() if self.track_name(t) == name), None)

    def transitions(self, track="V1"):
        t = self.track(track)
        return t["transitions"] if t else []

    def clip(self, track="V1", index=0, geometry=None):
        """Geometry entry of the index-th clip on a track, left to right."""
        clips = sorted((c for c in (geometry or self.geometry())["timeline"]["clips"] if c["track"] == track), key=lambda c: c["in_us"])
        return clips[index]

    def transition_geo(self, track="V1", index=0):
        items = sorted((t for t in self.geometry()["timeline"]["transitions"] if t["track"] == track), key=lambda t: t["start_us"])
        return items[index]

    def cut_point(self, track="V1", index=0):
        """Screen point of the index-th cut between two clips on a track, at mid-row height."""
        geo = self.geometry()
        a, b = self.clip(track, index, geo), self.clip(track, index + 1, geo)
        x, y = b["rect"][0], a["rect"][1] + a["rect"][3] / 2
        cx, cy, cw, ch = geo["timeline"]["canvas"]
        if not (cx <= x <= cx + cw):
            raise E2EError(f"cut {index} on {track} is off-screen at x={x:.0f}; call fit_timeline() first")
        return (x, y)

    def fit_timeline(self):
        """Shift+Z: zoom the timeline to show everything (needed after panels change size)."""
        self.key("shift+z")
        self.settle()


# ---- Assertions ------------------------------------------------------------------------------


def expect(condition, message):
    if not condition:
        raise E2EError(message)


def expect_eq(actual, expected, what):
    if actual != expected:
        raise E2EError(f"{what}: expected {expected!r}, got {actual!r}")


def expect_near(actual, expected, tolerance, what):
    if abs(actual - expected) > tolerance:
        raise E2EError(f"{what}: expected {expected} ± {tolerance}, got {actual}")


# ---- Runner ---------------------------------------------------------------------------------


def load_scenario(spec: str):
    path = Path(spec)
    if not path.exists():
        matches = sorted((HERE / "scenarios").glob(f"*{spec}*.py"))
        if not matches:
            raise SystemExit(f"no scenario matches {spec!r}")
        path = matches[0]
    module_spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(module_spec)
    module_spec.loader.exec_module(module)
    return path.stem, module


def run_scenarios(specs, project, report, app=None):
    report.mkdir(parents=True, exist_ok=True)
    if not specs:
        specs = [str(p) for p in sorted((HERE / "scenarios").glob("[0-9]*.py"))]
    results = []
    for spec in specs:
        name, module = load_scenario(spec)
        out = report / name
        out.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        result = {"scenario": name, "description": (module.__doc__ or "").strip().splitlines()[0] if module.__doc__ else "", "passed": False}
        print(f"[ RUN  ] {name}", flush=True)
        try:
            with Session(project=getattr(module, "PROJECT", project), report_dir=out, app=app) as session:
                try:
                    module.run(session)
                except BaseException:
                    result["notes"] = session.notes
                    try:
                        session.screenshot("failure")
                    except Exception:  # noqa: BLE001 - keep the original error
                        pass
                    raise
                result["notes"] = session.notes
                result["passed"] = True
        except BaseException as e:  # noqa: BLE001 - a scenario failure must never stop the run
            result["error"] = f"{type(e).__name__}: {e}"
            result["traceback"] = traceback.format_exc()
            if isinstance(e, KeyboardInterrupt):
                results.append(result)
                break
        result["seconds"] = round(time.monotonic() - started, 1)
        result["screenshots"] = sorted(str(p) for p in out.glob("*.png"))
        result["report_dir"] = str(out)
        (out / "result.json").write_text(json.dumps(result, indent=2))
        results.append(result)
        print(f"[ {'PASS' if result['passed'] else 'FAIL'} ] {name} ({result['seconds']} s)" + ("" if result["passed"] else f"\n         {result['error']}"), flush=True)
    passed = sum(r["passed"] for r in results)
    summary = {"passed": passed, "total": len(results), "results": results}
    (report / "summary.json").write_text(json.dumps(summary, indent=2))
    print(f"\n{passed}/{len(results)} scenarios passed. Report: {report}")
    return passed == len(results)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    run = sub.add_parser("run", help="run scenarios (all by default)")
    run.add_argument("scenarios", nargs="*", help="scenario files or name fragments (e.g. transitions)")
    run.add_argument("--project", default=os.environ.get("TEMPO_E2E_PROJECT"), help="the fixture .tempo to open")
    run.add_argument("--report", default=str(REPO / "target" / "e2e-report"), help="where results and screenshots go")
    run.add_argument("--app", default=None, help="the tempo binary (default target/e2e-bin/tempo)")
    sub.add_parser("list", help="list scenarios")
    args = parser.parse_args()
    if args.command == "list":
        for p in sorted((HERE / "scenarios").glob("[0-9]*.py")):
            print(p.stem)
        return 0
    return 0 if run_scenarios(args.scenarios, args.project, Path(args.report), args.app) else 1


if __name__ == "__main__":
    sys.exit(main())
