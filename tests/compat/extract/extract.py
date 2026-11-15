#!/usr/bin/env python3
"""Pull the Cypher that Studio, Robo Architect Lite and Robo Analyzer send, out of their
sources (statically), and write the compatibility corpus (``tests/compat/corpus/``).

    python3 tests/compat/extract/extract.py              # all clients
    python3 tests/compat/extract/extract.py robo         # one client
    python3 tests/compat/extract/extract.py --check      # only report; write nothing

What it reads is in ``clients.json`` (roots, globs); how assembled queries are filled in
is in ``assembly.json`` (the rules). The client checkouts are only read.

Output per client (``corpus/<client>/``):

    queries/<id>.cypher      one statement each, exactly the text that goes over Bolt
    params/<id>.json         its parameters (literal values, or {"$ref": …} the runner looks up)
    fixture/NN_*.cypher      data the queries run against (hand-kept files + 90_derived.cypher)
    manifest.json            every query: kind, origins (file:line), placeholders, scenario
    excluded.json            candidates that did not make it, and why
"""
from __future__ import annotations

import argparse
import fnmatch
import glob
import hashlib
import json
import os
import re
import subprocess
import sys
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
COMPAT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
sys.path.insert(0, COMPAT)

import jssrc  # noqa: E402
import pysrc  # noqa: E402
import shape  # noqa: E402
import prepare  # noqa: E402


def load_json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def root_path(root: dict) -> str | None:
    p = os.environ.get(root.get("env", ""), "") or root["path"]
    if os.path.isdir(p):
        return p
    fb = root.get("fallback")
    if fb and os.path.isdir(fb):
        return fb
    return None


def files_of(base: str, include: list[str], exclude: list[str]) -> list[str]:
    out = set()
    for pat in include:
        for f in glob.glob(os.path.join(base, pat), recursive=True):
            if os.path.isfile(f):
                rel = os.path.relpath(f, base)
                if any(fnmatch.fnmatch(rel, x) or fnmatch.fnmatch("/" + rel, "*/" + x.lstrip("*/")) for x in exclude):
                    continue
                if "/.venv/" in "/" + rel or "/node_modules/" in "/" + rel or "__pycache__" in rel:
                    continue
                out.add(f)
    return sorted(out)


def git_state(repo: str) -> dict:
    def run(*a):
        r = subprocess.run(["git", "-C", repo, *a], capture_output=True, text=True)
        return r.stdout.strip() if r.returncode == 0 else ""
    head = run("rev-parse", "--short", "HEAD")
    dirty = bool(run("status", "--porcelain", "--untracked-files=no"))
    return {"commit": head or None, "dirty": dirty}


# ---------------------------------------------------------------- transforms

# robo-data-analyzer graph/neo4j_client.py `for_ontological`: what the analyzer sends to an
# Ontological gateway is its Neo4j text with `MATCH (x:ANALYZER_OWNED {…})` unlabelled.
_OWNED_MATCH = re.compile(r"(MATCH\s*\(\s*[A-Za-z_]\w*)\s*:\s*ANALYZER_OWNED(?=\s*\{)")
TRANSFORMS = {"for_ontological": lambda q: _OWNED_MATCH.sub(r"\1", q)}


# ---------------------------------------------------------------- statements in .cypher / .md

def split_statements(text: str) -> list[tuple[int, str]]:
    """Split a script on top-level `;` (outside strings and // comments). (line, statement)."""
    out, cur, line, start_line = [], [], 1, 1
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if c in "'\"`":
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == "\\" else 1
            cur.append(text[i:j + 1])
            line += text[i:j + 1].count("\n")
            i = j + 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            i = j
            continue
        if c == ";":
            stmt = "".join(cur).strip()
            if stmt:
                out.append((start_line, stmt))
            cur = []
            i += 1
            start_line = line
            continue
        if c == "\n":
            line += 1
            if not "".join(cur).strip():
                start_line = line
        cur.append(c)
        i += 1
    stmt = "".join(cur).strip()
    if stmt:
        out.append((start_line, stmt))
    return out


