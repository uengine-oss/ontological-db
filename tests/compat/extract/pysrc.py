"""Python sources -> Cypher candidates (static, no import of the client code).

Every *maximal* string expression in a module — a literal, an f-string, implicit
concatenation, ``a + b`` of strings, ``"sep".join((...))`` of constants,
``textwrap.dedent(...)`` / ``.strip()`` around one of those — is evaluated to text.
Names inside it are looked up, in this order:

1. the assembly rules (``assembly.json``: per file, then per client),
2. a single string assignment to that name in the enclosing function,
3. a module-level (or class-level) string constant of this module,
4. a constant imported from another module of the same source root
   (``from x.y import NAME``, relative imports included).

Whatever is left becomes ``⟦expr⟧`` and the candidate is reported as unresolved, so
the rule table can be completed by hand. Nothing here imports or runs client code.
"""
from __future__ import annotations

import ast
import os
import re
from dataclasses import dataclass, field

UNRESOLVED_OPEN, UNRESOLVED_CLOSE = "⟦", "⟧"  # ⟦ ⟧


@dataclass
class Piece:
    text: str
    unresolved: list[str] = field(default_factory=list)
    placeholders: dict[str, str] = field(default_factory=dict)  # expr -> value used


class Unknown(Exception):
    pass


class ModuleIndex:
    """Module-level string constants of every module under one source root, lazily."""

    def __init__(self, root: str):
        self.root = root
        self._consts: dict[str, dict[str, object]] = {}
        self._imports: dict[str, dict[str, tuple[str, str]]] = {}
        self._loading: set[str] = set()

    # -- paths -------------------------------------------------------------
    def module_file(self, dotted: str) -> str | None:
        base = os.path.join(self.root, *dotted.split("."))
        for cand in (base + ".py", os.path.join(base, "__init__.py")):
            if os.path.isfile(cand):
                return cand
        return None

    def dotted_of(self, path: str) -> str:
        rel = os.path.relpath(path, self.root)[:-3]
        parts = rel.split(os.sep)
        if parts[-1] == "__init__":
            parts = parts[:-1]
        return ".".join(parts)

    def resolve_from(self, path: str, node: ast.ImportFrom) -> str | None:
        if node.level == 0:
            return node.module
        pkg = self.dotted_of(path).split(".")
        if not path.endswith("__init__.py"):
            pkg = pkg[:-1]
        if node.level > 1:
            pkg = pkg[: len(pkg) - (node.level - 1)]
        return ".".join(pkg + ([node.module] if node.module else []))

    # -- constants ----------------------------------------------------------
    def consts_of_file(self, path: str) -> dict[str, object]:
        key = os.path.abspath(path)
        if key in self._consts:
            return self._consts[key]
        if key in self._loading:
            return {}
        self._loading.add(key)
        consts: dict[str, object] = {}
        imports: dict[str, tuple[str, str]] = {}
        try:
            tree = ast.parse(open(path, encoding="utf-8").read())
        except (SyntaxError, UnicodeDecodeError, OSError):
            tree = ast.Module(body=[], type_ignores=[])
        # imports anywhere in the module (function-level `from x import NAME` too)
        for stmt in ast.walk(tree):
            if isinstance(stmt, ast.ImportFrom):
                mod = self.resolve_from(path, stmt)
                if mod:
                    for a in stmt.names:
                        imports[a.asname or a.name] = (mod, a.name)
        self._imports[key] = imports
        self._consts[key] = consts  # visible (partially) to recursive lookups

        def scan(body, prefix=""):
            for stmt in body:
                targets, value = [], None
                if isinstance(stmt, ast.Assign):
                    targets, value = stmt.targets, stmt.value
                elif isinstance(stmt, ast.AnnAssign) and stmt.value is not None:
                    targets, value = [stmt.target], stmt.value
                elif isinstance(stmt, ast.ClassDef):
                    scan(stmt.body, prefix + stmt.name + ".")
                    continue
                for t in targets:
                    if isinstance(t, ast.Name):
                        try:
                            consts[prefix + t.id] = self.const_value(value, path, consts)
                        except Unknown:
                            pass

        scan(tree.body)
        self._loading.discard(key)
        return consts

    def imported(self, path: str, name: str) -> object:
        self.consts_of_file(path)
        imp = self._imports.get(os.path.abspath(path), {}).get(name)
        if not imp:
            raise Unknown(name)
        mod, attr = imp
        f = self.module_file(mod)
        if f is None:
            # `from pkg import submodule` – not a constant
            raise Unknown(name)
        consts = self.consts_of_file(f)
        if attr in consts:
            return consts[attr]
        return self.imported(f, attr)  # re-exported

    def const_value(self, node, path, local_consts) -> object:
        """Evaluate a module-level expression made only of constants. Raises Unknown."""
        if isinstance(node, ast.Constant) and isinstance(node.value, (str, int, float, bool)):
            return node.value
        if isinstance(node, ast.Name):
            if node.id in local_consts:
                return local_consts[node.id]
            return self.imported(path, node.id)
        if isinstance(node, ast.Attribute) and isinstance(node.value, ast.Name):
            dotted = f"{node.value.id}.{node.attr}"
            if dotted in local_consts:
                return local_consts[dotted]
            raise Unknown(dotted)
        if isinstance(node, (ast.Tuple, ast.List, ast.Set)):
            return tuple(self.const_value(e, path, local_consts) for e in node.elts)
        if isinstance(node, ast.JoinedStr):
            out = []
            for v in node.values:
                if isinstance(v, ast.Constant):
                    out.append(v.value)
                elif isinstance(v, ast.FormattedValue):
                    val = self.const_value(v.value, path, local_consts)
                    out.append(repr(val) if v.conversion == ord("r") else str(val))
            return "".join(out)
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
            a = self.const_value(node.left, path, local_consts)
            b = self.const_value(node.right, path, local_consts)
            if isinstance(a, str) and isinstance(b, str):
                return a + b
            if isinstance(a, tuple) and isinstance(b, tuple):
                return a + b
            raise Unknown("binop")
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr == "join" and isinstance(node.func.value, ast.Constant) and node.args:
                seq = self.const_value(node.args[0], path, local_consts)
                if isinstance(seq, tuple) and all(isinstance(s, str) for s in seq):
                    return node.func.value.value.join(seq)
            if node.func.attr in ("strip", "lstrip", "rstrip") and not node.args:
                v = self.const_value(node.func.value, path, local_consts)
                if isinstance(v, str):
                    return getattr(v, node.func.attr)()
        raise Unknown(type(node).__name__)


