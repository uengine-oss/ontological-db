#!/usr/bin/env python3
"""Compare two mold folders — the golden one and a fresh capture — case by case.

    python3 tests/seam/compare.py <golden> <got> [--layer all|sql|trace] [--show N] [--width W]

A case is one statement of the compat corpus (fixture, read or scenario step: a `=== ` block),
or, in a whole-suite trace (an SQL regression file, TypeQL, the tutorials), one statement the
client sent together with everything the engine ran under it (a depth-0 line and the lines
after it). Files may be compressed (`*.xz`, as the golden traces are, or `*.gz`); a file and its
compressed copy are the same file. `SOURCE` (where the golden mold came from) is not compared.

Exit status: 0 the same, 1 the sql layer differs (or a file is missing), 3 only the trace
layer differs, 2 bad usage.
"""
from __future__ import annotations

import argparse
import difflib
import gzip
import lzma
import os
import sys


def read_lines(path):
    if path.endswith(".xz"):
        with lzma.open(path, "rt", encoding="utf-8") as f:
            return f.read().splitlines()
    if path.endswith(".gz"):
        with gzip.open(path, "rt", encoding="utf-8") as f:
            return f.read().splitlines()
    with open(path, encoding="utf-8") as f:
        return f.read().splitlines()


def files_of(root):
    out = {}
    for d, _, fs in os.walk(root):
        for f in fs:
            p = os.path.join(d, f)
            rel = os.path.relpath(p, root)
            if rel == "SOURCE":
                continue
            out[rel[:-3] if rel.endswith((".gz", ".xz")) else rel] = p
    return out


def cases(lines):
    """Split into (key, lines) blocks: `=== ` headers if there are any, else depth-0 lines."""
    header = (lambda ln: ln.startswith("=== ")) if any(ln.startswith("=== ") for ln in lines[:50]) \
        else (lambda ln: ln.startswith("0 "))
    out, cur = [], None
    for ln in lines:
        if header(ln) or cur is None:
            cur = (ln if header(ln) else "(before the first statement)", [])
            out.append(cur)
        cur[1].append(ln)
    return out


def clip(s, width):
    return s if len(s) <= width else s[:width] + f"… (+{len(s) - width} chars)"


def compare_file(want, got, show, width):
    """Returns (number of differing cases, total cases, report lines)."""
    wc, gc = cases(want), cases(got)
    report = []
    if [k for k, _ in wc] != [k for k, _ in gc]:
        # different statements, or in a different order: show where the sequences part
        diff = list(difflib.unified_diff([k for k, _ in wc], [k for k, _ in gc], "golden", "got", n=1, lineterm=""))
        report.append(f"    the cases themselves differ ({len(wc)} golden, {len(gc)} got):")
        report += ["      " + clip(d, width) for d in diff[: 6 * show + 2]]
        return max(1, sum(1 for d in diff if d[:1] in "+-") // 2), len(wc), report
    bad = [(k, w, g) for (k, w), (_, g) in zip(wc, gc) if w != g]
    for k, w, g in bad[:show]:
        report.append(f"    case {clip(k, width)}")
        for d in list(difflib.unified_diff(w, g, "golden", "got", n=1, lineterm=""))[2:40]:
            report.append("      " + clip(d, width))
    if len(bad) > show:
        report.append(f"    … and {len(bad) - show} more cases")
    return len(bad), len(wc), report


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("golden")
    ap.add_argument("got")
    ap.add_argument("--layer", default="all", choices=["all", "sql", "trace"])
    ap.add_argument("--show", type=int, default=3, help="differing cases to print per file (default 3)")
    ap.add_argument("--width", type=int, default=240, help="characters of a line to print (default 240)")
    args = ap.parse_args()
    for d in (args.golden, args.got):
        if not os.path.isdir(d):
            print(f"not a folder: {d}", file=sys.stderr)
            sys.exit(2)
    want, got = files_of(args.golden), files_of(args.got)
    layer_bad = {"sql": 0, "trace": 0}
    totals = {"sql": [0, 0], "trace": [0, 0]}
    for rel in sorted(set(want) | set(got)):
        layer = "trace" if rel.startswith("trace" + os.sep) else "sql"
        if args.layer != "all" and layer != args.layer:
            continue
        if rel not in got:
            print(f"{rel}: MISSING from the capture")
            layer_bad[layer] += 1
            continue
        if rel not in want:
            print(f"{rel}: not in the golden mold")
            layer_bad[layer] += 1
            continue
        w, g = read_lines(want[rel]), read_lines(got[rel])
        n, total, report = compare_file(w, g, args.show, args.width)
        totals[layer][0] += total - n
        totals[layer][1] += total
        if n:
            layer_bad[layer] += 1
            print(f"{rel}: {n} of {total} cases differ")
            print("\n".join(report))
    for layer in ("sql", "trace"):
        if args.layer in ("all", layer):
            same, total = totals[layer]
            verdict = "same" if layer_bad[layer] == 0 else f"DIFFERENT in {layer_bad[layer]} files"
            print(f"{layer:<5} layer: {verdict} — {same}/{total} cases identical")
    if layer_bad["sql"]:
        sys.exit(1)
    if layer_bad["trace"]:
        sys.exit(3)
    sys.exit(0)


if __name__ == "__main__":
    main()
