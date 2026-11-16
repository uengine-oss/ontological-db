"""How a Bolt result becomes something two runs (and two engines) can be compared on.

The rules, in the order they apply (README "정규화 규칙" says the same):

1. Graph values become plain JSON: a node is {"~node": id, "labels": sorted, "props": {…}}, a
   relationship {"~rel": id, "type", "start", "end", "props"}, a path {"~path": [...]}.
2. Row order: rows are kept in the order the server sent them only when the last top-level RETURN
   has an ORDER BY. Otherwise they are sorted by their JSON with every id masked.
3. Lists made by collect(), labels(), keys() (the column's expression says so) are sorted the
   same way, unless a WITH … ORDER BY comes before the RETURN.
4. Ids: node / relationship ids and elementIds are replaced by @n1, @n2 … / @r1 … in the order
   they are first met (after 2 and 3). Plain integers and strings are ids when they are the id
   or elementId of a node or relationship in the graph at that moment and the column's
   expression mentions id( / elementId( — or, for a string, it is one of those elementIds.
   Numbering restarts for every read and for every scenario (a scenario keeps its numbering
   across its steps, so the same node is the same @n in every step).
5. The clock: temporal values and ISO-8601 strings (dates too) within two days of the run become
   "<now>" (with the type for temporal values); epoch milliseconds within two days become "<now:ms>".
   ISO strings are parsed here, not with datetime.fromisoformat (which differs before Python 3.11).
   The run fixes the session TimeZone to UTC, so times far from now are written the same everywhere.
6. randomUUID(): strings shaped like a UUID become <uuid1>, <uuid2> … in first-seen order.
7. Floats keep 6 significant digits.
8. A failure is {"error": {"code": …, "message": …}}; digit runs of 9 or more in the message
   become <n> (they are ids or oids).
9. The connection: the PostgreSQL / Bolt port, host:port and the database name the run was given
   become <pg-port>, <pg-host>:<pg-port>, <pg-database>, <bolt-port> (fabric statements show them).
"""
from __future__ import annotations

import datetime as dt
import hashlib
import json
import math
import re

