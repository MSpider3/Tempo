"""Isolation: the harness, the app and the compositor talk only to the session's private Wayland, D-Bus and PipeWire sockets, and use a private HOME."""

import os

from _helpers import SYSTEM_SOCKETS, environ, unix_peers
from tempo_e2e import expect, expect_eq

def run(s):
    private = str(s.tmp)
    uid = os.getuid()
    for name in ("app", "mutter", "pipewire", "wireplumber"):
        env = environ(s.pid(name))
        expect(env["XDG_RUNTIME_DIR"].startswith(private), f"{name} uses the private runtime dir")
        expect(env["HOME"].startswith(private), f"{name} uses the private HOME")
        expect(env["DBUS_SESSION_BUS_ADDRESS"] == f"unix:path={s.run_dir}/bus", f"{name} uses the private session bus")
        expect("DISPLAY" not in env, f"{name} has no X11 DISPLAY")
    app_env = environ(s.pid("app"))
    expect_eq(app_env["WAYLAND_DISPLAY"], s.display, "the app's Wayland display")
    expect_eq(app_env["GDK_BACKEND"], "wayland", "the app's GDK backend")
    expect_eq(app_env.get("TEMPO_NO_AUDIO"), "1", "the app opens no sound output")

    allowed = tuple(p.format(uid=uid) for p in SYSTEM_SOCKETS)
    for name in ("app", "mutter"):
        peers = unix_peers(s.pid(name))
        s.note(f"{name}_unix_peers", peers)
        outside = [p for p in peers if not p.startswith(private) and not p.startswith(allowed)]
        expect(not outside, f"{name} talks only to private sockets, not {outside}")
    # The harness process itself: its session variables point at the private session
    # while it runs, and it is connected to private sockets only.
    for var in ("DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR", "HOME"):
        expect(os.environ.get(var, "").replace("unix:path=", "").startswith(private), f"the harness's {var} is private: {os.environ.get(var)}")
    expect("WAYLAND_DISPLAY" not in os.environ and "DISPLAY" not in os.environ, "the harness has no desktop display variables")
    peers = unix_peers(os.getpid())
    s.note("harness_unix_peers", peers)
    outside = [p for p in peers if not p.startswith(private) and not p.startswith(allowed)]
    expect(not outside, f"the harness talks only to private sockets, not {outside}")
    expect(f"{s.run_dir}/pipewire-0" in peers, f"the harness captures the screen from the private PipeWire: {peers}")
    app_peers = unix_peers(s.pid("app"))
    expect(f"{s.run_dir}/{s.display}" in app_peers, f"the app is connected to the private compositor: {app_peers}")
