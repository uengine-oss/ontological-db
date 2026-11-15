"""Turn extracted queries into a runnable corpus: graphs, parameters, fixture, scenarios.

Called by extract.py. Everything here is a rule applied to the query text (shape.py); the
rules are written out in tests/compat/README.md ("준비 데이터", "매개변수").
"""
from __future__ import annotations

import glob
import json
import os
import re
import shutil
import sys
import uuid

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))
import shape  # noqa: E402

VECTOR_DIM = 8
FIXED_TIME = "2026-01-15T09:00:00Z"
ANALYZER_LABEL = re.compile(r"^[A-Z][A-Z0-9_]{2,}$")  # TABLE, FUNCTION, ANALYZER_OWNED … (Robo's are CamelCase)


def dump(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(obj, f, ensure_ascii=False, indent=1, sort_keys=False)
        f.write("\n")


# ---------------------------------------------------------------- graph routing

def graph_of(entry: dict, conf: dict) -> str:
    if entry.get("graph"):
        return entry["graph"]
    ag = conf.get("analyzer_graph")
    if ag:
        labels = [l for _, ls, _ in shape.node_patterns(entry["text"]) for l in ls]
        labels += re.findall(r"\b[a-z_]\w*:([A-Z][A-Z0-9_]{2,})\b", shape.mask_strings(entry["text"]))
        if any(ANALYZER_LABEL.match(l) for l in labels) or "'analyzer'" in entry["text"]:
            return ag
    return conf["graph"]


# ---------------------------------------------------------------- what the fixture holds

class Model:
    """Labels, label chains, properties and relationship triples of one graph."""

    def __init__(self):
        self.chains: dict[str, list[str]] = {}       # label -> longest chain ending with it
        self.props: dict[str, set[str]] = {}
        self.literals: dict[tuple[str, str], list] = {}
        self.triples: set[tuple[str, str, str]] = set()
        self.partial: set[tuple[str | None, str, str | None]] = set()  # one end without a label
        self.keys: dict[str, tuple[str, list]] = {}  # label -> (key prop, values) known in the fixture
        self.types: dict[tuple[str, str], str] = {}  # (label, prop) -> bool | int | float | list, from coalesce() defaults

    def learn(self, text: str, writes: bool):
        for var, labels, _ in shape.node_patterns(text):
            for i, l in enumerate(labels):
                chain = labels[: i + 1]
                if len(chain) > len(self.chains.get(l, [])):
                    self.chains[l] = chain
        for label, props in shape.property_uses(text).items():
            self.props.setdefault(label, set()).update(props)
        vl = shape.var_labels(text)
        for m in re.finditer(r"coalesce\s*\(\s*(\w+)\.`?(\w+)`?\s*,\s*(true|false|-?\d+\.\d+|-?\d+|\[)", text, re.I):
            if m.group(1) in vl:
                d = m.group(3).lower()
                t = "bool" if d in ("true", "false") else "list" if d == "[" else "float" if "." in d else "int"
                self.types.setdefault((vl[m.group(1)][-1], m.group(2)), t)
        for label, prop, value in shape.literal_equalities(text):
            vals = self.literals.setdefault((label, prop), [])
            if value not in vals:
                vals.append(value)
        for r in shape.rel_patterns(text):
            for t in r["types"]:
                if r["from"] and r["to"]:
                    self.triples.add((r["from"][-1], t, r["to"][-1]))
                elif r["from"] or r["to"]:
                    self.partial.add((r["from"][-1] if r["from"] else None, t, r["to"][-1] if r["to"] else None))

    def complete_partial(self):
        """(A)-[:T]->(?) takes its other end from a full (A)-[:T]->(B) elsewhere, or from any (X)-[:T]->(B)."""
        for a, t, b in sorted(self.partial, key=lambda x: (x[0] or "", x[1], x[2] or "")):
            full = sorted(x for x in self.triples if x[1] == t)
            if not full:
                continue
            if a is None:
                ends = [x[0] for x in full if x[2] == b] or [full[0][0]]
                self.triples.add((ends[0], t, b))
            else:
                ends = [x[2] for x in full if x[0] == a] or [full[0][2]]
                self.triples.add((a, t, ends[0]))

    def infer(self, text: str) -> dict[str, str]:
        """variable -> label for variables the statement leaves unlabelled but ties to a relationship type."""
        out = {}
        for r in shape.rel_patterns(text):
            for t in r["types"]:
                cands = sorted(x for x in self.triples if x[1] == t)
                if not cands:
                    continue
                if not r["from"] and r.get("from_var"):
                    m = [x[0] for x in cands if not r["to"] or x[2] == r["to"][-1]]
                    if m:
                        out.setdefault(r["from_var"], m[0])
                if not r["to"] and r.get("to_var"):
                    m = [x[2] for x in cands if not r["from"] or x[0] == r["from"][-1]]
                    if m:
                        out.setdefault(r["to_var"], m[0])
        return out


RESERVED = {"_id", "_type", "_src", "_dst"}


def fixture_keys(fixture_text: str) -> tuple[dict[str, tuple[str, list]], set[str], set[tuple[str, str, str]]]:
    """From hand-kept fixture statements: label -> (key, values), labels created, triples created."""
    keys: dict[str, tuple[str, list]] = {}
    labels: set[str] = set()
    triples = set()
    for _, stmt in _statements(fixture_text):
        if shape.SCHEMA_DDL.match(stmt) or not re.search(r"\b(CREATE|MERGE)\s*\(", stmt, re.I):
            continue  # constraints and indexes name labels but make no nodes
        for var, ls, props in shape.node_patterns(stmt):
            if not ls:
                continue
            labels.update(ls)
            for k in ("id", "_source_id", "key", "schema_id", "class_name", "name"):
                m = re.search(r"\b" + k + r"\s*:\s*('(?:[^'\\]|\\.)*'|\"(?:[^\"\\]|\\.)*\")", props)
                if m:
                    cur = keys.setdefault(ls[-1], (k, []))
                    if cur[0] == k and m.group(1)[1:-1] not in cur[1]:
                        cur[1].append(m.group(1)[1:-1])
                    break
        for r in shape.rel_patterns(stmt):
            if r["from"] and r["to"]:
                for t in r["types"]:
                    triples.add((r["from"][-1], t, r["to"][-1]))
    return keys, labels, triples


def _statements(text):
    sys.path.insert(0, HERE)
    from extract import split_statements
    return split_statements(text)


def _camel_label(prop: str, known: dict) -> str | None:
    """bcId / bc_id / context_id / aggregateId -> a label the fixture has."""
    base = re.sub(r"(_id|Id|_ids|Ids)$", "", prop)
    if not base or base == prop:
        return None
    aliases = {"bc": "BoundedContext", "context": "BoundedContext", "agg": "Aggregate", "aggregate": "Aggregate",
               "cmd": "Command", "command": "Command", "evt": "Event", "event": "Event", "us": "UserStory",
               "userStory": "UserStory", "user_story": "UserStory", "rm": "ReadModel", "readModel": "ReadModel",
               "pol": "Policy", "policy": "Policy", "feature": "Feature", "proposal": "Proposal", "ui": "UI",
               "req": "Requirement", "requirement": "Requirement", "schema": "_OntologyModel", "task": "BpmTask",
               "process": "BpmProcess", "table": "TABLE", "column": "COLUMN", "parent": None, "source": None}
    if base in aliases:
        return aliases[base] if aliases[base] in known else None
    cand = base[0].upper() + base[1:]
    cand = re.sub(r"_(\w)", lambda m: m.group(1).upper(), cand)
    return cand if cand in known else None


def prop_value(label: str, prop: str, i: int, model: Model, known_keys: dict):
    """Deterministic value for node #i (1-based) of a derived label."""
    lits = model.literals.get((label, prop))
    if lits:
        return lits[(i - 1) % len(lits)]
    t = model.types.get((label, prop))
    if t == "bool":
        return i == 1
    if t == "int":
        return i * 10
    if t == "float":
        return round(0.5 + i / 10.0, 2)
    if t == "list":
        return [f"{prop}-{i}-a", f"{prop}-{i}-b"]
    p = prop
    low = p.lower()
    if p == "id":
        return f"{label}-{i}"
    if p in ("key", "_source_id"):
        return f"{label.lower()}-{i}"
    if p in ("name", "title", "displayName", "display_name", "label"):
        return f"{label} {i}"
    if low.endswith(("embedding", "vector")):
        return [round(((i * 7 + k * 3) % 11) / 10.0, 1) for k in range(VECTOR_DIM)]
    if re.search(r"(_ids|Ids)$", p):
        ref = _camel_label(p, known_keys)
        if ref and known_keys.get(ref):
            return list(known_keys[ref][1][:2])
        return [f"{p}-{i}-a", f"{p}-{i}-b"]
    if re.search(r"(_id|Id)$", p):
        ref = _camel_label(p, known_keys)
        if ref and known_keys.get(ref):
            vals = known_keys[ref][1]
            return vals[(i - 1) % len(vals)]
        return f"{p}-{i}"
    if re.search(r"(At|_at|Date|_date|time|Time)$", p) or low in ("timestamp", "since"):
        return f"2026-01-{10 + i:02d}T09:00:00Z"
    if re.match(r"(is|has|can)[A-Z_]", p) or low in ("enabled", "certified", "pivotal", "hotspot", "external",
                                                     "active", "recursive", "deleted", "archived", "required",
                                                     "nullable", "primary", "approved", "locked", "virtual"):
        return i == 1
    if low in ("count", "sequence", "seq", "order", "version", "level", "depth", "line", "start_line", "end_line",
               "priority_rank", "position", "index", "size", "max_rows", "default_limit", "credit_limit", "amount",
               "quantity", "qty", "rank", "number") or low.endswith(("count", "_line", "lines")):
        return i * 10
    if low in ("score", "confidence", "_confidence", "weight", "rating", "similarity"):
        return round(0.5 + i / 10.0, 2)
    if low in ("tags", "keywords", "synonyms", "labels", "aliases", "acceptancecriteria", "effects", "roles",
               "properties_list", "columns", "domain_terms", "evidence_files"):
        return [f"{p}-{i}-a", f"{p}-{i}-b"]
    return f"{label} {p} {i}"


def cy(v) -> str:
    """A Cypher literal."""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, list):
        return "[" + ", ".join(cy(x) for x in v) + "]"
    if v is None:
        return "null"
    return "'" + str(v).replace("\\", "\\\\").replace("'", "\\'") + "'"


