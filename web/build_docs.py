#!/usr/bin/env python3
"""Build the documentation site: docs/**/*.md → web/docs/**/*.html.

A static site with no build chain beyond pandoc. Every Markdown file under
docs/ becomes a page at the same relative path; images and media beside them
are copied; links between documents are rewritten to the pages; links that
leave docs/ (to code, examples, specs) point at the repository.

    python3 web/build_docs.py            # writes web/docs/
    python3 -m http.server -d web 8088   # http://localhost:8088/docs/

The pages share the landing page's palette and follow the reader's light/dark
preference. Mermaid diagrams render in the browser.
"""
import html, os, re, shutil, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "docs"
OUT = ROOT / "web" / "docs"
REPO = "https://github.com/uengine-oss/ontological-db/blob/main/"

# Sidebar groups, in reading order. Anything not listed falls in by name.
GROUPS = [
    ("tutorials", "튜토리얼"),
    ("guides", "사용자 매뉴얼"),
    ("", "레퍼런스"),
    ("00_overview", "개요"),
    ("01_architecture", "아키텍처"),
    ("01_architecture/09_performance", "아키텍처 · 성능"),
    ("02_api", "API"),
    ("03_backend", "엔진"),
    ("04_frontend", "Studio"),
    ("05_llm", "에이전트 · LLM"),
    ("06_data", "데이터"),
    ("07_security", "보안"),
    ("08_operations", "운영"),
    ("99_decisions", "결정 기록 (ADR)"),
]
FEATURED = "tutorials/virtual-fabric-tutorial.md"


def title_of(md: str, fallback: str) -> str:
    m = re.search(r"^#\s+(.+)$", md, re.M)
    return re.sub(r"[`*]", "", m.group(1)).strip() if m else fallback


def out_rel(rel: Path) -> Path:
    """docs/README.md is the site's front page; every other file keeps its place."""
    if rel == Path("README.md"):
        return Path("index.html")
    return rel.with_suffix(".html")


