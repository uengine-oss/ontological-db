#!/usr/bin/env python3
"""Shared machinery for the demo videos: the Studio, driven for real, with captions.

A demo is a `story(d)` function that types statements into the Studio's editor
and says, in captions, what is happening. This module does the rest:

- creates the demo database empty, with the extension, before recording;
- draws captions and title cards *in the page*, so they are in the picture, and
  writes the same lines as WebVTT and SRT beside the video;
- checks, after every statement, that the result frame shows what the caption
  is about to claim — and stops the recording if it does not;
- holds scenes on this process's clock, never the page's (a headless page's
  timers are throttled; a hold asked of the page can last many times longer);
- prints each scene's length, and converts the recording to H.264 MP4.

    from demo_lib import record

    def story(d):
        d.scene("title")
        d.card("<h1>…<em>…</em></h1><p>…</p>", 6, "spoken form of the card")
        d.scene("first")
        d.caption("무엇을 하려는지.", read=False)        # stays up while the statement is typed
        d.run("MATCH (n) RETURN count(n) AS n", expect="3", look=2)
        d.caption("결과가 무엇을 뜻하는지.")              # held long enough to read
        d.scene(None)

    record("my-feature", story)   # → docs/tutorials/media/my-feature-demo.mp4 (+ .vtt .srt .timing.json)

`FAST=1` runs the same checks with short holds and keeps no file — use it until
the story passes. `d.run(statement, expect=…, error=False, look=…)` accepts any
editor input: Cypher, a fabric statement, or a Studio command such as
`:sql SELECT …` (the Studio must be started with OG_STUDIO_ALLOW_RAW_SQL=1 for
that one) or `:param name => value`. `d.clear()` empties the result stream.
`d.page` is the Playwright page, for the parts of the Studio that are not the
editor. A page reload loses the overlay; call `d.overlay()` after one.

Env: TUT_STUDIO (the Studio's address; it must be running against the demo
database), TUT_EXEC (default "docker exec -i og-fabric"), FAST.
"""
import json, os, shlex, shutil, subprocess, sys, tempfile, time
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
MEDIA = ROOT / "docs" / "tutorials" / "media"
EXEC = shlex.split(os.environ.get("TUT_EXEC", "docker exec -i og-fabric"))
FAST = os.environ.get("FAST") == "1"
W, H = 1280, 720

CAPTION_CSS = """
#demo-caption { position:fixed; left:50%; bottom:26px; transform:translateX(-50%); z-index:99999;
  max-width:88%; padding:13px 26px; border-radius:14px; pointer-events:none; text-align:center;
  background:rgba(7,11,15,.90); border:1px solid rgba(16,185,129,.55); box-shadow:0 12px 40px rgba(0,0,0,.55);
  color:#f2f6f8; font:600 21px/1.5 -apple-system,"Apple SD Gothic Neo","Noto Sans KR",sans-serif; letter-spacing:-.01em;
  opacity:0; transition:opacity .25s; }
#demo-caption.on { opacity:1; }
#demo-card { position:fixed; inset:0; z-index:99998; display:none; align-items:center; justify-content:center;
  flex-direction:column; gap:18px; background:#0b1015; color:#eef3f6; text-align:center; padding:0 9%;
  font-family:-apple-system,"Apple SD Gothic Neo","Noto Sans KR",sans-serif; }
#demo-card.on { display:flex; }
#demo-card h1 { margin:0; font-size:40px; line-height:1.35; letter-spacing:-.02em; }
#demo-card h1 em { font-style:normal; color:#10b981; }
#demo-card p { margin:0; font-size:21px; line-height:1.6; color:#9fb0bd; }
#demo-card .row { display:flex; gap:14px; margin-top:10px; }
#demo-card .chip { padding:9px 18px; border:1px solid rgba(160,174,192,.3); border-radius:999px; font:600 16px ui-monospace,Menlo,monospace; color:#cfd9e1; }
#demo-card .chip b { color:#10b981; font-weight:600; }
"""


class Stop(Exception):
    pass


class Demo:
    def __init__(self, page):
        self.page = page
        self.t0 = time.monotonic()
        self.cues = []       # (start, end, text)
        self.scenes = []     # (name, seconds)
        self._cue = None
        self._scene = None

    # --- time ---------------------------------------------------------------
    def now(self):
        return time.monotonic() - self.t0

    def hold(self, seconds):
        time.sleep(0.25 if FAST else seconds)

    def scene(self, name):
        if self._scene:
            self.scenes.append((self._scene[0], self.now() - self._scene[1]))
        self._scene = (name, self.now()) if name else None

    # --- captions -----------------------------------------------------------
    def overlay(self):
        self.page.add_style_tag(content=CAPTION_CSS)
        self.page.evaluate("""() => {
            for (const id of ['demo-caption', 'demo-card']) {
              if (!document.getElementById(id)) {
                const d = document.createElement('div'); d.id = id; document.body.appendChild(d);
              }
            }
        }""")

    def caption(self, text, read=None):
        """Show a caption and give it time to be read."""
        self._end_cue()
        self.page.evaluate("t => { const c = document.getElementById('demo-caption'); c.textContent = t; c.classList.add('on'); }", text)
        self._cue = (self.now(), text)
        if read is not False:
            self.hold(read if read else max(3.2, len(text) / 8.5))

    def _end_cue(self):
        if self._cue:
            self.cues.append((self._cue[0], self.now(), self._cue[1]))
            self._cue = None

    def quiet(self):
        self._end_cue()
        self.page.evaluate("() => document.getElementById('demo-caption').classList.remove('on')")

    def card(self, html, seconds, spoken):
        self.quiet()
        self.page.evaluate("h => { const c = document.getElementById('demo-card'); c.innerHTML = h; c.classList.add('on'); }", html)
        start = self.now()
        self.hold(seconds)
        self.cues.append((start, self.now(), spoken))
        self.page.evaluate("() => document.getElementById('demo-card').classList.remove('on')")

    # --- the Studio ---------------------------------------------------------
    def frames(self):
        return self.page.locator(".frame").count()

    def run(self, statement, expect=None, error=False, look=1.6, slow=True):
        """Type a statement into the editor, run it, and check the frame it produced."""
        page = self.page
        before = self.frames()
        page.click("#editor")
        if slow and not FAST:
            page.keyboard.type(statement, delay=7 if len(statement) > 120 else 14)
            time.sleep(0.5)
        else:
            page.fill("#editor", statement)
        page.keyboard.press("Control+Enter")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.frames() > before and page.locator(".frame").first.locator(".tab").count():
                break
            time.sleep(0.1)
        else:
            raise Stop(f"no result frame for: {statement[:70]}")
        frame = page.locator(".frame").first
        tabs = " ".join(frame.locator(".tab").all_inner_texts())
        text = frame.locator(".body").inner_text()
        failed = "Error" in tabs
        if failed != error:
            raise Stop(f"{'expected an error' if error else 'unexpected error'} for: {statement[:70]}\n{text[:300]}")
        for want in ([expect] if isinstance(expect, str) else expect or []):
            if want not in text:
                raise Stop(f"the result of [{statement[:60]}…] does not show '{want}':\n{text[:400]}")
        self.hold(look)
        return text

    def clear(self):
        self.page.fill("#editor", ":clear")
        self.page.keyboard.press("Control+Enter")
        time.sleep(0.3)