def ident(name: str) -> str:
    return name if re.match(r"^[A-Za-z_]\w*$", name) else "`" + name.replace("`", "``") + "`"


def derive_fixture(model: Model, hand_text: str, graph: str, virtual: set[str], defaults: dict) -> tuple[str, dict]:
    keys, hand_labels, hand_triples = fixture_keys(hand_text)
    hand_labels |= virtual
    out = [f"// Derived from the labels, properties and relationship patterns the corpus queries name",
           f"// (tests/compat/extract/prepare.py). Graph: {graph}. Do not edit — re-run extract.py.", ""]
    derived = []
    # a label that is only ever a parent (ANALYZER_OWNED, _Entity) needs no nodes of its own
    parents = {c[i] for c in model.chains.values() for i in range(len(c) - 1)}
    supplement = []  # labels the hand-kept fixture has, given two more nodes with every property the corpus names
    for label in sorted(model.chains):
        if label in parents or label.startswith("⟦") or label in virtual:
            continue
        if label in hand_labels:
            if defaults.get("supplement") and model.props.get(label, set()) - {"id", "name"}:
                supplement.append(label)
            continue
        derived.append(label)
        keys[label] = ("id", [f"{label}-1", f"{label}-2"])
    for label in supplement:
        if label not in keys or keys[label][0] != "id":
            keys[label] = ("id", [f"{label}-1", f"{label}-2"])
        else:
            keys[label] = ("id", keys[label][1] + [f"{label}-1", f"{label}-2"])
    model.complete_partial()
    for label in derived + supplement:
        chain = model.chains[label]
        props = sorted(model.props.get(label, set()) | {"id", "name"} | set(defaults.get("always", {})))
        for i in (1, 2):
            kv = []
            for p in props:
                if p.startswith("⟦") or p in RESERVED or p.startswith("__"):
                    continue
                fixed = {**defaults.get("when_used", {}), **defaults.get("always", {})}
                v = fixed[p] if p in fixed and not model.literals.get((label, p)) else prop_value(label, p, i, model, keys)
                kv.append(f"{ident(p)}: {cy(v)}")
            out.append(f"CREATE (:{':'.join(ident(c) for c in chain)} {{{', '.join(kv)}}});")
    out.append("")
    for a, t, b in sorted(model.triples):
        if (a, t, b) in hand_triples or a not in keys or b not in keys:
            continue
        ka, va = keys[a]
        kb, vb = keys[b]
        # derived / supplemental nodes are the last two values of a key list
        va, vb = va[-2:], vb[-2:]
        for i in range(min(2, len(va), len(vb))):
            out.append(f"MATCH (a:{ident(a)} {{{ident(ka)}: {cy(va[i])}}}), (b:{ident(b)} {{{ident(kb)}: {cy(vb[i])}}}) "
                       f"CREATE (a)-[:{ident(t)}]->(b);")
    return "\n".join(out) + "\n", keys