def pandoc(md: str) -> str:
    p = subprocess.run(
        ["pandoc", "-f", "gfm+gfm_auto_identifiers", "-t", "html", "--no-highlight", "--wrap=none"],
        input=md, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(p.stderr)
    return p.stdout


def relink(page_html: str, rel: Path, pages: set) -> str:
    """Point each link at what it means on the site."""
    here = (SRC / rel).parent

    def fix(m):
        attr, url = m.group(1), html.unescape(m.group(2))
        if re.match(r"^(https?:|mailto:|#|data:)", url):
            return m.group(0)
        path, _, frag = url.partition("#")
        target = (here / path).resolve()
        frag = "#" + frag if frag else ""
        try:
            inside = target.relative_to(SRC)
        except ValueError:
            inside = None
        if inside is not None and inside in pages:
            to = out_rel(inside)
        elif inside is not None and target.exists() and attr == "href" and target.is_dir():
            index = next((inside / n for n in ("README.md", "00_index.md") if inside / n in pages), None)
            if index is None:
                return m.group(0)
            to = out_rel(index)
        elif inside is not None:
            return m.group(0)  # an image or a file copied alongside
        else:
            # Outside docs/: code, examples, specs. The site does not carry them.
            try:
                return f'{attr}="{REPO}{target.relative_to(ROOT).as_posix()}{frag}"'
            except ValueError:
                return m.group(0)
        up = os.path.relpath(OUT / to, (OUT / out_rel(rel)).parent)
        return f'{attr}="{html.escape(up)}{frag}"'

    return re.sub(r'\b(href|src)="([^"]+)"', fix, page_html)


def enrich(body: str) -> str:
    # A paragraph that is only a link to a video is the video.
    body = re.sub(
        r'<p><a href="([^"]+\.(?:mp4|webm))">([^<]*)</a></p>',
        r'<figure class="video"><video controls preload="metadata" src="\1"></video><figcaption>\2</figcaption></figure>',
        body)
    # pandoc: <pre class="mermaid"><code>…</code></pre>; mermaid wants the text itself.
    body = re.sub(r'<pre class="mermaid"><code>(.*?)</code></pre>',
                  lambda m: f'<pre class="mermaid">{m.group(1)}</pre>', body, flags=re.S)
    body = body.replace("<table>", '<div class="tablewrap"><table>').replace("</table>", "</table></div>")
    return body


def sidebar(nav, current: Path) -> str:
    parts = []
    for key, label in nav["groups"]:
        items = nav["by_group"].get(key, [])
        if not items:
            continue
        is_open = any(rel == current for rel, _ in items)
        links = []
        for rel, title in items:
            href = os.path.relpath(OUT / out_rel(rel), (OUT / out_rel(current)).parent)
            cls = ' class="here"' if rel == current else ""
            links.append(f'<a{cls} href="{html.escape(href)}">{html.escape(title)}</a>')
        parts.append(f'<details{" open" if is_open or key in ("tutorials",) else ""}>'
                     f'<summary>{html.escape(label)} <span>{len(items)}</span></summary>{"".join(links)}</details>')
    return "".join(parts)


PAGE = """<!doctype html>
<html lang="ko">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} — Ontological DB 문서</title>
<link rel="stylesheet" href="{root}assets/docs.css">
<header class="topbar">
  <a class="mark" href="{root}../index.html">
    <svg width="28" height="28" viewBox="0 0 120 120" aria-label="Ontological DB">
      <circle cx="60" cy="60" r="32" fill="none" stroke="#10b981" stroke-width="7.5"/>
      <circle cx="60" cy="60" r="17.5" fill="none" stroke="#10b981" stroke-width="6.5"/>
    </svg>
    <span class="wm">ONTOLOGICAL&nbsp;DB</span>
  </a>
  <a class="crumb" href="{root}index.html">문서</a>
  <input id="filter" type="search" placeholder="문서 제목 찾기" aria-label="문서 제목 찾기">
  <a class="src" href="{source}">원문 보기</a>
</header>
<div class="shell">
  <nav class="side" id="side">{sidebar}</nav>
  <main>
    {banner}
    <article>{body}</article>
    <footer>이 페이지는 <code>docs/{rel}</code> 에서 만들어졌습니다 · <code>python3 web/build_docs.py</code></footer>
  </main>
  <aside class="toc" id="toc"></aside>
</div>
<script src="{root}assets/docs.js"></script>
<script type="module">
  const blocks = document.querySelectorAll('pre.mermaid');
  if (blocks.length) {{
    const {{ default: mermaid }} = await import('https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs');
    const dark = matchMedia('(prefers-color-scheme: dark)').matches && !matchMedia('print').matches;
    mermaid.initialize({{ startOnLoad: false, theme: dark ? 'dark' : 'neutral' }});
    await mermaid.run({{ nodes: blocks }});
    document.documentElement.dataset.diagrams = 'ready';
  }} else {{
    document.documentElement.dataset.diagrams = 'ready';
  }}
</script>
"""


def main():
    files = sorted(SRC.rglob("*.md"))
    rels = [f.relative_to(SRC) for f in files]
    pages = set(rels)
    texts = {rel: (SRC / rel).read_text() for rel in rels}
    titles = {rel: title_of(texts[rel], rel.stem) for rel in rels}

    known = [k for k, _ in GROUPS]
    groups = list(GROUPS)
    by_group = {}
    for rel in rels:
        key = rel.parent.as_posix() if rel.parent != Path(".") else ""
        if key not in known:
            known.append(key)
            groups.append((key, key))
        by_group.setdefault(key, []).append((rel, titles[rel]))
    for key, items in by_group.items():
        # The front page first, then indexes, then by file name — which is how the files are numbered.
        items.sort(key=lambda it: (it[0].name not in ("README.md", "00_index.md"), it[0].name))
    nav = {"groups": groups, "by_group": by_group}

    if OUT.exists():
        shutil.rmtree(OUT)
    (OUT / "assets").mkdir(parents=True)
    for name in ("docs.css", "docs.js"):
        shutil.copy(Path(__file__).parent / "docs-assets" / name, OUT / "assets" / name)

    copied = 0
    for f in SRC.rglob("*"):
        if f.is_file() and f.suffix != ".md" and f.name != ".DS_Store":
            dest = OUT / f.relative_to(SRC)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(f, dest)
            copied += 1

    featured = Path(FEATURED)
    for rel in rels:
        body = enrich(relink(pandoc(texts[rel]), rel, pages))
        dest = OUT / out_rel(rel)
        dest.parent.mkdir(parents=True, exist_ok=True)
        root = os.path.relpath(OUT, dest.parent) + "/"
        banner = ""
        if rel == Path("README.md") and featured in pages:
            href = os.path.relpath(OUT / out_rel(featured), dest.parent)
            banner = (f'<a class="feature" href="{html.escape(href)}"><span>새 튜토리얼</span>'
                      f'<b>{html.escape(titles[featured])}</b>'
                      f'<em>PostgreSQL과 MySQL에 흩어진 데이터를 복사 없이 그래프로 — Studio에서 사이퍼만으로</em></a>')
        dest.write_text(PAGE.format(
            title=html.escape(titles[rel]), root=root, sidebar=sidebar(nav, rel), banner=banner, body=body,
            rel=rel.as_posix(), source=REPO + "docs/" + rel.as_posix()))
    print(f"ok — {len(rels)} pages, {copied} assets → {OUT.relative_to(ROOT)}/")


if __name__ == "__main__":
    main()
