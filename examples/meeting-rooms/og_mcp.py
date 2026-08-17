"""Talk to the *unmodified* Neo4j MCP server, over stdio, as a client would.

Nothing here knows about Ontological. It launches `mcp-neo4j-cypher` — the
server Neo4j publishes — points it at a Bolt URI, and calls the tools it
advertises. If a call succeeds, it succeeded against Neo4j's own client code
speaking Neo4j's own protocol; that is the whole point of driving the example
this way rather than through the Python driver directly.
"""

from __future__ import annotations

import json
import math
import os
import urllib.request
from contextlib import asynccontextmanager

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

BOLT_URI = os.environ.get("OG_BOLT_URI", "bolt://localhost:7687")
BOLT_USER = os.environ.get("OG_BOLT_USER", "dev")
BOLT_PASSWORD = os.environ.get("OG_BOLT_PASSWORD", "dev")
GRAPH = os.environ.get("OG_GRAPH", "meeting")

EMBED_URL = os.environ.get("OG_EMBED_URL", "http://localhost:11434/api/embed")
EMBED_MODEL = os.environ.get("OG_EMBED_MODEL", "qwen3-embedding:latest")
EMBED_DIMS = int(os.environ.get("OG_EMBED_DIMS", "1024"))


@asynccontextmanager
async def neo4j_mcp():
    """The published Neo4j MCP server, running against this database."""
    params = StdioServerParameters(
        command="mcp-neo4j-cypher",
        args=[
            "--db-url", BOLT_URI,
            "--username", BOLT_USER,
            "--password", BOLT_PASSWORD,
            "--database", GRAPH,
            # Not optional, despite the help text saying "default: 1000": the
            # argparse default is None, and None reaches the query as
            # `apoc.meta.schema({sample: None})`, where `None` parses as a
            # variable name. This is an upstream bug in mcp-neo4j-cypher and it
            # fails the same way against Neo4j; passing the flag is the
            # workaround on either database.
            "--schema-sample-size", "1000",
        ],
        env={**os.environ},
    )
    async with stdio_client(params) as (read, write):
        async with ClientSession(read, write) as session:
            await session.initialize()
            yield Tools(session)


class Tools:
    """Thin wrapper: call a tool, get parsed JSON, raise on a tool error."""

    def __init__(self, session: ClientSession):
        self.session = session

    async def names(self) -> list[str]:
        return [t.name for t in (await self.session.list_tools()).tools]

    async def call(self, name: str, **arguments):
        result = await self.session.call_tool(name, arguments)
        text = "".join(c.text for c in result.content if getattr(c, "text", None))
        if result.isError:
            raise RuntimeError(f"{name} failed: {text}")
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            return text

    async def schema(self, **kw):
        return await self.call("get_neo4j_schema", **kw)

    async def read(self, query: str, params: dict | None = None):
        return await self.call("read_neo4j_cypher", query=query, params=params or {})

    async def write(self, query: str, params: dict | None = None):
        return await self.call("write_neo4j_cypher", query=query, params=params or {})


# --------------------------------------------------------------------------
# Embedding
# --------------------------------------------------------------------------
# There are two places this can happen, and the example exercises both.
#
# In the database, with `genai.vector.encode()`: the whole question is then one
# Cypher statement, which is what an agent holding nothing but the Neo4j MCP
# server can actually send. This is the path `scenario.py` takes.
#
# On the client, with `embed()` below: the vector goes in as an ordinary Cypher
# parameter. This is how the rooms are indexed in `load.py`, because writing an
# embedding is an ingestion job rather than a question — and it is the fallback
# when the database has not been configured to reach an embedding endpoint.


def embed(text: str) -> list[float]:
    """One embedding, truncated to `EMBED_DIMS` and L2-normalised.

    qwen3-embedding emits 4096 dimensions and is Matryoshka-trained, so a
    prefix is a valid smaller embedding; pgvector's HNSW index stops at 2000,
    which is why the prefix is taken at all. Normalising afterwards is what
    keeps cosine distance meaningful after the truncation.
    """
    req = urllib.request.Request(
        EMBED_URL,
        data=json.dumps({"model": EMBED_MODEL, "input": text}).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=180) as fh:
        vec = json.loads(fh.read())["embeddings"][0]
    vec = vec[:EMBED_DIMS]
    norm = math.sqrt(sum(x * x for x in vec)) or 1.0
    return [x / norm for x in vec]


def room_document(name: str, location: str = "") -> str:
    """What a room's embedding is *of*.

    Embedding the bare name and then querying with "라일락 회의실" compares a
    name against a noun phrase, and the noun drags the match toward whichever
    room name happens to sit near "meeting room" in the model's space — which is
    not a ranking anyone asked for. Indexing the same phrasing the question uses
    is what makes the comparison meaningful.
    """
    doc = f"{name} meeting room / {name} 회의실"
    return f"{doc} ({location})" if location else doc
