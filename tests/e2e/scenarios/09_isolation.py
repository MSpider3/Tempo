"""Isolation: the app and compositor talk only to the session's private Wayland, D-Bus and PipeWire sockets, and use a private HOME."""

import os
import re
import subprocess

from tempo_e2e import expect, expect_eq

# Read-only system services a GTK app may reach; none of them is the user's desktop.
SYSTEM_SOCKETS = ("/run/dbus/system_bus_socket", "/run/systemd/", "/var/run/dbus/", "/run/user/{uid}/systemd/journal")


def environ(pid):
    raw = open(f"/proc/{pid}/environ", "rb").read().split(b"\0")
    return dict(item.decode(errors="replace").split("=", 1) for item in raw if b"=" in item)


def unix_peers(pid):
    """Paths of the Unix sockets the process is connected to (from `ss`)."""
    out = subprocess.run(["ss", "-xpn"], capture_output=True, text=True, check=True).stdout
    path_of, mine = {}, []
    for line in out.splitlines()[1:]:
        cols = line.split()
        if len(cols) < 8:
            continue
        local, local_inode, peer_inode = cols[4], cols[5], cols[7]
        if local != "*":
            path_of[local_inode] = local
        if re.search(rf"pid={pid},", line):
            mine.append(peer_inode)
    return sorted({path_of[i] for i in mine if i in path_of})


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
    app_peers = unix_peers(s.pid("app"))
    expect(f"{s.run_dir}/{s.display}" in app_peers, f"the app is connected to the private compositor: {app_peers}")
