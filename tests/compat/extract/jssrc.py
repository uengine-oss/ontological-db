"""JavaScript sources -> Cypher candidates (static).

A small scanner, not a parser: it walks the file once, skips comments and regular
expression literals, and yields every string literal ('…', "…") and template literal
(`…`). A template's ``${expr}`` parts are replaced from the assembly rules (keyed by
the expression text); anything else becomes ``⟦expr⟧``. Adjacent literals joined
with ``+`` on the same expression are concatenated.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field

from pysrc import UNRESOLVED_CLOSE, UNRESOLVED_OPEN

REGEX_PRECEDERS = set("(,=:[!&|?{};+-*%<>~^") | {""}


@dataclass
class JsLiteral:
    text: str
    line: int
    end_line: int
    unresolved: list[str] = field(default_factory=list)
    placeholders: dict[str, str] = field(default_factory=dict)


def _line_of(src: str, i: int) -> int:
    return src.count("\n", 0, i) + 1


def _scan_template(src: str, i: int, rules: dict[str, str], lit: JsLiteral) -> int:
    """src[i] is the opening backtick. Returns index after the closing one."""
    i += 1
    out = []
    while i < len(src):
        c = src[i]
        if c == "\\":
            nxt = src[i + 1]
            out.append({"n": "\n", "t": "\t", "`": "`", "\\": "\\", "$": "$"}.get(nxt, "\\" + nxt))
            i += 2
            continue
        if c == "`":
            lit.text += "".join(out)
            return i + 1
        if c == "$" and src[i + 1: i + 2] == "{":
            depth, j = 1, i + 2
            while j < len(src) and depth:
                if src[j] == "{":
                    depth += 1
                elif src[j] == "}":
                    depth -= 1
                elif src[j] == "/" and src[j - 1] in "(,=:[!&|?" and src[j + 1] not in "/*":
                    j += 1  # a regular expression literal inside the expression
                    while j < len(src) and src[j] not in "/\n":
                        j += 2 if src[j] == "\\" else 1
                elif src[j] == "`":  # nested template inside the expression
                    inner = JsLiteral("", 0, 0)
                    j = _scan_template(src, j, rules, inner) - 1
                elif src[j] in "'\"":
                    q = src[j]
                    j += 1
                    while j < len(src) and src[j] != q:
                        j += 2 if src[j] == "\\" else 1
                j += 1
            expr = src[i + 2: j - 1].strip()
            at = f"@{lit.line}:{expr}"
            if at in rules:
                lit.placeholders[expr] = rules[at]
                out.append(rules[at])
            elif expr in rules:
                lit.placeholders[expr] = rules[expr]
                out.append(rules[expr])
            else:
                lit.unresolved.append(expr)
                out.append(f"{UNRESOLVED_OPEN}{expr}{UNRESOLVED_CLOSE}")
            i = j
            continue
        out.append(c)
        i += 1
    lit.text += "".join(out)
    return i


def literals(src: str, rules: dict[str, str]) -> list[JsLiteral]:
    out: list[JsLiteral] = []
    i, n = 0, len(src)
    prev = ""  # last significant char
    pending: JsLiteral | None = None  # literal that may continue with `+ '…'`
    while i < n:
        c = src[i]
        if c in " \t\r\n":
            i += 1
            continue
        if src.startswith("//", i):
            i = src.find("\n", i)
            i = n if i < 0 else i
            continue
        if src.startswith("/*", i):
            j = src.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c == "/" and (prev in REGEX_PRECEDERS or re.search(r"\b(return|typeof|case)\s*$", src[max(0, i - 10):i])):
            j, in_class = i + 1, False
            while j < n and src[j] != "\n":
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == "[":
                    in_class = True
                elif src[j] == "]":
                    in_class = False
                elif src[j] == "/" and not in_class:
                    break
                j += 1
            i = j + 1
            while i < n and src[i].isalpha():
                i += 1
            prev = "x"
            continue
        if c in "'\"`":
            start = i
            lit = JsLiteral("", _line_of(src, i), 0)
            if c == "`":
                i = _scan_template(src, i, rules, lit)
            else:
                j, buf = i + 1, []
                while j < n and src[j] != c:
                    if src[j] == "\\":
                        nxt = src[j + 1]
                        buf.append({"n": "\n", "t": "\t"}.get(nxt, nxt))
                        j += 2
                        continue
                    buf.append(src[j])
                    j += 1
                lit.text = "".join(buf)
                i = j + 1
            lit.end_line = _line_of(src, i - 1)
            # `a' + 'b` on one expression: glue to the previous literal
            between = src[pending_end:start] if pending is not None else None
            if pending is not None and between is not None and re.fullmatch(r"\s*\+\s*", between):
                pending.text += lit.text
                pending.end_line = lit.end_line
                pending.unresolved += lit.unresolved
                pending.placeholders.update(lit.placeholders)
            else:
                if pending is not None:
                    out.append(pending)
                pending = lit
            pending_end = i
            prev = "x"
            continue
        prev = c
        i += 1
    if pending is not None:
        out.append(pending)
    return out