STRING_NODES = (ast.Constant, ast.JoinedStr, ast.BinOp, ast.Call)


def _is_str_expr(node) -> bool:
    if isinstance(node, ast.Constant):
        return isinstance(node.value, str)
    if isinstance(node, ast.JoinedStr):
        return True
    if isinstance(node, ast.BinOp) and isinstance(node.op, (ast.Add, ast.Mod)):
        return _is_str_expr(node.left) or (isinstance(node.op, ast.Add) and _is_str_expr(node.right))
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
        if node.func.attr in ("strip", "lstrip", "rstrip", "format") and _is_str_expr(node.func.value):
            return True
        if node.func.attr == "dedent" and node.args and _is_str_expr(node.args[0]):
            return True
    return False


class Evaluator:
    def __init__(self, index: ModuleIndex, path: str, rules: dict[str, str], func_locals: dict[str, ast.AST]):
        self.index = index
        self.path = path
        self.rules = rules
        self.func_locals = func_locals
        self._resolving: set[str] = set()
        self.module_consts = index.consts_of_file(path)

    line = 0  # the candidate being evaluated (for `@<line>:<expr>` rules)

    def value_of(self, expr: ast.AST, piece: Piece) -> str:
        src = ast.unparse(expr)
        at = f"@{self.line}:{src}"
        if at in self.rules:
            piece.placeholders[src] = self.rules[at]
            return self.rules[at]
        if src in self.rules:
            piece.placeholders[src] = self.rules[src]
            return self.rules[src]
        try:
            v = self.index.const_value(expr, self.path, self.module_consts)
            if isinstance(v, (str, int, float)) and not isinstance(v, bool):
                return str(v)
            if isinstance(v, tuple):
                raise Unknown("tuple")
        except Unknown:
            pass
        if (isinstance(expr, ast.Call) and isinstance(expr.func, ast.Attribute) and expr.func.attr == "format"
                and not expr.args and not _is_str_expr(expr.func.value)):
            probe = Piece("")
            base = self.value_of(expr.func.value, probe)
            if not probe.unresolved:
                kw = {k.arg: self.value_of(k.value, piece) for k in expr.keywords if k.arg}
                try:
                    return base.format(**kw)
                except (KeyError, IndexError, ValueError):
                    pass
        if isinstance(expr, ast.Name) and expr.id in self.func_locals and expr.id not in self._resolving:
            local = self.func_locals[expr.id]
            self._resolving.add(expr.id)
            try:
                return self._local_value(expr, local, piece, src)
            finally:
                self._resolving.discard(expr.id)
        piece.unresolved.append(src)
        return f"{UNRESOLVED_OPEN}{src}{UNRESOLVED_CLOSE}"

    def _local_value(self, expr, local, piece, src):
        if True:
            if _is_str_expr(local):
                sub = self.eval(local)
                piece.unresolved.extend(sub.unresolved)
                piece.placeholders.update(sub.placeholders)
                return sub.text
            try:
                v = self.index.const_value(local, self.path, self.module_consts)
                if isinstance(v, (str, int, float)):
                    return str(v)
            except Unknown:
                pass
        piece.unresolved.append(src)
        return f"{UNRESOLVED_OPEN}{src}{UNRESOLVED_CLOSE}"

    def eval(self, node) -> Piece:
        piece = Piece("")
        piece.text = self._eval(node, piece)
        return piece

    def _eval(self, node, piece: Piece) -> str:
        if isinstance(node, ast.Constant) and isinstance(node.value, str):
            return node.value
        if isinstance(node, ast.JoinedStr):
            out = []
            for v in node.values:
                if isinstance(v, ast.Constant):
                    out.append(v.value)
                elif isinstance(v, ast.FormattedValue):
                    out.append(self.value_of(v.value, piece))
            return "".join(out)
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Add):
            left = self._eval(node.left, piece) if _is_str_expr(node.left) else self.value_of(node.left, piece)
            right = self._eval(node.right, piece) if _is_str_expr(node.right) else self.value_of(node.right, piece)
            return left + right
        if isinstance(node, ast.BinOp) and isinstance(node.op, ast.Mod):
            fmt = self._eval(node.left, piece)
            return fmt.replace("%s", self.value_of(node.right, piece), 1) if "%s" in fmt else fmt
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
            if node.func.attr in ("strip", "lstrip", "rstrip"):
                return getattr(self._eval(node.func.value, piece), node.func.attr)()
            if node.func.attr == "dedent":
                import textwrap
                return textwrap.dedent(self._eval(node.args[0], piece))
            if node.func.attr == "format":
                fmt = self._eval(node.func.value, piece)
                kw = {k.arg: self.value_of(k.value, piece) for k in node.keywords if k.arg}
                pos = [self.value_of(a, piece) for a in node.args]

                def sub(m):
                    name = m.group(1)
                    if name == "":
                        return pos.pop(0) if pos else m.group(0)
                    if name.isdigit():
                        i = int(name)
                        return pos[i] if i < len(pos) else m.group(0)
                    return kw.get(name, m.group(0))

                return re.sub(r"(?<!\{)\{([A-Za-z_0-9]*)\}(?!\})", sub, fmt).replace("{{", "{").replace("}}", "}")
        return self.value_of(node, piece)


