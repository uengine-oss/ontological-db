"""Light, regex-level reading of a Cypher text — enough to sort and prepare the corpus.

Not a parser. It answers: is this text a whole Cypher statement, is it a read or a
write, which ``$parameters`` does it take, which labels / relationship types / properties
does it name, and how does its last RETURN look (columns, ORDER BY, LIMIT, collect()).
The extractor and the runner both use it, so the rules live in one place.
"""
from __future__ import annotations

import re

START = re.compile(
    r"^\s*(?:(?://[^\n]*\n|/\*.*?\*/)\s*)*"
    r"(OPTIONAL\s+MATCH|MATCH|MERGE|CREATE|UNWIND|WITH|CALL|RETURN|SHOW|DROP|FOREACH|USE|EXPLAIN|PROFILE"
    r"|LOAD\s+CSV|REFRESH|ALTER\s+DATASOURCE|DETACH\s+DELETE)\b",
    re.I | re.S,
)
SQL_CREATE = re.compile(
    r"^\s*CREATE\s+(OR\s+REPLACE\s+|TEMP(ORARY)?\s+|UNLOGGED\s+|UNIQUE\s+INDEX\s+\w+\s+ON\s+\w|FOREIGN\s+TABLE|TABLE|VIEW|SCHEMA|EXTENSION|DATABASE|FUNCTION|ROLE|USER\b|SERVER|TRIGGER|SEQUENCE|POLICY|PUBLICATION|MATERIALIZED|TYPE\s)",
    re.I,
)
SQL_DROP = re.compile(r"^\s*DROP\s+(TABLE|VIEW|SCHEMA|EXTENSION|DATABASE|FUNCTION|ROLE|USER|SERVER|TRIGGER|SEQUENCE|POLICY|FOREIGN|MATERIALIZED|TYPE\s|OWNED)", re.I)
FABRIC_CREATE = re.compile(r"^\s*CREATE\s+(DATASOURCE|VIRTUAL|GRAPH|CREDENTIAL)", re.I)
SCHEMA_DDL = re.compile(
    r"^\s*(CREATE|DROP)\s+(CONSTRAINT|(VECTOR\s+|FULLTEXT\s+|TEXT\s+|RANGE\s+|POINT\s+|LOOKUP\s+)?INDEX)\b", re.I)


def mask_strings(text: str) -> str:
    """Replace the inside of '…' and "…" with x's (same length) and blank // comments.

    `…` is an identifier quote in Cypher, not a string: it is kept."""
    out, i, n = [], 0, len(text)
    while i < n:
        c = text[i]
        if c == "`":
            j = text.find("`", i + 1)
            j = n - 1 if j < 0 else j
            out.append(text[i:j + 1])
            i = j + 1
            continue
        if c in "'\"":
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == "\\" else 1
            out.append(c + "x" * (min(j, n) - i - 1) + (c if j < n else ""))
            i = j + 1
            continue
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            out.append(" " * (j - i))
            i = j
            continue
        out.append(c)
        i += 1
    return "".join(out)


def looks_like_cypher(text: str) -> tuple[bool, str]:
    t = text.strip()
    if not t or "⟦" in t and not START.match(t.replace("⟦", "").lstrip()):
        pass
    m = START.match(t)
    if not m:
        return False, "no clause keyword at the start"
    first = m.group(1).upper()
    masked = mask_strings(t)
    if first == "CREATE" and SQL_CREATE.match(t) and not FABRIC_CREATE.match(t):
        return False, "SQL CREATE"
    if first == "DROP" and SQL_DROP.match(t):
        return False, "SQL DROP"
    if first == "CREATE" and re.match(r"^\s*CREATE\s+INDEX\s+\w+\s+ON\s+[\w.]+\s*\(", t, re.I):
        return False, "SQL CREATE INDEX"
    if first == "WITH" and re.search(r"\bAS\s*\(\s*SELECT\b", masked, re.I):
        return False, "SQL CTE"
    if first == "WITH" and re.match(r"^\s*WITH\s+\w+\s*(,|$)", t) and not re.search(r"\b(MATCH|RETURN|MERGE|UNWIND|CREATE)\b", masked):
        return False, "prose"
    if re.search(r"\bSELECT\b[\s\S]*\bFROM\b", masked, re.I) and not re.search(
            r"\b(MATCH|MERGE|RETURN|UNWIND|SHOW|DATASOURCE|VIRTUAL)\b", masked, re.I):
        return False, "SQL SELECT"
    # natural language that happens to start with a keyword ("Match the …")
    words = re.findall(r"[A-Za-z]+", masked[:200])
    if first == "USE" and not re.match(r"^\s*USE\s+[\w`.]+\s+(OPTIONAL\s+MATCH|MATCH|MERGE|CREATE|UNWIND|WITH|CALL|RETURN)\b", t, re.I):
        return False, "not a USE clause"
    if first in ("MATCH", "CREATE", "RETURN", "WITH", "USE", "CALL", "SHOW", "DROP") and len(words) > 3:
        if not re.search(r"[(){}\[\]$:]", masked[:400]) and first not in ("SHOW", "DROP") and not FABRIC_CREATE.match(t):
            return False, "prose"
    if not text.lstrip()[:len(m.group(1))].isupper() and not text.lstrip()[:len(m.group(1))].islower():
        # "Match" / "Create" — capitalised word, almost always prose
        return False, "capitalised word"
    if re.search(r"[ㄱ-힝]", masked[:40]) and not re.search(r"[(){}$]", masked[:80]):
        return False, "prose (Korean)"
    return True, first


