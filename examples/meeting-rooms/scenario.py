#!/usr/bin/env python3
"""«"라일락" 회의실을 어제 예약했던 사람 목록» — end to end, over Neo4j's MCP server.

The four steps an agent actually takes, each one a tool call to the unmodified
`mcp-neo4j-cypher` server:

  1. get_neo4j_schema      what labels, properties and directions exist
  2. read_neo4j_cypher     resolve "라일락" to the stored name, by vector search
  3. read_neo4j_cypher     the question, as Cypher, with the resolved name bound
  4. read_neo4j_cypher     the same question with a name that does not exist,
                           to show the resolution step is load-bearing

Step 2 is the one worth watching. "라일락" appears nowhere in the database — the
rooms are stored under English names — so a literal match returns nothing. The
similarity score comes back with the answer, so the caller can tell a confident
resolution from a guess instead of being handed a bare string.

And it is a single Cypher statement, which is what makes it reachable at all: an
agent holding only `read_neo4j_cypher` has no way to produce an embedding, so if
the text-to-vector step were the client's job this loop would need a second tool
that the Neo4j MCP server does not have.
"""

from __future__ import annotations

import asyncio
import json
from datetime import datetime, timedelta

from og_mcp import embed, neo4j_mcp

QUESTION = '"라일락" 회의실을 어제 예약했던 사람 목록'

# Vector search under its Neo4j name, with the text embedded inside the
# database — both `db.index.vector.queryNodes` and `genai.vector.encode` are
# Neo4j's own spellings. Here the first plans down to `og_vector_search`, so the
# HNSW index does the work and the score is a real cosine similarity.
RESOLVE_IN_DB = """
CALL db.index.vector.queryNodes('room_name', 3, genai.vector.encode($text))
YIELD node, score
RETURN node.name AS canonical, node.code AS code, node.location AS location, score
"""

# The same query for a database that has not been given an embedding endpoint:
# identical but for who produced the vector.
RESOLVE_WITH_PARAM = """
CALL db.index.vector.queryNodes('room_name', 3, $probe) YIELD node, score
RETURN node.name AS canonical, node.code AS code, node.location AS location, score
"""

RESERVERS = """
MATCH (e:Employee)<-[:RESERVED_BY]-(r:Reservation)-[:FOR_ROOM]->(m:MeetingRoom)
WHERE m.name = $room AND r.begin_time >= $from AND r.begin_time < $to
RETURN DISTINCT e.name AS name, e.title AS title, e.team AS team,
       r.begin_time AS begins, r.purpose AS purpose
ORDER BY begins
"""


def rule(title: str) -> None:
    print(f"\n\033[36m{'─' * 72}\n{title}\n{'─' * 72}\033[0m")


def first_line(err: Exception) -> str:
    """The sentence of an error worth printing beside a fallback."""
    for line in str(err).splitlines():
        if line.strip():
            return line.strip()[:100]
    return err.__class__.__name__


async def main() -> None:
    midnight = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)
    yesterday, today = midnight - timedelta(days=1), midnight

    async with neo4j_mcp() as mcp:
        print(f"question: {QUESTION}")
        print(f"tools:    {', '.join(await mcp.names())}")

        # 1 ------------------------------------------------------- the schema
        rule("1. get_neo4j_schema")
        schema = await mcp.schema()
        for label, entry in schema.items():
            props = ", ".join(entry.get("properties", {}))
            print(f"  {label:<14} {entry['type']:<13} count={entry.get('count', 0):<4} {props}")
            for rel, info in entry.get("relationships", {}).items():
                arrow = "-->" if info["direction"] == "out" else "<--"
                print(f"      {arrow} {rel} {info.get('labels', [])}")

        # 2 --------------------------------------------- canonical name lookup
        rule("2. read_neo4j_cypher — resolve 라일락 by vector similarity")
        literal = await mcp.read(
            "MATCH (m:MeetingRoom) WHERE m.name = $n RETURN m.name AS name",
            {"n": "라일락"},
        )
        print(f"  literal match on '라일락': {literal}   ← nothing is stored under that name")

        # Try the one-statement form first and fall back on its actual failure,
        # rather than asking a separate question about whether it would work: a
        # capability probe that is not the query can disagree with the query.
        try:
            candidates = await mcp.read(RESOLVE_IN_DB, {"text": "라일락 회의실"})
            print("  embedding: in the database, genai.vector.encode()")
        except Exception as why:
            print(f"  embedding: on the client — the database declined ({first_line(why)})")
            candidates = await mcp.read(RESOLVE_WITH_PARAM, {"probe": embed("라일락 회의실")})
        for i, c in enumerate(candidates):
            mark = "→" if i == 0 else " "
            print(f"  {mark} {c['canonical']:<12} {c['code']:<7} {c['location']:<10} "
                  f"similarity={c['score']:.4f}")
        canonical = candidates[0]["canonical"]
        margin = candidates[0]["score"] - candidates[1]["score"]
        print(f"\n  canonical = {canonical!r}  (margin over runner-up: {margin:+.4f})")

        # 3 ------------------------------------------------ answer the question
        rule("3. read_neo4j_cypher — the question, with the resolved name bound")
        rows = await mcp.read(
            RESERVERS,
            {"room": canonical, "from": yesterday.isoformat(sep=" "), "to": today.isoformat(sep=" ")},
        )
        print(f"  {'name':<8} {'title':<18} {'team':<12} {'begins':<20} purpose")
        for r in rows:
            print(f"  {r['name']:<8} {r['title']:<18} {r['team']:<12} "
                  f"{str(r['begins'])[:19]:<20} {r['purpose']}")
        print(f"\n  {len(rows)} 명")

        # 4 ------------------------------------------ the step is load-bearing
        rule("4. the same query without step 2")
        naive = await mcp.read(
            RESERVERS,
            {"room": "라일락", "from": yesterday.isoformat(sep=" "), "to": today.isoformat(sep=" ")},
        )
        print(f"  room='라일락' (unresolved) → {len(naive)} rows: {naive}")

    print()
    print(json.dumps({"question": QUESTION, "canonical": canonical,
                      "answer": [r["name"] for r in rows]}, ensure_ascii=False))


if __name__ == "__main__":
    asyncio.run(main())
