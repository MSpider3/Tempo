"""Deliver page: the upload fields (Title, Description, Chapters, Thumbnail, Privacy, Category), a job rendered to a file that plays, and the upload stopping at the token prompt with no network."""

import json
import subprocess

from tempo_e2e import center, expect, expect_eq

# A stand-in for a YouTube uploader: no network permission, and a script that fails if it ever runs.
DATA_FILES = {
    "tempo/plugins/test.youtube/plugin.toml": '[plugin]\nid = "test.youtube"\nname = "YouTube"\nversion = "1.0.0"\n\n[[uploader]]\nid = "send"\nname = "YouTube"\nfunction = "send"\n',
    "tempo/plugins/test.youtube/main.lua": 'function send(video) error("e2e: the upload script must not run without a token") end\n',
}
TITLE, DESCRIPTION = "E2E title", "E2E description"


def label(s, text):
    return s.widget(text=text, type="GtkLabel")


def checkbox(s, text):
    geo = s.geometry()
    return s.ancestor(s.widgets(text=text, type="GtkLabel", geometry=geo)[0], "GtkCheckButton", geo)


def tcp_peers(pid):
    out = subprocess.run(["ss", "-tnp"], capture_output=True, text=True, check=True).stdout
    return [line for line in out.splitlines() if f"pid={pid}," in line]


def run(s):
    s.click(*center(s.widget(tooltip="Export page")["rect"]))
    s.wait_for(lambda: s.widgets(contains="Render Settings"), "the Export page")
    s.screenshot("0_export")
    for text in ("File Name", "Location", "Quality"):
        label(s, text)
    expect(checkbox(s, "Chapters from markers")["active"], "Chapters from markers is on by default")
    # Resolve's Deliver page filters chapters by marker colour; Tempo's checkbox has no such filter (yet).
    s.note("chapter_colour_filter", bool(s.widgets(contains="colour") or s.widgets(contains="color")))

    # The upload section shows because an enabled plugin offers an uploader.
    box = checkbox(s, "Upload directly to YouTube")
    expect(not box["active"], "Upload directly to YouTube starts off")
    expect(not s.widgets(text="Privacy", type="GtkLabel"), "the upload fields are hidden until it is ticked")
    s.click(*center(box["rect"]))
    s.wait_for(lambda: s.widgets(text="Privacy", type="GtkLabel"), "the upload fields")
    for text in ("Title", "Description", "Thumbnail", "Privacy", "Category"):
        label(s, text)
    title = s.row_widget("Title", "GtkEntry")
    s.note("default_upload_title", title["text"])  # empty: the job then takes the timeline name
    thumb_on = checkbox(s, "Upload Thumbnail")
    expect(not thumb_on["active"], "Upload Thumbnail starts off")
    path = s.row_widget("Thumbnail", "GtkEntry")
    expect(not path["sensitive"], "the thumbnail path row is disabled until Upload Thumbnail is ticked")
    s.click(*center(thumb_on["rect"]))
    s.wait_for(lambda: s.row_widget("Thumbnail", "GtkEntry")["sensitive"], "the thumbnail path row to enable")
    s.screenshot("1_fields")
    s.click(*center(checkbox(s, "Upload Thumbnail")["rect"]))  # off again: there is no image to pick
    s.wait_for(lambda: not checkbox(s, "Upload Thumbnail")["active"], "Upload Thumbnail to be unticked")

    # Per-job title and description.
    s.fill(s.row_widget("Title", "GtkEntry"), TITLE, enter=False)
    desc = s.widget(type="GtkTextView")
    s.click(*center(desc["rect"]))
    s.type_text(DESCRIPTION)

    # Render to a file in the session dir.
    out_dir = s.tmp / "home" / "renders"
    out_dir.mkdir()
    s.fill(s.row_widget("Location", "GtkEntry"), str(out_dir))
    s.click(*center(s.widget(text="Add to Render Queue")["rect"]))
    line = s.wait_for(lambda: next(iter(s.widgets(contains="Will upload to YouTube")), None), "the job in the queue with its upload line")
    tip = line.get("tooltip") or ""
    expect("Privacy: private" in tip and "Category: Film & Animation" in tip, f"the job's upload settings: {tip!r}")
    s.screenshot("2_queued")
    s.click(*center(s.widget(text="Render All")["rect"]))
    s.wait_for(lambda: s.widgets(text="✓ Completed"), "the render to finish", timeout=240)
    files = sorted(out_dir.glob("*.mp4"))
    expect_eq(len(files), 1, "rendered files")
    out = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "json", str(files[0])], capture_output=True, text=True, check=True).stdout
    seconds = float(json.loads(out)["format"]["duration"])
    s.note("rendered", {"file": files[0].name, "duration_s": seconds})
    expect(11.0 < seconds < 13.0, f"the rendered file plays for the timeline's 12 s: {seconds}")

    # The upload step stops at the token prompt: the plugin never runs and no connection is made.
    s.wait_for(lambda: s.widgets(contains="needs a token"), "the upload to ask for a token")
    s.wait_for(lambda: s.widgets(text="Access token"), "the upload dialog with its Access token field")
    expect(s.widgets(contains="Upload to YouTube"), "the dialog is the YouTube one")
    expect(not s.widgets(contains="failed"), "the upload script did not run")
    expect(TITLE in [w["text"] for w in s.widgets(type="GtkEntry")], "the dialog carries the job's title")
    s.screenshot("3_token_prompt")
    expect_eq(tcp_peers(s.pid("app")), [], "the app's TCP connections")
    s.click(*center(s.widget(text="Cancel")["rect"]))
    s.wait_for(lambda: not s.widgets(text="Access token"), "the dialog to close")
    expect_eq(tcp_peers(s.pid("app")), [], "the app's TCP connections after cancelling")