def is_complete(text: str) -> bool:
    """A whole statement, not a piece the code glues to others later."""
    m = mask_strings(text)
    u = m.upper()
    first = START.match(text).group(1).upper() if START.match(text) else ""
    if first in ("SHOW", "DROP", "REFRESH", "ALTER DATASOURCE", "LOAD CSV", "USE", "EXPLAIN", "PROFILE"):
        return True
    if first == "CREATE" and (SCHEMA_DDL.match(text) or FABRIC_CREATE.match(text)):
        return True
    if re.search(r"\bRETURN\b", u):
        return True
    if re.search(r"\b(CREATE|MERGE|SET|DELETE|REMOVE|FOREACH)\b", u):
        return True
    if first == "CALL" and not re.search(r"\bYIELD\b", u):
        return True  # CALL proc() alone
    if first == "CALL" and re.search(r"\bYIELD\b", u) and not re.search(r"\bWITH\b\s*$", u):
        return True
    return False


WRITE_CLAUSE = re.compile(r"\b(CREATE|MERGE|SET|DELETE|REMOVE|FOREACH|DETACH)\b", re.I)
WRITE_PROC = re.compile(r"\bCALL\s+(apoc\.(create|merge|refactor|periodic|nodes\.delete)|db\.create)", re.I)


def kind(text: str) -> str:
    """read | write | schema | fabric-read | fabric-write"""
    t = text.strip()
    m = mask_strings(t)
    if SCHEMA_DDL.match(t):
        return "schema"
    if re.match(r"^\s*SHOW\s+(CONSTRAINTS|INDEXES|INDEX)\b", t, re.I):
        return "read"
    if re.match(r"^\s*SHOW\b", t, re.I):
        return "fabric-read"
    if re.match(r"^\s*(CREATE|DROP|REFRESH|ALTER)\s+(DATASOURCE|VIRTUAL|GRAPH|CREDENTIAL)", t, re.I) or re.match(
            r"^\s*(DROP|REFRESH|ALTER)\b", t, re.I):
        return "fabric-write"
    if WRITE_CLAUSE.search(m) or WRITE_PROC.search(m):
        return "write"
    return "read"


PARAM = re.compile(r"\$([A-Za-z_][A-Za-z0-9_]*)|\$`([^`]+)`")


def params(text: str) -> list[str]:
    seen = []
    for m in PARAM.finditer(mask_strings(text)):
        name = m.group(1) or m.group(2)
        if name not in seen:
            seen.append(name)
    return seen


# ---------------------------------------------------------------- patterns

NODE_BODY = re.compile(r"^\s*([A-Za-z_]\w*)?\s*((?::\s*`?[A-Za-z_][\w]*`?\s*)+)?\s*(\{.*\}|\$[A-Za-z_]\w*)?\s*$", re.S)
REL_BETWEEN = re.compile(r"^\s*(<?)-\s*(?:\[(.*)\])?\s*-(>?)\s*$", re.S)
REL_BODY = re.compile(r"^\s*([A-Za-z_]\w*)?\s*(:\s*[!`A-Za-z_|:\s]*[\w`])?\s*(\*[\d.\s]*)?\s*(\{.*\})?\s*$", re.S)