def markdown_blocks(text: str) -> list[tuple[int, str]]:
    out = []
    for m in re.finditer(r"```cypher\s*\n(.*?)```", text, re.S):
        out.append((text.count("\n", 0, m.start()) + 2, m.group(1)))
    return out


# ---------------------------------------------------------------- scanning

def scan_client(name: str, conf: dict, assembly: dict) -> tuple[list[dict], list[dict], list[dict]]:
    """-> (raw candidates, schema-script statements, sources)"""
    raws, scripts, sources = [], [], []
    rules_client = dict(assembly.get("placeholders", {}).get(name, {}))
    for root in conf["roots"]:
        base = root_path(root)
        if base is None:
            print(f"  ! {root['name']}: no checkout at {root['path']} — skipped", file=sys.stderr)
            continue
        st = git_state(root.get("repo", base))
        sources.append({"name": root["name"], "path": base, **st})
        index = pysrc.ModuleIndex(base)
        for f in files_of(base, root.get("include", []), root.get("exclude", [])):
            rel = os.path.relpath(f, base)
            rules = dict(rules_client)
            rules.update(assembly.get("file_placeholders", {}).get(f"{root['name']}:{rel}", {}))
            if f.endswith(".py"):
                for c in pysrc.candidates(f, index, rules):
                    raws.append({"text": c.text, "origin": f"{root['name']}:{rel}:{c.line}", "context": c.context,
                                 "unresolved": c.unresolved, "placeholders": c.placeholders,
                                 "function": c.function, "hint": c.function_src_hint,
                                 "transform": root.get("transform")})
            elif f.endswith((".js", ".mjs", ".ts")):
                src = open(f, encoding="utf-8").read()
                for lit in jssrc.literals(src, rules):
                    raws.append({"text": lit.text, "origin": f"{root['name']}:{rel}:{lit.line}", "context": "js",
                                 "unresolved": lit.unresolved, "placeholders": lit.placeholders,
                                 "function": None, "hint": "", "transform": root.get("transform")})
        for f in files_of(base, root.get("markdown", []), []):
            rel = os.path.relpath(f, base)
            for line, block in markdown_blocks(open(f, encoding="utf-8").read()):
                for sl, stmt in split_statements(block):
                    raws.append({"text": stmt, "origin": f"{root['name']}:{rel}:{line + sl - 1}",
                                 "context": "llm-example", "unresolved": [], "placeholders": {},
                                 "function": None, "hint": "", "transform": None})
        for f in files_of(base, root.get("schema_scripts", []), []):
            rel = os.path.relpath(f, base)
            for line, stmt in split_statements(open(f, encoding="utf-8").read()):
                scripts.append({"text": stmt, "origin": f"{root['name']}:{rel}:{line}", "file": rel})
    return raws, scripts, sources