# ---------------------------------------------------------------- parameters

def ref(label, prop, nth=0):
    return {"$ref": "prop", "label": label, "prop": prop, "nth": nth}


def eref(kind, label, nth=0):
    return {"$ref": kind, "label": label, "nth": nth}


def _clause_before(masked: str, pos: int) -> str:
    """The clause keyword that governs position pos (MATCH / MERGE / CREATE / SET / WHERE / …)."""
    best, kw = -1, ""
    for m in re.finditer(r"\b(OPTIONAL\s+MATCH|MATCH|MERGE|CREATE|SET|WHERE|WITH|UNWIND|RETURN|DELETE|REMOVE|ON\s+CREATE\s+SET|ON\s+MATCH\s+SET)\b",
                         masked[:pos], re.I):
        best, kw = m.start(), re.sub(r"\s+", " ", m.group(1).upper())
    return kw


def new_value(p: str, prop: str | None = None):
    name = prop or p
    low = name.lower()
    if low.endswith(("embedding", "vector", "vec")):
        return [round(((k * 5) % 11) / 10.0, 1) for k in range(VECTOR_DIM)]
    if re.search(r"(At|_at|Date|time)$", name) or low in ("now", "timestamp"):
        return FIXED_TIME
    if re.match(r"(is|has)[A-Z_]", name) or low in ("enabled", "certified", "pivotal", "hotspot", "active", "external",
                                                     "recursive", "value_bool"):
        return True
    if low in ("count", "sequence", "seq", "order", "version", "max_rows", "default_limit", "limit", "level",
               "amount", "credit_limit", "quantity", "position", "rank", "start_line", "end_line", "depth", "hops",
               "k", "top_k", "topk", "offset", "skip", "max_depth", "min_count", "threshold_count", "n") \
            or low.endswith(("count", "_limit", "_depth")):
        return 7 if low not in ("depth", "hops", "max_depth", "k", "top_k", "topk", "offset", "skip") else \
            (0 if low in ("offset", "skip") else 2 if "depth" in low or low == "hops" else 3)
    if low in ("score", "confidence", "weight", "_confidence", "s_conf", "o_conf"):
        return 0.9
    if re.search(r"(_ids|Ids|ids)$", name) or low in ("tags", "keywords", "synonyms", "labels", "effects"):
        return [f"compat-{name}-a", f"compat-{name}-b"]
    return f"compat-{name}"