def _labels(s: str | None) -> list[str]:
    if not s:
        return []
    return [x.strip().strip("`") for x in s.split(":") if x.strip()]


CLAUSE_WORDS = {"MATCH", "MERGE", "CREATE", "OPTIONAL", "WHERE", "AND", "OR", "NOT", "XOR", "RETURN", "WITH",
                "IN", "EXISTS", "UNWIND", "DELETE", "DETACH", "SET", "FOREACH", "CASE", "WHEN", "THEN", "ELSE",
                "YIELD", "ON", "FOR", "IS", "AS", "DISTINCT", "BY", "UNION", "ALL", "CALL"}


def _paren_spans(m: str) -> list[tuple[int, int]]:
    """(start, end) of every parenthesis group that is not a call's argument list."""
    spans, stack = [], []
    for i, c in enumerate(m):
        if c == "(":
            j = i - 1
            while j >= 0 and m[j] in " \t\r\n":
                j -= 1
            call = j >= 0 and (m[j].isalnum() or m[j] in "_`.")
            if call and m[j] != "`":
                k = j
                while k >= 0 and (m[k].isalnum() or m[k] == "_"):
                    k -= 1
                if m[k + 1: j + 1].upper() in CLAUSE_WORDS:
                    call = False
            stack.append((i, call))
        elif c == ")" and stack:
            s, call = stack.pop()
            if not call:
                spans.append((s, i + 1))
    spans.sort()
    return spans


def _nodes(text: str):
    m = mask_strings(text)
    out = []
    for s, e in _paren_spans(m):
        g = NODE_BODY.match(m[s + 1: e - 1])
        if not g:
            continue
        props = text[s + 1 + g.start(3): s + 1 + g.end(3)] if g.group(3) else ""
        out.append((s, e, g.group(1), _labels(g.group(2)), props))
    return out


def node_patterns(text: str) -> list[tuple[str | None, list[str], str]]:
    return [(v, l, p) for _, _, v, l, p in _nodes(text) if l or p]


def var_labels(text: str) -> dict[str, list[str]]:
    """variable -> the longest label list it is given anywhere in the statement."""
    out: dict[str, list[str]] = {}
    for var, labels, _ in node_patterns(text):
        if var and labels and len(labels) > len(out.get(var, [])):
            out[var] = labels
    m = mask_strings(text)
    for g in re.finditer(r"\b([A-Za-z_]\w*)\s*:\s*`?([A-Z_][\w]*)`?", m):
        pre = m[max(0, g.start() - 6): g.start()]
        if re.search(r"(WHERE|AND|OR|NOT|\()\s*$", pre, re.I) and g.group(1) not in out:
            out[g.group(1)] = [g.group(2)]
    return out


def rel_patterns(text: str) -> list[dict]:
    m = mask_strings(text)
    vl = var_labels(text)
    nodes = _nodes(text)
    out = []
    for (s1, e1, v1, l1, _), (s2, e2, v2, l2, _) in zip(nodes, nodes[1:]):
        g = REL_BETWEEN.match(m[e1:s2])
        if not g:
            continue
        left, body, right = g.group(1), g.group(2) or "", g.group(3)
        rb = REL_BODY.match(body) if body.strip() else None
        types = []
        hops = False
        if rb:
            types = [t.strip().strip("`") for t in re.split(r"[|:]", (rb.group(2) or "").lstrip(":")) if t.strip()]
            hops = bool(rb.group(3))
        a = l1 or vl.get(v1 or "", [])
        b = l2 or vl.get(v2 or "", [])
        if left and not right:
            a, b = b, a
        va, vb = v1, v2
        if left and not right:
            va, vb = vb, va
        out.append({"from": a, "types": types, "to": b, "varlen": hops, "directed": bool(left) != bool(right),
                    "from_var": va, "to_var": vb})
    return out


PROP_ACCESS = re.compile(r"\b([A-Za-z_]\w*)\.([A-Za-z_]\w*)\b")


def property_uses(text: str) -> dict[str, set[str]]:
    """label -> property names read or written through a variable of that label."""
    vl = var_labels(text)
    out: dict[str, set[str]] = {}
    m = mask_strings(text)
    for g in PROP_ACCESS.finditer(m):
        var, prop = g.group(1), g.group(2)
        if var in vl:
            out.setdefault(vl[var][-1], set()).add(prop)
    for var, labels, props in node_patterns(text):
        if labels and props:
            for k in re.findall(r"([A-Za-z_]\w*)\s*:", mask_strings(props)):
                out.setdefault(labels[-1], set()).add(k)
    return out


