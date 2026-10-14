#!/usr/bin/env python3
"""Export tutorials as files to hand around: a PDF and a Word document each.

    python3 web/build_docs.py                              # the site pages are the PDFs' source
    python3 examples/tutorials/export_docs.py [name …]     # default: every docs/tutorials/*-tutorial.md
    python3 web/build_docs.py                              # so the site carries the files

The PDF is the site's page printed by a browser with its print stylesheet —
diagrams drawn, Korean set properly, results boxed as on screen. The Word
document is converted from the Markdown; a Mermaid diagram becomes a picture
taken from the same page, since Word cannot draw one.
"""
import re, subprocess, sys, tempfile
from pathlib import Path
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
TUT = ROOT / "docs" / "tutorials"
SITE = ROOT / "web" / "docs" / "tutorials"
REFERENCE = Path(__file__).parent / "reference.docx"

names = sys.argv[1:] or sorted(p.name[: -len("-tutorial.md")] for p in TUT.glob("*-tutorial.md"))

with sync_playwright() as p:
    browser = p.chromium.launch()
    for name in names:
        md_path = TUT / f"{name}-tutorial.md"
        page_path = SITE / f"{name}-tutorial.html"
        if not page_path.exists():
            sys.exit(f"{page_path.relative_to(ROOT)} is missing — build the site first: python3 web/build_docs.py")
        md = md_path.read_text()
        title = re.sub(r"^튜토리얼:\s*", "", re.search(r"^#\s+(.+)$", md, re.M).group(1))

        page = browser.new_page(viewport={"width": 1100, "height": 900}, color_scheme="light", device_scale_factor=2)
        page.goto(page_path.as_uri())
        page.wait_for_function("document.documentElement.dataset.diagrams === 'ready'", timeout=60000)
        diagrams = []
        for i, svg in enumerate(page.locator("pre.mermaid svg").all(), 1):
            out = TUT / "images" / f"{name}-diagram-{i}.png"
            out.parent.mkdir(exist_ok=True)
            svg.screenshot(path=str(out))
            diagrams.append(out.relative_to(TUT).as_posix())
        page.emulate_media(media="print")
        page.pdf(path=str(TUT / f"{name}-tutorial.pdf"), format="A4", print_background=True,
                 margin={"top": "16mm", "bottom": "16mm", "left": "15mm", "right": "15mm"},
                 display_header_footer=True, header_template="<span></span>",
                 footer_template='<div style="width:100%;font-size:8px;color:#778;padding:0 15mm;display:flex;justify-content:space-between">'
                                 f'<span>Ontological DB — {title}</span>'
                                 '<span><span class="pageNumber"></span> / <span class="totalPages"></span></span></div>')
        page.close()

        it = iter(diagrams)
        md = re.sub(r"```mermaid\n.*?\n```", lambda m: f"![그림]({next(it)})", md, flags=re.S)
        # A document on paper has no player and needs no link to itself.
        md = re.sub(r"\[(데모 영상[^\]]*)\]\(media/[^)]+\)", r"*\1 — 문서 사이트의 이 튜토리얼 페이지에 있습니다.*", md)
        md = re.sub(r"이 문서는 \[PDF\]\([^)]+\) 와 \[Word 문서\]\([^)]+\) 로도 받을 수 있습니다\.", "", md)
        with tempfile.NamedTemporaryFile("w", suffix=".md", dir=TUT, delete=False) as f:
            f.write(md)
        try:
            subprocess.run(["pandoc", "-f", "gfm", "-t", "docx", "--resource-path", str(TUT),
                            "--reference-doc", str(REFERENCE), "-o", str(TUT / f"{name}-tutorial.docx"), f.name], check=True)
        finally:
            Path(f.name).unlink()
        sizes = " · ".join(f"{ext} {(TUT / f'{name}-tutorial.{ext}').stat().st_size / 1e6:.1f} MB" for ext in ("pdf", "docx"))
        print(f"{name}: {sizes}")
    browser.close()