def params_for(text: str, kind: str, model: Model, keys: dict, by_name: dict | None = None) -> dict:
    by_name = by_name or {}
    names = shape.params(text)
    if not names:
        return {}
    masked = shape.mask_strings(text)
    vl = shape.var_labels(text)
    for var, lab in model.infer(text).items():
        vl.setdefault(var, [lab])
    out: dict = {}

    def label_of(var):
        ls = vl.get(var)
        return ls[-1] if ls else None

    def existing(label, prop, nth=0):
        if label and label in keys and keys[label][0] == prop and len(keys[label][1]) > nth:
            return keys[label][1][nth]
        return ref(label, prop, nth) if label else new_value(prop)

    unwinds = {m.group(2): m.group(1) for m in re.finditer(r"UNWIND\s+\$(\w+)\s+AS\s+(\w+)", masked, re.I)}
    # WITH …, $p AS xs … UNWIND xs AS x  — the same thing in two steps
    for m in re.finditer(r"\$(\w+)\s+AS\s+(\w+)", masked, re.I):
        u = re.search(r"UNWIND\s+" + re.escape(m.group(2)) + r"\s+AS\s+(\w+)", masked, re.I)
        if u:
            unwinds.setdefault(u.group(1), m.group(1))
    # parameters that name a node by id first, so a "$type IN labels(n)" can follow that node's label
    names = sorted(names, key=lambda n: 1 if re.search(re.escape("$" + n) + r"\s+IN\s+labels", masked) else 0)
    for p in names:
        pe = re.escape("$" + p) + r"(?!\w)"
        val = None
        # $p IN labels(v): a label name
        m = re.search(pe + r"\s+IN\s+labels\s*\(\s*(\w+)\s*\)", masked, re.I)
        if m:
            lab = label_of(m.group(1))
            if not lab:
                known = sorted(k for k in keys if keys[k][0] == "id")
                lab = known[0] if known else "Command"
                for other in out.values():
                    if isinstance(other, str) and re.match(r"^[A-Za-z_]\w*-\d+$", other) and other.rsplit("-", 1)[0] in keys:
                        lab = other.rsplit("-", 1)[0]
            val = lab
        # LIMIT / SKIP
        if val is None and re.search(r"\b(LIMIT|SKIP)\s+" + pe, masked, re.I):
            val = 0 if re.search(r"\bSKIP\s+" + pe, masked, re.I) else 10
        # vector / fulltext procedures
        if val is None:
            m = re.search(r"db\.index\.vector\.queryNodes\s*\(([^)]*)\)", masked)
            if m and "$" + p in m.group(1):
                args = [a.strip() for a in m.group(1).split(",")]
                val = 3 if len(args) > 1 and args[1] == "$" + p else new_value("embedding")
        if val is None and re.search(r"db\.index\.fulltext\.queryNodes\s*\([^)]*" + pe, masked):
            val = "Customer"
        # elementId(v) = $p / id(v) = $p / … IN $p
        if val is None:
            m = re.search(r"\b(elementId|id)\s*\(\s*(\w+)\s*\)\s*(=|IN)\s*" + pe, masked) or \
                re.search(pe + r"\s*(=)\s*(elementId|id)\s*\(\s*(\w+)\s*\)", masked)
            if m:
                if m.group(1) in ("elementId", "id"):
                    fn, var, op = m.group(1), m.group(2), m.group(3)
                else:
                    op, fn, var = m.group(1), m.group(2), m.group(3)
                kind_ = "elementId" if fn == "elementId" else "id"
                lab = label_of(var) or "*"
                val = [eref(kind_, lab, 0), eref(kind_, lab, 1)] if op.upper() == "IN" else eref(kind_, lab, 0)
        # {prop: $p} inside a node pattern
        if val is None:
            for var, labels, props in shape.node_patterns(text):
                mm = re.search(r"([A-Za-z_]\w*|`[^`]+`)\s*:\s*" + pe, props)
                if mm and labels:
                    prop = mm.group(1).strip("`")
                    pos = text.find(props) if props else -1
                    clause = _clause_before(masked, pos) if pos >= 0 else "MATCH"
                    if clause in ("CREATE",) or (clause == "MERGE" and kind != "read"):
                        val = new_value(p, prop) if prop not in ("id", "key", "_source_id") else f"compat-new-{labels[-1]}"
                    else:
                        val = existing(labels[-1], prop)
                    break
                if mm and not labels:
                    prop = mm.group(1).strip("`")
                    lab = label_of(var) if var else None
                    val = existing(lab, prop) if lab else {"$ref": "anyprop", "prop": prop, "nth": 0}
                    break
        # v.prop <op> $p  /  $p = v.prop   (also toString(v.`prop`) = $p, toLower(v.prop) CONTAINS toLower($p))
        if val is None:
            m = re.search(r"\b(\w+)\.`?(\w+)`?\s*\)*\s*(=|<>|STARTS\s+WITH|ENDS\s+WITH|CONTAINS|>=|<=|>|<)\s*(?:\w+\s*\(\s*)*" + pe, masked, re.I)
            var = prop = None
            if m:
                var, prop = m.group(1), m.group(2)
            else:
                m = re.search(pe + r"\s*=\s*(\w+)\.(\w+)", masked)
                if m:
                    var, prop = m.group(1), m.group(2)
            if m:
                clause = _clause_before(masked, m.start())
                if clause in ("SET", "ON CREATE SET", "ON MATCH SET"):
                    val = new_value(p, prop)
                elif var in unwinds:
                    val = None
                else:
                    lab = label_of(var)
                    val = existing(lab, prop) if lab else {"$ref": "anyprop", "prop": prop, "nth": 0}
        # v.prop IN $p
        if val is None:
            m = re.search(r"\b(\w+)\.(\w+)\s+IN\s+" + pe, masked, re.I)
            if m:
                lab = label_of(m.group(1))
                val = [existing(lab, m.group(2), 0), existing(lab, m.group(2), 1)] if lab else \
                    [{"$ref": "anyprop", "prop": m.group(2), "nth": 0}, {"$ref": "anyprop", "prop": m.group(2), "nth": 1}]
        # a name the client always gives the same kind of value (assembly.json param_names)
        if val is None and p in by_name:
            val = by_name[p]
        # <anything> IN $p  -> a list
        if val is None and re.search(r"\bIN\s+" + pe, masked, re.I):
            val = [new_value(p) + "-a", new_value(p) + "-b"] if isinstance(new_value(p), str) else [new_value(p)]
        # UNWIND $p AS x
        if val is None and p in unwinds.values():
            x = [k for k, v in unwinds.items() if v == p][0]
            val = unwind_items(text, masked, x, model, keys)
        # SET v += $p / SET v = $p / CREATE (v:L $p)
        if val is None and re.search(r"(\+=|=)\s*" + pe + r"|\(\s*\w*\s*(:\s*\w+\s*)+" + pe, masked):
            if re.search(r"\b\w+\s*(\+=|=)\s*" + pe + r"(?!\s*\.)", masked) and not re.search(r"\w\.\w+\s*=\s*" + pe, masked):
                val = {"name": "compat", "description": "compat"}
        if val is None:
            m = re.search(r"\b\w+\.(\w+)\s*=\s*" + pe, masked)
            val = new_value(p, m.group(1) if m else None)
        out[p] = val
    # two parameters that would look up the same (label, property) get different nodes
    seen: dict[tuple, int] = {}
    for p, val in out.items():
        if isinstance(val, dict) and val.get("$ref") in ("prop", "anyprop", "elementId", "id"):
            k = (val["$ref"], val.get("label"), val.get("prop"))
            if k in seen:
                val["nth"] = seen[k] + 1
            seen[k] = val["nth"]
    return out


