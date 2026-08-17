#!/usr/bin/env python3
"""Load the meeting-room example — every write through the Neo4j MCP server.

Run `schema.sql` first; it declares the ontology. From here on nothing is
Ontological-specific: the vector index is created with Neo4j's `CREATE VECTOR
INDEX` DDL, and every node and relationship with ordinary `CREATE`, all sent as
`write_neo4j_cypher` calls to the published `mcp-neo4j-cypher` server.

    psql -d ogstudio -f schema.sql
    python3 load.py
"""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta

from og_mcp import EMBED_DIMS, embed, neo4j_mcp, room_document

ROOMS = [
    ("Lilac",     "R-201", "2F West",  8,  "Small huddle room with a whiteboard"),
    ("Rose",      "R-202", "2F West",  12, "Standard meeting room"),
    ("Magnolia",  "R-301", "3F East",  20, "Large room with video conferencing"),
    ("Tulip",     "R-302", "3F East",  6,  "Focus room for pair work"),
    ("Sunflower", "R-401", "4F North", 30, "Town-hall space"),
]

EMPLOYEES = [
    ("김지영", "E-1001", "Product Manager",   "Platform"),
    ("박현우", "E-1002", "Backend Engineer",  "Platform"),
    ("이수민", "E-1003", "Designer",          "Experience"),
    ("최민준", "E-1004", "Data Scientist",    "Insights"),
    ("정하늘", "E-1005", "Engineering Lead",  "Platform"),
]

# (reservation code, room, reserver, day offset from today, start hour, hours, purpose)
RESERVATIONS = [
    ("RSV-001", "Lilac",     "김지영", -1, 10, 1, "Sprint planning"),
    ("RSV-002", "Lilac",     "박현우", -1, 14, 2, "Schema review"),
    ("RSV-003", "Lilac",     "이수민", -1, 17, 1, "Design critique"),
    ("RSV-004", "Lilac",     "최민준",  0,  9, 1, "Today's standup"),
    ("RSV-005", "Lilac",     "정하늘", -2, 11, 1, "Two days ago, not yesterday"),
    ("RSV-006", "Rose",      "김지영", -1, 10, 1, "Yesterday, but the wrong room"),
    ("RSV-007", "Magnolia",  "정하늘", -1, 13, 3, "All-hands rehearsal"),
    ("RSV-008", "Sunflower", "최민준",  1, 15, 2, "Tomorrow's town hall"),
]

CREATE_INDEX = f"""
CREATE VECTOR INDEX room_name IF NOT EXISTS
FOR (m:MeetingRoom) ON (m.name_vec)
OPTIONS {{indexConfig: {{
  `vector.dimensions`: {EMBED_DIMS},
  `vector.similarity_function`: 'cosine'
}}}}
"""


async def main() -> None:
    midnight = datetime.now().replace(hour=0, minute=0, second=0, microsecond=0)

    async with neo4j_mcp() as mcp:
        print("tools advertised:", ", ".join(await mcp.names()))

        print("\n· CREATE VECTOR INDEX (Neo4j DDL, through write_neo4j_cypher)")
        print("  ", await mcp.write(CREATE_INDEX))

        print("\n· rooms")
        for name, code, location, seats, descr in ROOMS:
            await mcp.write(
                "CREATE (m:MeetingRoom {name: $name, code: $code, location: $location,"
                " seats: $seats, descr: $descr, name_vec: $vec})",
                {
                    "name": name, "code": code, "location": location,
                    "seats": seats, "descr": descr,
                    "vec": embed(room_document(name, location)),
                },
            )
            print(f"   {name} ({code}) embedded")

        print("\n· employees")
        for name, code, title, team in EMPLOYEES:
            await mcp.write(
                "CREATE (e:Employee {name: $name, code: $code, title: $title, team: $team})",
                {"name": name, "code": code, "title": title, "team": team},
            )
        print(f"   {len(EMPLOYEES)} created")

        print("\n· reservations")
        for code, room, who, day, hour, hours, purpose in RESERVATIONS:
            begin = midnight + timedelta(days=day, hours=hour)
            await mcp.write(
                "MATCH (m:MeetingRoom {name: $room}), (e:Employee {name: $who}) "
                "CREATE (r:Reservation {code: $code, begin_time: $begin,"
                "                       end_time: $end, purpose: $purpose}) "
                "CREATE (r)-[:FOR_ROOM]->(m) "
                "CREATE (r)-[:RESERVED_BY]->(e)",
                {
                    "room": room, "who": who, "code": code, "purpose": purpose,
                    "begin": begin.isoformat(sep=" "),
                    "end": (begin + timedelta(hours=hours)).isoformat(sep=" "),
                },
            )
            print(f"   {code}  {room:<10} {who}  {begin:%Y-%m-%d %H:%M}")

    print("\nloaded.")


if __name__ == "__main__":
    asyncio.run(main())