UUID = re.compile(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
ISO = re.compile(r"^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:[.,](\d+))?)?"
                 r"(Z|[+-]\d{2}(?::?\d{2})?)?(?:\[[^\]]+\])?)?$")


def parse_iso(s: str):
    """ISO-8601 date or date-time -> aware datetime (a time without an offset is UTC), or None.

    Parsed by hand: datetime.fromisoformat() before Python 3.11 refuses fractions that are not 3 or 6
    digits long (PostgreSQL drops trailing zeros: .3238) and the `Z` suffix, and the answers must not
    depend on the Python that ran them."""
    m = ISO.match(s)
    if not m:
        return None
    y, mo, d, hh, mi, ss, frac, off = m.groups()
    try:
        tz = dt.timezone.utc
        if off and off != "Z":
            sign = -1 if off[0] == "-" else 1
            digits = off[1:].replace(":", "")
            tz = dt.timezone(sign * dt.timedelta(hours=int(digits[:2]), minutes=int(digits[2:4] or 0)))
        micro = int((frac or "0")[:6].ljust(6, "0"))
        return dt.datetime(int(y), int(mo), int(d), int(hh or 0), int(mi or 0), int(ss or 0), micro, tzinfo=tz)
    except ValueError:
        return None
MASK = "@"

# what the run was connected to (run.py sets it): values that differ only because a run used another
# port or database name. {"pg_host", "pg_port", "pg_database", "bolt_port"}
CONNECTION: dict = {}


def set_connection(**kw):
    CONNECTION.clear()
    CONNECTION.update({k: v for k, v in kw.items() if v not in (None, "")})


def _conn_text(s: str) -> str:
    c = CONNECTION
    if not c:
        return s
    if c.get("pg_host") and c.get("pg_port"):
        s = s.replace(f"{c['pg_host']}:{c['pg_port']}", "<pg-host>:<pg-port>")
    if c.get("pg_port"):
        s = re.sub(r"(?<![\d.])" + re.escape(str(c["pg_port"])) + r"(?![\d.])", "<pg-port>", s)
    if c.get("bolt_port"):
        s = re.sub(r"(?<![\d.])" + re.escape(str(c["bolt_port"])) + r"(?![\d.])", "<bolt-port>", s)
    if c.get("pg_database"):
        s = re.sub(r"(?<![\w])" + re.escape(c["pg_database"]) + r"(?![\w])", "<pg-database>", s)
    return s


def _now():
    return dt.datetime.now(dt.timezone.utc)


class Normalizer:
    def __init__(self, known: dict[str, str] | None = None, now: dt.datetime | None = None):
        # known: elementId (str) -> "n" | "r"
        self.known = dict(known or {})
        self.now = now or _now()
        self.tokens: dict[tuple[str, str], str] = {}
        self.counts = {"n": 0, "r": 0}
        self.uuids: dict[str, str] = {}

    def learn(self, known: dict[str, str]):
        self.known.update(known)

    # -- clock -------------------------------------------------------------
    def near_now(self, d: dt.datetime) -> bool:
        if d.tzinfo is None:
            d = d.replace(tzinfo=dt.timezone.utc)
        return abs((d - self.now).total_seconds()) < 2 * 86400

    def _iso_now(self, s: str) -> bool:
        d = parse_iso(s)
        if d is None:
            return False
        if len(s) == 10:  # a date: today or the days next to it, in any time zone
            return abs((d.date() - self.now.date()).days) <= 2
        return self.near_now(d)

    # -- ids ----------------------------------------------------------------
    def token(self, kind: str, eid: str) -> str:
        key = (kind, str(eid))
        if key not in self.tokens:
            self.counts[kind] += 1
            self.tokens[key] = f"@{kind}{self.counts[kind]}"
        return self.tokens[key]

    # -- values: first to "raw" JSON (ids kept, marked), then masked / tokenised --------
    def raw(self, v, id_column: bool = False):
        """Bolt value -> JSON-able, with ids kept as {"~id": kind, "v": eid}."""
        from neo4j.graph import Node, Path, Relationship
        try:
            from neo4j.time import Date, DateTime, Duration, Time
        except ImportError:  # pragma: no cover
            Date = DateTime = Duration = Time = ()
        if v is None or isinstance(v, bool):
            return v
        if isinstance(v, Node):
            return {"~node": {"~id": "n", "v": v.element_id}, "labels": sorted(v.labels),
                    "props": {k: self.raw(x) for k, x in sorted(dict(v).items())}}
        if isinstance(v, Relationship):
            return {"~rel": {"~id": "r", "v": v.element_id}, "type": v.type,
                    "start": {"~id": "n", "v": v.start_node.element_id} if v.start_node is not None else None,
                    "end": {"~id": "n", "v": v.end_node.element_id} if v.end_node is not None else None,
                    "props": {k: self.raw(x) for k, x in sorted(dict(v).items())}}
        if isinstance(v, Path):
            out = []
            for i, n in enumerate(v.nodes):
                out.append(self.raw(n))
                if i < len(v.relationships):
                    out.append(self.raw(v.relationships[i]))
            return {"~path": out}
        if DateTime and isinstance(v, DateTime):
            native = v.to_native()
            return {"~temporal": "DateTime", "v": "<now>" if self.near_now(native) else v.iso_format()}
        if Date and isinstance(v, Date):
            near = abs((v.to_native() - self.now.date()).days) <= 2
            return {"~temporal": "Date", "v": "<now>" if near else v.iso_format()}
        if Time and isinstance(v, Time):
            return {"~temporal": "Time", "v": v.iso_format()}
        if Duration and isinstance(v, Duration):
            return {"~temporal": "Duration", "v": str(v)}
        if isinstance(v, int):
            if CONNECTION and v in (CONNECTION.get("pg_port"), CONNECTION.get("bolt_port")):
                return "<pg-port>" if v == CONNECTION.get("pg_port") else "<bolt-port>"
            s = str(v)
            if s in self.known and (id_column or abs(v) >= 2 ** 31):
                return {"~id": self.known[s], "v": s, "int": True}
            if abs(v - self.now.timestamp() * 1000) < 2 * 86400 * 1000:
                return "<now:ms>"
            return v
        if isinstance(v, float):
            if math.isnan(v) or math.isinf(v):
                return {"~float": str(v)}
            if v == 0:
                return 0.0
            return float(f"{v:.6g}")
        if isinstance(v, str):
            if v in self.known:
                return {"~id": self.known[v], "v": v}
            if UUID.match(v):
                return {"~uuid": v}
            if self._iso_now(v):
                return "<now>"
            return _conn_text(v)
        if isinstance(v, (list, tuple)):
            return [self.raw(x, id_column) for x in v]
        if isinstance(v, dict):
            return {str(k): self.raw(x, id_column) for k, x in sorted(v.items(), key=lambda kv: str(kv[0]))}
        if isinstance(v, (bytes, bytearray)):
            return {"~bytes": bytes(v).hex()}
        # points, unknown structures
        return {"~other": type(v).__name__, "v": str(v)}

    def mask(self, v):
        if isinstance(v, dict):
            if "~id" in v and set(v) <= {"~id", "v", "int"}:
                return MASK + v["~id"]
            if set(v) == {"~uuid"}:
                return "<uuid>"
            return {k: self.mask(x) for k, x in v.items()}
        if isinstance(v, list):
            return [self.mask(x) for x in v]
        return v

    def sort_lists(self, v):
        if isinstance(v, dict):
            if ("~id" in v and set(v) <= {"~id", "v", "int"}) or set(v) == {"~uuid"}:
                return v
            return {k: self.sort_lists(x) for k, x in v.items()}
        if isinstance(v, list):
            items = [self.sort_lists(x) for x in v]
            return sorted(items, key=lambda x: canon(self.mask(x)))
        return v

    def finish(self, v):
        if isinstance(v, dict):
            if "~id" in v and set(v) <= {"~id", "v", "int"}:
                return self.token(v["~id"], v["v"])
            if set(v) == {"~uuid"}:
                u = v["~uuid"]
                if u not in self.uuids:
                    self.uuids[u] = f"<uuid{len(self.uuids) + 1}>"
                return self.uuids[u]
            return {k: self.finish(x) for k, x in v.items()}
        if isinstance(v, list):
            return [self.finish(x) for x in v]
        return v


def canon(v) -> str:
    return json.dumps(v, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def sha1(v) -> str:
    return hashlib.sha1(canon(v).encode()).hexdigest()


def error_of(e) -> dict:
    code = getattr(e, "code", None) or type(e).__name__
    msg = str(getattr(e, "message", None) or e)
    msg = _conn_text(re.sub(r"\d{9,}", "<n>", msg))
    msg = re.sub(r"\s+", " ", msg).strip()[:400]
    return {"code": code, "message": msg}