def unwind_items(text, masked, x, model, keys):
    fields = sorted(set(re.findall(r"\b" + re.escape(x) + r"\.(\w+)", masked)))
    if not fields:
        # bare value: {id: x} in a pattern?
        vl0 = shape.var_labels(text)
        for var, lab in model.infer(text).items():
            vl0.setdefault(var, [lab])
        for var, labels, props in shape.node_patterns(text):
            mm = re.search(r"(\w+)\s*:\s*" + re.escape(x) + r"\b(?!\.)", props)
            if mm:
                lab = labels[-1] if labels else (vl0.get(var, [None])[-1] if var else None)
                if lab:
                    return [_existing(keys, lab, mm.group(1), 0), _existing(keys, lab, mm.group(1), 1)]
                return [{"$ref": "anyprop", "prop": mm.group(1), "nth": 0}, {"$ref": "anyprop", "prop": mm.group(1), "nth": 1}]
        if re.search(r"\b(\w+)\.(\w+)\s*=\s*" + re.escape(x) + r"\b", masked):
            mm = re.search(r"\b(\w+)\.(\w+)\s*=\s*" + re.escape(x) + r"\b", masked)
            lab = shape.var_labels(text).get(mm.group(1), [None])[-1]
            if lab:
                return [_existing(keys, lab, mm.group(2), 0), _existing(keys, lab, mm.group(2), 1)]
        return ["compat-a", "compat-b"]
    items = []
    vl = shape.var_labels(text)
    for var, lab in model.infer(text).items():
        vl.setdefault(var, [lab])
    for i in range(2):
        item = {}
        for fi, f in enumerate(fields):
            v = None
            for var, labels, props in shape.node_patterns(text):
                mm = re.search(r"(\w+)\s*:\s*" + re.escape(x) + r"\." + f + r"\b", props)
                if mm and labels:
                    pos = text.find(props)
                    clause = _clause_before(masked, pos)
                    if clause in ("MATCH", "OPTIONAL MATCH"):
                        v = _existing(keys, labels[-1], mm.group(1), i)
                    else:
                        v = f"compat-new-{labels[-1]}-{i + 1}" if mm.group(1) in ("id", "key", "_source_id") else \
                            new_value(f, mm.group(1))
                    break
                if mm and var and vl.get(var):
                    v = _existing(keys, vl[var][-1], mm.group(1), i)
                    break
                if mm and _clause_before(masked, text.find(props)) in ("MATCH", "OPTIONAL MATCH"):
                    # an unlabelled node matched by a property: any node that has it (fields apart)
                    v = {"$ref": "anyprop", "prop": mm.group(1), "nth": i + 2 * fi}
                    break
            if v is None:
                m2 = re.search(r"\b(\w+)\.(\w+)\s*=\s*" + re.escape(x) + r"\." + f + r"\b", masked)
                if m2 and _clause_before(masked, m2.start()) == "WHERE" and vl.get(m2.group(1)):
                    v = _existing(keys, vl[m2.group(1)][-1], m2.group(2), i)
            if v is None:
                v = new_value(f)
                if isinstance(v, str):
                    v = f"{v}-{i + 1}"
            item[f] = v
        items.append(item)
    return items


