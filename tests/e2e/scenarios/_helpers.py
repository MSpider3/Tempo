"""Steps several scenarios share. Not a scenario (no number prefix)."""

import re
import subprocess

from tempo_e2e import center

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


def select_cut(s, index, track="V1"):
    """Click the index-th cut low in the row: selects the edit point there."""
    x, _ = s.cut_point(track, index)
    s.click(x, s.clip(track, index)["edit_y"])
    s.wait_for(lambda: (s.state()["edit_selection"] or {}).get("kind") == "edit", f"cut {index} to be selected")


def add_transition_at_cut(s, index):
    """Ctrl+T on a cut, then click the new V1 transition so the Inspector shows it."""
    before = len(s.transitions("V1"))
    select_cut(s, index)
    s.key("ctrl+t")
    s.wait_for(lambda: len(s.transitions("V1")) > before, "Ctrl+T to add a transition")
    spans = sorted(s.geometry()["timeline"]["transitions"], key=lambda g: abs((g["start_us"] + g["end_us"]) / 2 - s.clip("V1", index + 1)["in_us"]))
    s.click(*center(spans[0]["rect"]))
    s.wait_for(lambda: s.state()["selected_transition"], "the transition to be selected")


def seek_by_ruler(s, seconds):
    """Click the ruler at `seconds` (measured from the first clip); returns the playhead."""
    geo = s.geometry()
    first = s.clip("V1", 0, geo)
    ruler = geo["timeline"]["ruler"]
    s.click(first["rect"][0] + seconds * geo["timeline"]["pps"], ruler[1] + ruler[3] / 2)
    return s.wait_for(lambda: abs(s.state()["playhead_us"] - seconds * 1e6) < 60_000 and s.state()["playhead_us"], f"the playhead near {seconds} s")


def viewer_colour(s, name):
    """Mean colour of the middle of the viewer picture, once the screen is still."""
    surface = s.widget(type="TempoVideoSurface")
    cx, cy = center(surface["rect"])
    return s.screenshot(name).mean([cx - 20, cy - 20, 40, 40])


GRAPH_PAD = 10.0  # GRAPH_PAD in crates/tempo-ui/src/inspector.rs


def graph(s):
    """The Inspector's transition curve graph, scrolled into view."""
    find = lambda: next(iter(s.widgets(type="GtkDrawingArea", tooltip="Double-click", hidden=True)), None)  # noqa: E731
    s.wait_for(find, "the curve graph")
    return s.scroll_into_view(find)


def point(rect, u, v):
    """Screen point of curve coordinates (u along, v up) in the graph `rect`."""
    x, y, w, h = rect
    return (x + GRAPH_PAD + u * (w - 2 * GRAPH_PAD), y + GRAPH_PAD + (1 - v) * (h - 2 * GRAPH_PAD))


def curve(s, track="V1"):
    """The keys of the first transition on `track`."""
    return s.transitions(track)[0]["curve"]
