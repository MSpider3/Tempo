"""Finding things on screen for tempo_e2e.Session: widgets by label, rows, timeline items.
Split out of tempo_e2e.py; see docs/TESTING_E2E.md for the API."""


class E2EError(AssertionError):
    """A failed expectation or a harness step that did not happen in time."""


# Widgets that take the mouse wheel for themselves.
CONTROLS = ("GtkScale", "GtkSpinButton", "GtkDropDown", "GtkEntry", "GtkButton", "GtkToggleButton", "GtkSwitch", "GtkCheckButton", "GtkDrawingArea", "GtkTextView")


def center(rect):
    """Middle of an [x, y, w, h] rectangle."""
    x, y, w, h = rect
    return (x + w / 2.0, y + h / 2.0)


class FindMixin:
    """Needs `geometry()`, `state()`, `wait_for()`, `click()`, `key()`, `type_text()`,
    `scroll()` and `settle()` from Session."""

    def widgets(self, text=None, type=None, tooltip=None, css=None, contains=None, geometry=None, hidden=False):
        """Mapped widgets matching all the given filters (exact `text`, substring `contains`).
        Widgets scrolled out of their scrolled area are left out unless `hidden`."""
        found = []
        for w in (geometry or self.geometry())["widgets"]:
            if not hidden and not w.get("visible", True):
                continue
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
        for lab in self.widgets(text=label, type="GtkLabel", geometry=geo, hidden=True):
            lx, ly, lw, lh = lab["rect"]
            cy = ly + lh / 2
            found = [w for w in self.widgets(type=type, geometry=geo, hidden=True) if w["rect"][0] >= lx + lw - 2 and abs(w["rect"][1] + w["rect"][3] / 2 - cy) <= max(lh, 14)]
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

    def scroll_into_view(self, find, attempts=25):
        """Wheel the enclosing GtkScrolledWindow until `find()`'s widget is fully inside it;
        return the widget as it is then. The wheel is turned over a plain label inside the
        scrolled area, never over a slider or field (those would change their value)."""
        for _ in range(attempts):
            geo = self.geometry()
            w = find()
            mine = next((g for g in geo["widgets"] if g["id"] == w["id"] and g["type"] == w["type"]), w)
            sw = self.ancestor(mine, "GtkScrolledWindow", geo)
            if sw is None:
                return w
            sx, sy, swid, sh = sw["rect"]
            x, y, wid, h = w["rect"]
            if y >= sy and y + h <= sy + sh:
                return w
            spot = self._wheel_spot(sw, geo)
            if spot is None:
                raise E2EError(f"no plain label inside the scrolled area to scroll over, for {w.get('type')} {w.get('text')!r}")
            self.scroll(*spot, 1 if y + h > sy + sh else -1)
            self.settle(quiet=0.15)
        raise E2EError(f"could not scroll {w.get('type')} {w.get('text')!r} into view")

    def _wheel_spot(self, sw, geo):
        """Middle of a fully visible label in `sw` that is not part of a control."""
        sx, sy, swid, sh = sw["rect"]
        spots = []
        for lab in self.descendants(sw, "GtkLabel", geo):
            x, y, w, h = lab["rect"]
            if w <= 0 or y < sy or y + h > sy + sh or x < sx or x + w > sx + swid:
                continue
            if any(self.ancestor(lab, t, geo) for t in CONTROLS):
                continue
            spots.append((abs(y + h / 2 - (sy + sh / 2)), (x + w / 2, y + h / 2)))
        return min(spots)[1] if spots else None

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
