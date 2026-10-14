#!/usr/bin/env python3
"""Every local href and src in the built site must point at a file that is there.

Run after build_docs.py. Links inside code samples are not links, so <pre> and
<code> are left out. Exits non-zero if anything is broken, which is what stops
a deploy.
"""
import os, re, sys
from pathlib import Path

WEB = Path(__file__).resolve().parent
broken, checked = [], 0
for page in sorted(WEB.rglob("*.html")):
    text = page.read_text()
    text = re.sub(r"<(pre|code)\b.*?</\1>", "", text, flags=re.S)
    for m in re.finditer(r'\b(?:href|src)="([^"#?]+)[^"]*"', text):
        url = m.group(1)
        if re.match(r"^(https?:|mailto:|data:|javascript:)", url):
            continue
        checked += 1
        target = Path(os.path.normpath(page.parent / url))
        if not target.exists():
            broken.append((page.relative_to(WEB), url))
print(f"{checked} local links checked, {len(broken)} broken")
for page, url in broken:
    print(f"  {page}: {url}")
sys.exit(1 if broken else 0)