def literal_equalities(text: str) -> list[tuple[str, str, object]]:
    """(label, property, literal) from {p: 'x'} and v.p = 'x' — values a fixture should hold."""
    vl = var_labels(text)
    out = []
    lit = r"('(?:[^'\\]|\\.)*'|\"(?:[^\"\\]|\\.)*\"|-?\d+(?:\.\d+)?|true|false)"
    for var, labels, props in node_patterns(text):
        if labels and props:
            for k, v in re.findall(r"([A-Za-z_]\w*)\s*:\s*" + lit, props):
                out.append((labels[-1], k, _lit(v)))
    for g in re.finditer(r"\b([A-Za-z_]\w*)\.([A-Za-z_]\w*)\s*(?:=|IN\s*\[)\s*" + lit, text):
        var = g.group(1)
        if var in vl:
            out.append((vl[var][-1], g.group(2), _lit(g.group(3))))
    return out


def _lit(v: str):
    if v[0] in "'\"":
        return v[1:-1].replace("\\'", "'").replace('\\"', '"')
    if v in ("true", "false"):
        return v == "true"
    return float(v) if "." in v else int(v)


# ---------------------------------------------------------------- the last RETURN


def split_top(s: str, sep: str = ",") -> list[str]:
    parts, depth, cur = [], 0, []
    ms = mask_strings(s)
    for i, c in enumerate(ms):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        if c == sep and depth == 0:
            parts.append("".join(cur))
            cur = []
            continue
        cur.append(s[i])
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


def _top_level_positions(masked: str, word: str) -> list[int]:
    """Positions of a keyword outside any (), [], {}."""
    out, depth = [], 0
    rx = re.compile(r"\b" + word + r"\b", re.I)
    marks = {m.start() for m in rx.finditer(masked)}
    for i, c in enumerate(masked):
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif depth == 0 and i in marks:
            out.append(i)
    return out


def last_return(text: str) -> dict:
    """Columns of the final projection and what decides their order.

    ``ordered``: an ORDER BY follows the last top-level RETURN.
    ``limit``: a LIMIT follows it (without ORDER BY the subset is not defined).
    ``ordered_collect``: a WITH … ORDER BY comes before the RETURN (collect() keeps it).
    ``columns``: [(expression, alias)]
    """
    m = mask_strings(text)
    rets = _top_level_positions(m, "RETURN")
    info = {"columns": [], "ordered": False, "limit": False, "ordered_collect": False, "union": False}
    if not rets:
        return info
    info["union"] = bool(_top_level_positions(m, "UNION"))
    r = rets[-1]
    tail_m = m[r + 6:]
    tail = text[r + 6:]
    cut = len(tail_m)
    for kw in ("ORDER", "SKIP", "LIMIT"):
        p = _top_level_positions(tail_m, kw)
        if p:
            cut = min(cut, p[0])
    info["ordered"] = bool(_top_level_positions(tail_m, "ORDER"))
    info["order_by"] = []
    op = _top_level_positions(tail_m, "ORDER")
    if op:
        rest = tail[op[0]:]
        rest = re.sub(r"^ORDER\s+BY\s+", "", rest, flags=re.I)
        rm = mask_strings(rest)
        end = len(rm)
        for kw in ("SKIP", "LIMIT", "UNION"):
            p = _top_level_positions(rm, kw)
            if p:
                end = min(end, p[0])
        info["order_by"] = [re.sub(r"\s+(ASC|DESC|ASCENDING|DESCENDING)\s*$", "", x, flags=re.I).strip()
                            for x in split_top(rest[:end])]
    info["limit"] = bool(_top_level_positions(tail_m, "LIMIT"))
    before = m[:r]
    info["ordered_collect"] = bool(re.search(r"\bORDER\s+BY\b", before, re.I))
    body = tail[:cut].strip()
    body = re.sub(r"^DISTINCT\b", "", body, flags=re.I).strip()
    cols = []
    for part in split_top(body):
        mm = re.match(r"(.*?)\s+AS\s+`?([^`\s]+)`?\s*$", part, re.S | re.I)
        if mm:
            cols.append((mm.group(1).strip(), mm.group(2)))
        else:
            cols.append((part, part))
    info["columns"] = cols
    return info