@dataclass
class Candidate:
    text: str
    line: int
    end_line: int
    context: str  # assign / call-arg / return / list / augassign / other
    unresolved: list[str]
    placeholders: dict[str, str]
    function: str | None
    function_src_hint: str  # a few markers found in the enclosing function (e.g. analyzer_session)


def _parents(tree):
    parents = {}
    for n in ast.walk(tree):
        for c in ast.iter_child_nodes(n):
            parents[c] = n
    return parents


def _func_locals(func) -> dict[str, ast.AST]:
    """Names assigned exactly once in the function (simple ``x = <expr>``)."""
    seen: dict[str, list[ast.AST]] = {}
    if func is None:
        return {}
    for n in ast.walk(func):
        if isinstance(n, ast.Assign):
            for t in n.targets:
                if isinstance(t, ast.Name):
                    seen.setdefault(t.id, []).append(n.value)
        elif isinstance(n, ast.AnnAssign) and isinstance(n.target, ast.Name) and n.value is not None:
            seen.setdefault(n.target.id, []).append(n.value)
        elif isinstance(n, ast.AugAssign) and isinstance(n.target, ast.Name):
            seen.setdefault(n.target.id, []).append(n.value)
            seen[n.target.id].append(n.value)  # never "exactly once"
    return {k: v[0] for k, v in seen.items() if len(v) == 1}