def _existing(keys, label, prop, nth):
    if label in keys and keys[label][0] == prop and len(keys[label][1]) > nth:
        return keys[label][1][nth]
    return ref(label, prop, nth)


# ---------------------------------------------------------------- scenarios

def phase(text: str) -> int:
    m = shape.mask_strings(text).upper()
    if shape.SCHEMA_DDL.match(text) or re.match(r"^\s*CREATE\s+(DATASOURCE|VIRTUAL|GRAPH)", text, re.I):
        return 0
    if re.search(r"\b(DELETE|DETACH)\b", m) or re.match(r"^\s*DROP\b", text, re.I):
        return 3
    if re.search(r"\b(CREATE|MERGE)\b", m):
        return 1
    return 2


def scenario_name(origin: str) -> str:
    root, path, _ = origin.rsplit(":", 2)
    return f"{root}:{path}"


# ---------------------------------------------------------------- write

def write_corpus(outdir, name, conf, entries, excluded, scripts, sources, assembly):
    qdir, pdir, fdir = (os.path.join(outdir, d) for d in ("queries", "params", "fixture"))
    for d in (qdir, pdir):
        if os.path.isdir(d):
            shutil.rmtree(d)
        os.makedirs(d)
    os.makedirs(fdir, exist_ok=True)

    # client schema scripts are copied as fixture files (10_*), unchanged
    for old in glob.glob(os.path.join(fdir, "1?_client_*.cypher")):
        os.remove(old)
    by_file: dict[str, list] = {}
    for s in scripts:
        by_file.setdefault(s["file"], []).append(s)
    for i, (f, stmts) in enumerate(sorted(by_file.items())):
        dst = os.path.join(fdir, f"1{i}_client_{os.path.basename(f)}")
        with open(dst, "w", encoding="utf-8") as fh:
            fh.write(f"// Copied from the client: {stmts[0]['origin'].rsplit(':', 1)[0]} (statements unchanged)\n\n")
            fh.write("// One change: randomUUID() is replaced by a fixed UUID (uuid5 of file, statement, occurrence) so the\n"
                     "// fixture is the same at every run; the clients' queries keep their randomUUID().\n\n")
            for si, s in enumerate(stmts):
                n = [0]

                def fixed(_m, si=si, n=n):
                    n[0] += 1
                    return "'" + str(uuid.uuid5(uuid.NAMESPACE_URL, f"{f}#{si}#{n[0]}")) + "'"
                fh.write(re.sub(r"randomUUID\(\s*\)", fixed, s["text"].rstrip()) + ";\n\n")

    for e in entries:
        e["graph"] = graph_of(e, conf)
    graphs = sorted({e["graph"] for e in entries} | {conf["graph"]})

    # what each graph's queries expect
    models = {g: Model() for g in graphs}
    for e in entries:
        models[e["graph"]].learn(e["text"], e["kind"] != "read")

    hand = {}
    for g in graphs:
        texts = []
        for f in sorted(glob.glob(os.path.join(fdir, "*.cypher"))):
            if os.path.basename(f).startswith("90_"):
                continue
            t = open(f, encoding="utf-8").read()
            m = re.search(r"^//\s*graph:\s*(\S+)", t, re.M)
            if (m.group(1) if m else conf["graph"]) == g:
                texts.append(t)
        hand[g] = "\n".join(texts)
    keys_by_graph = {}
    for old in glob.glob(os.path.join(fdir, "90_derived*.cypher")):
        os.remove(old)
    virtual = set()
    for t in list(hand.values()) + [e["text"] for e in entries]:
        virtual.update(re.findall(r"CREATE\s+VIRTUAL\s+NODE\s+`?(\w+)", t, re.I))
    defaults = {k: v for k, v in assembly.get("fixture_defaults", {}).get(name, {}).items()}
    for g in graphs:
        text, keys = derive_fixture(models[g], hand[g], g, virtual, defaults)
        keys_by_graph[g] = keys
        fn = "90_derived.cypher" if g == conf["graph"] else f"90_derived_{g}.cypher"
        with open(os.path.join(fdir, fn), "w", encoding="utf-8") as fh:
            fh.write(f"// graph: {g}\n" + text)

    overrides = assembly.get("params", {})
    queries = []
    for e in entries:
        with open(os.path.join(qdir, e["id"] + ".cypher"), "w", encoding="utf-8") as fh:
            fh.write(e["text"].rstrip() + "\n")
        prm = params_for(e["text"], e["kind"], models[e["graph"]], keys_by_graph[e["graph"]],
                         assembly.get("param_names", {}).get(name, {}))
        for key in [e["id"]] + e["origins"]:
            prm.update(overrides.get(key, {}))
        if prm:
            dump(os.path.join(pdir, e["id"] + ".json"), prm)
        q = {"id": e["id"], "kind": e["kind"], "graph": e["graph"], "origins": e["origins"],
             "params": e["params"]}
        if e["placeholders"]:
            q["placeholders"] = e["placeholders"]
        if e.get("assembly_rule"):
            q["assembly_rule"] = e["assembly_rule"]
        if set(e["contexts"]) == {"llm-example"}:
            q["llm_example"] = True
        queries.append(q)

    # writes go into ordered scenarios, one per source file
    scen: dict[str, dict] = {}
    for q, e in zip(queries, entries):
        if q["kind"] in ("read", "fabric-read"):
            continue
        sname = scenario_name(e["origins"][0])
        key = (sname, q["graph"])
        s = scen.setdefault(key, {"name": sname if q["graph"] == conf["graph"] else f"{sname}@{q['graph']}",
                                  "graph": q["graph"], "isolation": "rollback", "steps": []})
        s["steps"].append((phase(e["text"]), int(e["origins"][0].rsplit(":", 1)[1]), q["id"]))
        q["scenario"] = s["name"]
    scenarios = []
    for s in sorted(scen.values(), key=lambda s: s["name"]):
        s["steps"] = [i for _, _, i in sorted(s["steps"])]
        scenarios.append(s)

    manifest = {
        "client": name,
        "graphs": graphs,
        "sources": [{k: v for k, v in s.items()} for s in sources],
        "vector_dimensions": VECTOR_DIM,
        "fixture": sorted(os.path.basename(f) for f in glob.glob(os.path.join(fdir, "*.cypher"))),
        "counts": {
            "queries": len(queries),
            "by_kind": {k: sum(1 for q in queries if q["kind"] == k) for k in sorted({q["kind"] for q in queries})},
            "scenarios": len(scenarios),
            "assembled": sum(1 for q in queries if q.get("assembly_rule")),
            "llm_examples": sum(1 for q in queries if q.get("llm_example")),
            "excluded": len(excluded),
        },
        "queries": queries,
        "scenarios": scenarios,
    }
    dump(os.path.join(outdir, "manifest.json"), manifest)
    dump(os.path.join(outdir, "excluded.json"), sorted(excluded, key=lambda x: x["origin"]))
    print(f"  wrote {outdir}: {len(queries)} queries, {len(scenarios)} scenarios, fixture {manifest['fixture']}")