def collapse(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def qid(client: str, text: str) -> str:
    return f"{client}-{hashlib.sha1(collapse(text).encode()).hexdigest()[:10]}"


def classify(raws: list[dict], name: str, assembly: dict) -> tuple[dict[str, dict], list[dict]]:
    skip = {s["origin"]: s["reason"] for s in assembly.get("skip", []) if s.get("client", name) == name}
    covered = {}
    for a in assembly.get("assembled", []):
        if a["client"] == name:
            for o in a.get("covers", []):
                covered[o] = a["rule"]
    kept: dict[str, dict] = {}
    excluded: list[dict] = []
    for r in raws:
        text = textwrap.dedent(r["text"]).strip()
        if len(text.split()) == 1 or re.search(r"\\[swdbS(]", text):
            continue  # a single keyword-looking word, or a regular expression
        ok, why = shape.looks_like_cypher(text)
        if not ok:
            if shape.START.match(text) and len(text) > 12 and why not in ("prose", "capitalised word", "prose (Korean)"):
                excluded.append({"origin": r["origin"], "reason": f"not Cypher ({why})", "text": text[:300]})
            continue
        if r["origin"] in covered:
            excluded.append({"origin": r["origin"], "reason": "assembled by hand: " + covered[r["origin"]], "text": text[:400]})
            continue
        if r["origin"] in skip:
            excluded.append({"origin": r["origin"], "reason": f"skipped: {skip[r['origin']]}", "text": text[:300]})
            continue
        if r["unresolved"]:
            reason = "assembled by hand: " + covered[r["origin"]] if r["origin"] in covered else \
                "unresolved placeholder: " + ", ".join(r["unresolved"])
            excluded.append({"origin": r["origin"], "reason": reason, "text": text[:400]})
            continue
        if not shape.is_complete(text):
            reason = "assembled by hand: " + covered[r["origin"]] if r["origin"] in covered else "fragment"
            excluded.append({"origin": r["origin"], "reason": reason, "text": text[:400], "context": r["context"]})
            continue
        if r.get("transform"):
            text = TRANSFORMS[r["transform"]](text)
        add(kept, name, text, r)
    for a in assembly.get("assembled", []):
        if a["client"] != name:
            continue
        r = {"origin": a["origins"][0], "context": "assembled", "placeholders": {}, "hint": "",
             "function": None, "rule": a["rule"], "graph": a.get("graph")}
        text = a["query"].strip()
        e = add(kept, name, text, r)
        for o in a["origins"][1:]:
            if o not in e["origins"]:
                e["origins"].append(o)
    return kept, excluded


def add(kept, name, text, r):
    key = collapse(text)
    e = kept.get(key)
    if e is None:
        e = kept[key] = {
            "id": qid(name, text), "text": text, "kind": shape.kind(text), "origins": [],
            "contexts": [], "placeholders": {}, "hints": [], "params": shape.params(text),
        }
        if r.get("rule"):
            e["assembly_rule"] = r["rule"]
        if r.get("graph"):
            e["graph"] = r["graph"]
    if r["origin"] not in e["origins"]:
        e["origins"].append(r["origin"])
    if r["context"] not in e["contexts"]:
        e["contexts"].append(r["context"])
    e["placeholders"].update(r.get("placeholders") or {})
    if r.get("hint") and r["hint"] not in e["hints"]:
        e["hints"].append(r["hint"])
    return e


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("clients", nargs="*")
    ap.add_argument("--check", action="store_true", help="report only, write nothing")
    args = ap.parse_args()
    clients = load_json(os.path.join(HERE, "clients.json"))
    assembly = load_json(os.path.join(HERE, "assembly.json"))
    names = args.clients or [k for k in clients if not k.startswith("_")]
    for name in names:
        conf = clients[name]
        print(f"[{name}]")
        raws, scripts, sources = scan_client(name, conf, assembly)
        kept, excluded = classify(raws, name, assembly)
        entries = sorted(kept.values(), key=lambda e: e["origins"][0])
        counts = {}
        for e in entries:
            counts[e["kind"]] = counts.get(e["kind"], 0) + 1
        unresolved = [x for x in excluded if x["reason"].startswith("unresolved")]
        frags = [x for x in excluded if x["reason"] == "fragment"]
        print(f"  candidates {len(raws)}  kept {len(entries)} {counts}  schema-script statements {len(scripts)}")
        print(f"  excluded {len(excluded)} (unresolved {len(unresolved)}, fragments {len(frags)})")
        if args.check:
            for x in unresolved + frags:
                print(f"    - {x['origin']}: {x['reason']}")
            continue
        prepare.write_corpus(os.path.join(COMPAT, "corpus", name), name, conf, entries, excluded, scripts,
                             sources, assembly)


if __name__ == "__main__":
    main()
