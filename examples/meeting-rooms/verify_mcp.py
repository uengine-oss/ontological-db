#!/usr/bin/env python3
"""Does the published Neo4j MCP server work against this database, unmodified?

Each check below is something `mcp-neo4j-cypher` does on the way to answering a
question, stated as a pass/fail so the answer is a matrix rather than an
impression. A check that fails names what is missing.

The server is the real one, launched over stdio, driven through the MCP
protocol — not the Neo4j driver with the MCP part imagined.
"""

from __future__ import annotations

import asyncio
import sys

from og_mcp import BOLT_URI, GRAPH, embed, neo4j_mcp

PASS, FAIL = "\033[32m✓\033[0m", "\033[31m✗\033[0m"


class Checks:
    def __init__(self):
        self.rows: list[tuple[bool, str, str]] = []

    async def check(self, name: str, coro, note=lambda v: ""):
        try:
            value = await coro()
            self.rows.append((True, name, note(value)))
        except Exception as e:
            detail = str(e).replace("\n", " ")
            self.rows.append((False, name, detail[:150]))

    def report(self) -> int:
        width = max(len(n) for _, n, _ in self.rows)
        print()
        for ok, name, note in self.rows:
            print(f"  {PASS if ok else FAIL} {name:<{width}}  {note}")
        failed = sum(1 for ok, _, _ in self.rows if not ok)
        print(f"\n  {len(self.rows) - failed}/{len(self.rows)} checks passed")
        return 1 if failed else 0


async def main() -> int:
    print(f"neo4j MCP server → {BOLT_URI}  graph={GRAPH}")
    c = Checks()

    async with neo4j_mcp() as mcp:
        await c.check(
            "MCP handshake + tool discovery",
            mcp.names,
            lambda v: ", ".join(v),
        )

        # get_neo4j_schema — needs CALL, and needs apoc.meta.schema specifically.
        await c.check(
            "get_neo4j_schema (apoc.meta.schema)",
            mcp.schema,
            lambda v: f"{len(v)} labels/types: {', '.join(list(v)[:6])}",
        )
        await c.check(
            "  … reports relationship direction",
            lambda: mcp.schema(),
            lambda v: next(
                (f"{lab} -{r}-> {i.get('labels')}"
                 for lab, e in v.items()
                 for r, i in e.get("relationships", {}).items()
                 if i.get("direction") == "out"),
                "no outgoing relationship reported",
            ),
        )

        # Both query tools gate on EXPLAIN reporting a query type.
        await c.check(
            "read_neo4j_cypher (EXPLAIN reports 'r')",
            lambda: mcp.read("MATCH (m:MeetingRoom) RETURN count(m) AS n"),
            lambda v: f"{v}",
        )
        await c.check(
            "read tool refuses a write",
            lambda: refuses(mcp.read("CREATE (x:MeetingRoom {name: '__probe'})")),
            lambda v: v,
        )
        await c.check(
            "write tool refuses a read",
            lambda: refuses(mcp.write("MATCH (m:MeetingRoom) RETURN m.name AS n")),
            lambda v: v,
        )

        await c.check(
            "parameters bind (never interpolated)",
            lambda: mcp.read(
                "MATCH (m:MeetingRoom) WHERE m.name = $n RETURN m.code AS code",
                {"n": "Lilac"},
            ),
            lambda v: f"{v}",
        )

        # The step the whole example turns on.
        await c.check(
            "db.index.vector.queryNodes + score",
            lambda: mcp.read(
                "CALL db.index.vector.queryNodes('room_name', 3, $p) YIELD node, score "
                "RETURN node.name AS name, score",
                {"p": embed("라일락 회의실")},
            ),
            lambda v: "  ".join(f"{r['name']}={r['score']:.4f}" for r in v),
        )

        await c.check(
            "syntax error is reported, not swallowed",
            lambda: refuses(mcp.read("MATCH (m:MeetingRoom RETURN m")),
            lambda v: v,
        )

        # The step an agent cannot take for itself: turning the question's text
        # into a vector. Without this the loop needs a tool the Neo4j MCP server
        # does not have.
        await c.check(
            "genai.vector.encode (embedding in the database)",
            lambda: mcp.read(
                "CALL db.index.vector.queryNodes('room_name', 2, genai.vector.encode($t)) "
                "YIELD node, score RETURN node.name AS name, score",
                {"t": "라일락 회의실"},
            ),
            lambda v: "  ".join(f"{r['name']}={r['score']:.4f}" for r in v),
        )

        await c.check(
            "write_neo4j_cypher returns change counters",
            lambda: counters(mcp),
            lambda v: v,
        )

    return c.report()


async def refuses(coro) -> str:
    """A check that passes when the call is *rejected*."""
    try:
        result = await coro
    except Exception as e:
        return f"rejected: {str(e).splitlines()[0][:90]}"
    raise AssertionError(f"accepted when it should not have: {result}")


async def counters(mcp) -> str:
    """A write must report what it changed, not just that it succeeded.

    Checked on a create *and* the matching delete, because a counter that is
    wired to one and not the other still looks right on a single call.
    """
    made = await mcp.write("CREATE (x:MeetingRoom {name: '__verify_probe', code: 'VP'})")
    gone = await mcp.write("MATCH (x:MeetingRoom {name: '__verify_probe'}) DETACH DELETE x")
    if made.get("nodes_created") != 1 or made.get("properties_set") != 2:
        raise AssertionError(f"create reported {made}, expected 1 node and 2 properties")
    if gone.get("nodes_deleted") != 1:
        raise AssertionError(f"delete reported {gone}, expected 1 node deleted")
    return f"+{made['nodes_created']} node/{made['properties_set']} props, " \
           f"-{gone['nodes_deleted']} node"


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