HINT_MARKERS = ("analyzer_session", "ANALYZER_NEO4J_DATABASE", "analyzer_database(")


def candidates(path: str, index: ModuleIndex, rules: dict[str, str]) -> list[Candidate]:
    try:
        src = open(path, encoding="utf-8").read()
        tree = ast.parse(src)
    except (SyntaxError, UnicodeDecodeError):
        return []
    parents = _parents(tree)
    lines = src.splitlines()
    out: list[Candidate] = []

    def enclosing_func(n):
        p = parents.get(n)
        while p is not None and not isinstance(p, (ast.FunctionDef, ast.AsyncFunctionDef)):
            p = parents.get(p)
        return p

    for node in ast.walk(tree):
        if not _is_str_expr(node):
            continue
        parent = parents.get(node)
        # only maximal string expressions
        if parent is not None and _is_str_expr(parent) and not (
            isinstance(parent, ast.Call) and node in parent.args and getattr(parent.func, "attr", "") == "format"
        ):
            continue
        if isinstance(parent, ast.JoinedStr) or isinstance(parent, ast.FormattedValue):
            continue
        if isinstance(parent, ast.Expr):  # docstring / bare literal
            continue
        func = enclosing_func(node)
        ev = Evaluator(index, path, rules, _func_locals(func))
        ev.line = node.lineno
        piece = ev.eval(node)
        if isinstance(parent, ast.AugAssign):
            ctx = "augassign"
        elif isinstance(parent, (ast.Assign, ast.AnnAssign)):
            ctx = "assign"
        elif isinstance(parent, ast.Call):
            attr = getattr(parent.func, "attr", getattr(parent.func, "id", ""))
            ctx = "append" if attr in ("append", "extend", "insert") else "call-arg"
        elif isinstance(parent, ast.Return):
            ctx = "return"
        elif isinstance(parent, (ast.List, ast.Tuple, ast.Set)):
            ctx = "list"
        elif isinstance(parent, ast.keyword):
            ctx = "call-arg"
        else:
            ctx = type(parent).__name__.lower() if parent is not None else "module"
        hint = ""
        if func is not None:
            fsrc = "\n".join(lines[func.lineno - 1: func.end_lineno])
            hint = ",".join(m for m in HINT_MARKERS if m in fsrc)
        out.append(Candidate(piece.text, node.lineno, node.end_lineno or node.lineno, ctx,
                             piece.unresolved, piece.placeholders,
                             func.name if func is not None else None, hint))
    return out
