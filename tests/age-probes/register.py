#!/usr/bin/env python3
"""The rewrite register (DIRECTION.md 4.2, rules 1-2): every gateway rewrite with its number, the AGE
defect or missing Neo4j surface it is for, its reproduction (the probes) and the upstream draft.

    python3 tests/age-probes/register.py [--json RESULT.json]   > markdown on stdout

Built from probes.json (one row per distinct `rewrite`); with --json, the last run's result per probe
is shown too. bolt/README.md carries the output ("Rewrite register")."""
import argparse, json, os
HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("--json", default=None)
args = ap.parse_args()
d = json.load(open(os.path.join(HERE, "probes.json"), encoding="utf-8"))
probes = d["probes"] if isinstance(d, dict) else d
res = {}
if args.json:
    r = json.load(open(args.json, encoding="utf-8"))
    res = {x["id"]: x["status"] for x in (r["results"] if isinstance(r, dict) else r)}
rows, order = {}, []
for p in probes:
    key = p.get("rewrite") or p["id"]
    if key not in rows:
        rows[key] = {"defects": [], "upstream": [], "probes": [], "kinds": set()}
        order.append(key)
    row = rows[key]
    for x in p.get("defects") or []:
        if x not in row["defects"]:
            row["defects"].append(x)
    if p.get("upstream") and p["upstream"] not in row["upstream"]:
        row["upstream"].append(p["upstream"])
    kind = "gateway-only" if p.get("gateway_only") else p.get("kind", "defect")
    row["kinds"].add(kind)
    st = res.get(p["id"])
    row["probes"].append(f"`{p['id']}`" + (f" {st}" if st else ""))
print("| # | rewrite (what AGE gets wrong or lacks → what the gateway sends) | kind | defect ids | upstream draft | probes |")
print("|---|---|---|---|---|---|")
for i, key in enumerate(order, 1):
    r = rows[key]
    up = ", ".join(os.path.basename(u).split("-")[0] if u.startswith("upstream/") else u for u in r["upstream"]) or "—"
    print(f"| R{i:02d} | {key.replace('|', chr(92) + '|')} | {', '.join(sorted(r['kinds']))} | {', '.join(r['defects']) or '—'} | {up} | {', '.join(r['probes'])} |")