def admin(sql, db="postgres"):
    p = subprocess.run(EXEC + ["psql", "-X", "-q", "-h", "localhost", "-p", "28816", "-d", db],
                       input=sql, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(p.stderr)


def stamp(t, sep):
    h, rem = divmod(t, 3600)
    m, s = divmod(rem, 60)
    return f"{int(h):02d}:{int(m):02d}:{int(s):02d}{sep}{int(round((s % 1) * 1000)):03d}"


def record(name, story, db=None, studio=None, setup_sql=None):
    """Create the database, drive the Studio through `story`, and write the video."""
    db = db or "demo_" + name.replace("-", "_")
    studio = (studio or os.environ.get("TUT_STUDIO", "http://127.0.0.1:7480")).rstrip("/")
    admin(f"DROP DATABASE IF EXISTS {db} WITH (FORCE);\nCREATE DATABASE {db};")
    admin("CREATE EXTENSION ontological CASCADE;", db=db)
    if setup_sql:
        admin(setup_sql, db=db)
    work = Path(tempfile.mkdtemp(prefix="fabric-demo-"))

    with sync_playwright() as p:
        browser = p.chromium.launch()
        context = browser.new_context(viewport={"width": W, "height": H}, device_scale_factor=1,
                                      record_video_dir=str(work), record_video_size={"width": W, "height": H},
                                      color_scheme="dark")
        page = context.new_page()
        problems = []
        page.on("pageerror", lambda e: problems.append(str(e)))
        # The Studio's pool may hold connections to the database that was just dropped.
        for _ in range(15):
            try:
                page.goto(studio + "/")
                page.wait_for_selector("#editor", timeout=5000)
                break
            except Exception:
                time.sleep(1)
        else:
            sys.exit(f"the Studio at {studio} is not answering")
        d = Demo(page)
        d.overlay()
        try:
            story(d)
        except Stop as e:
            context.close()
            browser.close()
            sys.exit(f"\nrecording stopped — the screen was not what the caption says:\n{e}")
        total = d.now()
        context.close()
        raw = Path(page.video.path())
        browser.close()

    if problems:
        sys.exit("the Studio page raised errors: " + "; ".join(problems))

    print(f"{'scene':<24}{'seconds':>8}")
    for scene_name, sec in d.scenes:
        print(f"{scene_name:<24}{sec:>8.1f}")
    print(f"{'total':<24}{total:>8.1f}   {len(d.cues)} captions")
    if FAST:
        print("FAST run — checks passed, nothing kept")
        shutil.rmtree(work, ignore_errors=True)
        return

    MEDIA.mkdir(parents=True, exist_ok=True)
    out = MEDIA / f"{name}-demo.mp4"
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-i", str(raw), "-c:v", "libx264", "-preset", "slow",
                    "-crf", "23", "-pix_fmt", "yuv420p", "-r", "25", "-movflags", "+faststart", "-an", str(out)], check=True)
    # The title card, as the still a player shows before it is played.
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", "3", "-i", str(out), "-frames:v", "1",
                    "-q:v", "3", str(MEDIA / f"{name}-demo.jpg")], check=True)
    vtt = ["WEBVTT", ""]
    srt = []
    for i, (a, b, text) in enumerate(d.cues, 1):
        vtt += [f"{stamp(a, '.')} --> {stamp(b, '.')}", text, ""]
        srt += [str(i), f"{stamp(a, ',')} --> {stamp(b, ',')}", text, ""]
    (MEDIA / f"{name}-demo.vtt").write_text("\n".join(vtt))
    (MEDIA / f"{name}-demo.srt").write_text("\n".join(srt))
    (MEDIA / f"{name}-demo.timing.json").write_text(json.dumps(
        {"total": round(total, 1), "scenes": [[n, round(s, 1)] for n, s in d.scenes],
         "captions": [[round(a, 2), round(b, 2), t] for a, b, t in d.cues]}, ensure_ascii=False, indent=1))
    shutil.rmtree(work, ignore_errors=True)
    print(f"wrote {out.relative_to(ROOT)} ({out.stat().st_size / 1e6:.1f} MB) with .vtt and .srt")


