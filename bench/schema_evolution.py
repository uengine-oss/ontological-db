#!/usr/bin/env python3
"""Schema evolution benchmark — what a schema change costs on a loaded graph.

The question: when the schema changes under data that already exists, how long
does the change take, and how many rows does it rewrite?

Four ways of answering it, on identical data:

  ontological          the change as a catalog edit where it can be one
                       (og_schema_apply) — names, labels and the hierarchy are
                       catalog entries that point at storage
  ontological-physical the same database migrated the classical way: build the
                       new structure, copy every row across, delete the old.
                       Set-based SQL, the best case of that approach
  neo4j                Neo4j 5 community, the refactoring Cypher its
                       documentation gives, batched with CALL … IN TRANSACTIONS
  age                  Apache AGE 1.5 on the same PostgreSQL server as
                       Ontological; Cypher where AGE can express the change,
                       its own SQL bulk path where it cannot

Every scenario is followed by a check that the graph now answers as it should
on every system. A system whose check fails has its timing voided — the same
rule `bench/harness.py` applies.

    python3 bench/schema_evolution.py --sizes 10000,100000,1000000
    python3 bench/schema_evolution.py --systems ontological,neo4j --sizes 100000

Results: bench/results/schema_evolution.json and a Markdown table on stdout.
"""

import argparse
import json
import os
import sys
import time

import psycopg2

PG = dict(host=os.environ.get("PGHOST", "localhost"), port=int(os.environ.get("PGPORT", "28816")),
          user=os.environ.get("PGUSER", "dev"))
NEO4J_URI = os.environ.get("NEO4J_URI", "bolt://localhost:27687")
NEO4J_AUTH = (os.environ.get("NEO4J_USER", "neo4j"), os.environ.get("NEO4J_PASSWORD", "benchpass123"))
HERE = os.path.dirname(os.path.abspath(__file__))
AGE_TIMEOUT_S = int(os.environ.get("AGE_TIMEOUT_S", "900"))

SCENARIOS = [
    ("S1", "rename a property", "Person.name → fullName"),
    ("S2", "rename a node type", "Person → Individual"),
    ("S3", "rename a relationship type", "KNOWS → FOLLOWS"),
    ("S4", "introduce a supertype", "every Individual is also an Agent"),
    ("S5", "drop a property", "Individual.status"),
    ("S6", "change a property's type", "Individual.age: string → int"),
    ("S7", "add a uniqueness constraint", "Individual.fullName"),
]


def edges_of(n):
    """Two deterministic out-edges per node."""
    return [(i, (i * 7919 + 1) % n) for i in range(n)] + [(i, (i * 104729 + 3) % n) for i in range(n)]


def expected_age_over_60(n):
    return sum(1 for i in range(n) if 20 + i % 50 > 60)


# --------------------------------------------------------------------------
# PostgreSQL helpers
# --------------------------------------------------------------------------


def pg_connect(db):
    c = psycopg2.connect(dbname=db, **PG)
    c.autocommit = True
    return c


def recreate(db):
    c = pg_connect("postgres")
    with c.cursor() as cur:
        cur.execute(f"DROP DATABASE IF EXISTS {db}")
        cur.execute(f"CREATE DATABASE {db}")
    c.close()


def q(conn, sql, args=None):
    with conn.cursor() as cur:
        cur.execute(sql, args)
        try:
            return cur.fetchall()
        except psycopg2.ProgrammingError:
            return None


def one(conn, sql, args=None):
    rows = q(conn, sql, args)
    return rows[0][0] if rows else None


def writes(conn):
    """Rows inserted + updated + deleted in this database so far, including the
    system catalogs and temporary tables a migration builds."""
    q(conn, "SELECT pg_stat_force_next_flush()")
    time.sleep(0.05)
    # Database-wide, so rows in a table the change dropped still count, and
    # catalog rows count too: a catalog edit is not free, it is small.
    return one(conn, "SELECT (tup_inserted + tup_updated + tup_deleted)::int8 "
                     "FROM pg_stat_database WHERE datname = current_database()")


# --------------------------------------------------------------------------
# Systems
# --------------------------------------------------------------------------


class Ontological:
    name = "ontological"
    db = "bench_evo_og"

    def setup(self, n):
        recreate(self.db)
        self.c = pg_connect(self.db)
        c = self.c
        q(c, "CREATE EXTENSION ontological CASCADE")
        q(c, "SELECT og_create_graph('evo')")
        q(c, "SELECT og_create_type('evo','Person','entity')")
        for p, t in [("i", "int"), ("name", "string"), ("age", "string"), ("status", "string")]:
            q(c, "SELECT og_add_property('evo','Person',%s,%s)", (p, t))
        q(c, "SELECT og_create_type('evo','KNOWS','relation')")
        tid = one(c, "SELECT og_type_id('evo','Person')")
        rid = one(c, "SELECT og_type_id('evo','KNOWS')")
        self.tid, self.rid = tid, rid
        # The bulk path bench/harness.py uses: straight into the registries,
        # the typed table and both adjacency directions.
        q(c, f"""
            INSERT INTO og_data.og_node (id, type_id)
            SELECT og_make_id(0,{tid},i+1), {tid} FROM generate_series(0,{n-1}) i;
            INSERT INTO og_data.n_{tid} (id, p_i, p_name, p_age, p_status)
            SELECT og_make_id(0,{tid},i+1), i, 'n'||i, (20 + i % 50)::text,
                   CASE WHEN i % 3 = 0 THEN 'inactive' ELSE 'active' END
              FROM generate_series(0,{n-1}) i;
            UPDATE og_data.og_id_alloc SET next_id = {n+1} WHERE type_id = {tid};
            CREATE TEMP TABLE e_raw AS
              SELECT i AS s, ((i::int8 * 7919 + 1) % {n})::int AS d FROM generate_series(0,{n-1}) i
              UNION ALL
              SELECT i, ((i::int8 * 104729 + 3) % {n})::int FROM generate_series(0,{n-1}) i;
            INSERT INTO og_data.og_edge (id, type_id, src, dst)
            SELECT og_make_id(0,{rid}, row_number() OVER ()), {rid},
                   og_make_id(0,{tid}, s+1), og_make_id(0,{tid}, d+1) FROM e_raw;
            UPDATE og_data.og_id_alloc SET next_id = {2*n+1} WHERE type_id = {rid};
            INSERT INTO og_data.e_{rid} (id, src, dst)
            SELECT id, src, dst FROM og_data.og_edge WHERE type_id = {rid};
            INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
            SELECT src, {rid}, 'o', chunk, count(*)::int4, array_agg(dst), array_agg(id)
              FROM (SELECT src, dst, id,
                           ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
                      FROM og_data.og_edge WHERE type_id = {rid}) x
             GROUP BY src, chunk;
            INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
            SELECT dst, {rid}, 'i', chunk, count(*)::int4, array_agg(src), array_agg(id)
              FROM (SELECT src, dst, id,
                           ((row_number() OVER (PARTITION BY dst ORDER BY id)) - 1)::int4 / 256 AS chunk
                      FROM og_data.og_edge WHERE type_id = {rid}) x
             GROUP BY dst, chunk;
        """)
        q(c, "SELECT og_create_index('evo','Person','i')")
        q(c, "VACUUM ANALYZE")

    def cypher_count(self, query):
        return one(self.c, "SELECT (og_cypher('evo', %s)->>'c')::int8", (query,))

    def apply(self, change):
        q(self.c, "SELECT og_schema_apply('evo', %s::jsonb)", (json.dumps(change),))

    def run(self, sid):
        c = self.c
        if sid == "S1":
            self.apply({"kind": "rename_property", "type": "Person", "property": "name", "new_name": "fullName"})
        elif sid == "S2":
            self.apply({"kind": "rename_type", "type": "Person", "new_name": "Individual"})
        elif sid == "S3":
            self.apply({"kind": "rename_type", "type": "KNOWS", "new_name": "FOLLOWS"})
        elif sid == "S4":
            q(c, "SELECT og_create_type('evo','Agent','entity', ARRAY[]::text[], true)")
            self.apply({"kind": "add_parent", "type": "Individual", "parent": "Agent"})
        elif sid == "S5":
            self.apply({"kind": "drop_property", "type": "Individual", "property": "status"})
        elif sid == "S6":
            self.apply({"kind": "alter_property_type", "type": "Individual", "property": "age", "to": "int"})
        elif sid == "S7":
            self.apply({"kind": "add_unique", "type": "Individual", "property": "fullName"})

    # --- checks shared by both Ontological variants ------------------------

    def check(self, sid, n):
        cc = self.cypher_count
        if sid == "S1":
            return (cc("MATCH (p:Person) WHERE p.fullName STARTS WITH 'n' RETURN count(p) AS c") == n
                    and cc("MATCH (p:Person {fullName: 'n7'}) RETURN count(p) AS c") == 1)
        if sid == "S2":
            return (cc("MATCH (p:Individual) RETURN count(p) AS c") == n
                    and cc("MATCH (a:Individual {i: 7})-[:KNOWS]->(b:Individual) RETURN count(b) AS c") == 2)
        if sid == "S3":
            return (cc("MATCH (a:Individual {i: 7})-[:FOLLOWS]->(b:Individual) RETURN count(b) AS c") == 2
                    and cc("MATCH (a:Individual {i: 7})-[:KNOWS]->(b) RETURN count(b) AS c") == 0)
        if sid == "S4":
            return cc("MATCH (a:Agent) RETURN count(a) AS c") == n
        if sid == "S5":
            return cc("MATCH (a:Individual) WHERE a.status IS NOT NULL RETURN count(a) AS c") == 0
        if sid == "S6":
            return cc("MATCH (a:Individual) WHERE a.age > 60 RETURN count(a) AS c") == expected_age_over_60(n)
        if sid == "S7":
            try:
                q(self.c, "SELECT og_cypher('evo', 'CREATE (:Individual {fullName: \"n7\", i: -1})')")
                return False
            except psycopg2.Error:
                return True
        return False

    def cost(self):
        return writes(self.c)


class OntologicalPhysical(Ontological):
    """The classical migration, done on Ontological's own storage.

    A renamed type gets new identifiers — an identifier encodes its type — so
    every row is copied, every edge endpoint and adjacency entry pointing at it
    is rewritten, and the old rows are deleted. This is what the catalog path
    avoids; measuring it on the same storage isolates that difference.
    """
    name = "ontological-physical"
    db = "bench_evo_phys"

    def run(self, sid):
        c = self.c
        if sid == "S1":
            q(c, f"""
                SELECT og_add_property('evo','Person','fullName','string');
                UPDATE og_data.n_{self.tid} SET p_fullname = p_name;
                SELECT og_schema_apply('evo', '{{"kind":"drop_property","type":"Person","property":"name"}}');
            """)
        elif sid == "S2":
            q(c, "SELECT og_create_type('evo','Individual','entity')")
            for p, t in [("i", "int"), ("fullName", "string"), ("age", "string"), ("status", "string")]:
                q(c, "SELECT og_add_property('evo','Individual',%s,%s)", (p, t))
            new = one(c, "SELECT og_type_id('evo','Individual')")
            old = self.tid
            q(c, f"""
                CREATE TEMP TABLE remap AS
                  SELECT id AS old_id, og_make_id(0, {new}, row_number() OVER (ORDER BY id)) AS new_id
                    FROM og_data.n_{old};
                CREATE UNIQUE INDEX ON remap (old_id);
                ANALYZE remap;
                INSERT INTO og_data.og_node (id, type_id) SELECT new_id, {new} FROM remap;
                INSERT INTO og_data.n_{new} (id, p_i, p_fullname, p_age, p_status)
                  SELECT r.new_id, o.p_i, o.p_fullname, o.p_age, o.p_status
                    FROM og_data.n_{old} o JOIN remap r ON r.old_id = o.id;
                UPDATE og_data.og_id_alloc SET next_id = (SELECT count(*) + 1 FROM remap) WHERE type_id = {new};
                UPDATE og_data.og_edge e SET src = r.new_id FROM remap r WHERE e.src = r.old_id;
                UPDATE og_data.og_edge e SET dst = r.new_id FROM remap r WHERE e.dst = r.old_id;
                UPDATE og_data.e_{self.rid} e SET src = r.new_id FROM remap r WHERE e.src = r.old_id;
                UPDATE og_data.e_{self.rid} e SET dst = r.new_id FROM remap r WHERE e.dst = r.old_id;
                UPDATE og_data.og_adj a SET src = r.new_id FROM remap r WHERE a.src = r.old_id;
                UPDATE og_data.og_adj a
                   SET nbr = (SELECT array_agg(coalesce(r.new_id, x) ORDER BY o)
                                FROM unnest(a.nbr) WITH ORDINALITY AS u(x, o)
                                LEFT JOIN remap r ON r.old_id = u.x);
                DELETE FROM og_data.n_{old};
                DELETE FROM og_data.og_node WHERE type_id = {old};
                SELECT og_drop_type('evo', 'Person');
            """)
            self.tid = new
            q(c, "SELECT og_create_index('evo','Individual','i')")
        elif sid == "S3":
            q(c, "SELECT og_create_type('evo','FOLLOWS','relation')")
            new = one(c, "SELECT og_type_id('evo','FOLLOWS')")
            old = self.rid
            q(c, f"""
                CREATE TEMP TABLE eremap AS
                  SELECT id AS old_id, og_make_id(0, {new}, row_number() OVER (ORDER BY id)) AS new_id
                    FROM og_data.e_{old};
                CREATE UNIQUE INDEX ON eremap (old_id);
                ANALYZE eremap;
                INSERT INTO og_data.og_edge (id, type_id, src, dst)
                  SELECT r.new_id, {new}, e.src, e.dst FROM og_data.e_{old} e JOIN eremap r ON r.old_id = e.id;
                INSERT INTO og_data.e_{new} (id, src, dst)
                  SELECT r.new_id, e.src, e.dst FROM og_data.e_{old} e JOIN eremap r ON r.old_id = e.id;
                UPDATE og_data.og_id_alloc SET next_id = (SELECT count(*) + 1 FROM eremap) WHERE type_id = {new};
                UPDATE og_data.og_adj a
                   SET etype = {new},
                       eid = (SELECT array_agg(r.new_id ORDER BY o)
                                FROM unnest(a.eid) WITH ORDINALITY AS u(x, o)
                                JOIN eremap r ON r.old_id = u.x)
                 WHERE a.etype = {old};
                DELETE FROM og_data.e_{old};
                DELETE FROM og_data.og_edge WHERE type_id = {old};
                SELECT og_drop_type('evo', 'KNOWS');
            """)
            self.rid = new
        else:
            # S4–S7 have no cheaper-or-dearer classical form here: introducing a
            # supertype is catalog-only by nature on this storage, dropping a
            # column is metadata in PostgreSQL, and a type change or a unique
            # index is physical on every system. Run the same path so the table
            # stays comparable.
            super().run(sid)


class Neo4j:
    name = "neo4j"

    def setup(self, n):
        from neo4j import GraphDatabase
        self.d = GraphDatabase.driver(NEO4J_URI, auth=NEO4J_AUTH)
        with self.d.session() as s:
            s.run("MATCH (x) CALL { WITH x DETACH DELETE x } IN TRANSACTIONS OF 20000 ROWS").consume()
            for c in s.run("SHOW CONSTRAINTS YIELD name RETURN name").values():
                s.run(f"DROP CONSTRAINT {c[0]}").consume()
            for ix in s.run("SHOW INDEXES YIELD name, type WHERE type <> 'LOOKUP' RETURN name").values():
                s.run(f"DROP INDEX {ix[0]}").consume()
            s.run("CREATE INDEX person_i FOR (p:Person) ON (p.i)").consume()
            s.run("CALL db.awaitIndexes(600)").consume()
            for lo in range(0, n, 20000):
                s.run("UNWIND range($lo, $hi) AS i "
                      "CREATE (:Person {i: i, name: 'n' + toString(i), age: toString(20 + i % 50), "
                      "status: CASE WHEN i % 3 = 0 THEN 'inactive' ELSE 'active' END})",
                      lo=lo, hi=min(lo + 20000, n) - 1).consume()
            es = edges_of(n)
            for lo in range(0, len(es), 20000):
                s.run("UNWIND $rows AS r MATCH (a:Person {i: r[0]}), (b:Person {i: r[1]}) "
                      "CREATE (a)-[:KNOWS]->(b)", rows=[list(e) for e in es[lo:lo + 20000]]).consume()
            s.run("CALL db.awaitIndexes(600)").consume()
            got = s.run("MATCH ()-[r:KNOWS]->() RETURN count(r) AS c").single()["c"]
            if got != len(es):
                raise RuntimeError(f"neo4j loaded {got} edges, expected {len(es)}")
        self.counters = {}

    def _run(self, cypher):
        with self.d.session() as s:
            summ = s.run(cypher).consume()
            c = summ.counters
            for k in ("properties_set", "labels_added", "labels_removed",
                      "relationships_created", "relationships_deleted", "nodes_created", "nodes_deleted"):
                self.counters[k] = self.counters.get(k, 0) + getattr(c, k)

    def run(self, sid):
        batch = "IN TRANSACTIONS OF 10000 ROWS"
        if sid == "S1":
            self._run(f"MATCH (p:Person) CALL {{ WITH p SET p.fullName = p.name REMOVE p.name }} {batch}")
        elif sid == "S2":
            self._run(f"MATCH (p:Person) CALL {{ WITH p SET p:Individual REMOVE p:Person }} {batch}")
        elif sid == "S3":
            # A relationship's type cannot be changed in Neo4j; it is recreated.
            self._run(f"MATCH (a)-[k:KNOWS]->(b) CALL {{ WITH a, k, b CREATE (a)-[:FOLLOWS]->(b) DELETE k }} {batch}")
        elif sid == "S4":
            self._run(f"MATCH (p:Individual) CALL {{ WITH p SET p:Agent }} {batch}")
        elif sid == "S5":
            self._run(f"MATCH (p:Individual) CALL {{ WITH p REMOVE p.status }} {batch}")
        elif sid == "S6":
            self._run(f"MATCH (p:Individual) CALL {{ WITH p SET p.age = toInteger(p.age) }} {batch}")
        elif sid == "S7":
            self._run("CREATE CONSTRAINT individual_fullname FOR (p:Individual) REQUIRE p.fullName IS UNIQUE")
            with self.d.session() as s:
                s.run("CALL db.awaitIndexes(600)").consume()

    def count(self, cypher):
        with self.d.session() as s:
            return s.run(cypher).single()["c"]

    def check(self, sid, n):
        cc = self.count
        if sid == "S1":
            return (cc("MATCH (p:Person) WHERE p.fullName STARTS WITH 'n' RETURN count(p) AS c") == n
                    and cc("MATCH (p:Person) WHERE p.name IS NOT NULL RETURN count(p) AS c") == 0)
        if sid == "S2":
            return (cc("MATCH (p:Individual) RETURN count(p) AS c") == n
                    and cc("MATCH (p:Person) RETURN count(p) AS c") == 0
                    and cc("MATCH (a:Individual {i: 7})-[:KNOWS]->(b:Individual) RETURN count(b) AS c") == 2)
        if sid == "S3":
            return (cc("MATCH (a:Individual {i: 7})-[:FOLLOWS]->(b:Individual) RETURN count(b) AS c") == 2
                    and cc("MATCH ()-[k:KNOWS]->() RETURN count(k) AS c") == 0)
        if sid == "S4":
            return cc("MATCH (a:Agent) RETURN count(a) AS c") == n
        if sid == "S5":
            return cc("MATCH (a:Individual) WHERE a.status IS NOT NULL RETURN count(a) AS c") == 0
        if sid == "S6":
            return cc("MATCH (a:Individual) WHERE a.age > 60 RETURN count(a) AS c") == expected_age_over_60(n)
        if sid == "S7":
            try:
                self._run("CREATE (:Individual {fullName: 'n7', i: -1})")
                return False
            except Exception:
                return True
        return False

    def cost(self):
        c = self.counters
        return (c.get("properties_set", 0) + c.get("labels_added", 0) + c.get("labels_removed", 0)
                + c.get("relationships_created", 0) + c.get("relationships_deleted", 0))


class Age:
    """Apache AGE on the same server. AGE has no statement that renames a label,
    so S2 and S3 use its own SQL bulk path (the one bench/harness.py loads with):
    copy into a new label table under new graphids, repoint the edges, drop the
    old label. S4 is not expressible — a vertex has exactly one label."""
    name = "age"
    db = "bench_evo_age"
    pre = "LOAD 'age'; SET search_path = ag_catalog, \"$user\", public;"

    def setup(self, n):
        recreate(self.db)
        self.c = pg_connect(self.db)
        c = self.c
        q(c, "CREATE EXTENSION age")
        q(c, self.pre + " SELECT create_graph('evo'); SELECT create_vlabel('evo','Person');"
                        " SELECT create_elabel('evo','KNOWS');")
        q(c, self.pre + f"""
            INSERT INTO evo."Person" (properties)
            SELECT agtype_build_map('i', i, 'name', 'n'||i, 'age', (20 + i % 50)::text,
                                    'status', CASE WHEN i % 3 = 0 THEN 'inactive' ELSE 'active' END)
              FROM generate_series(0,{n-1}) i;
            CREATE TEMP TABLE idx AS
              SELECT id, agtype_access_operator(VARIADIC ARRAY[properties, '"i"'::agtype])::text::int AS ord
                FROM evo."Person";
            CREATE UNIQUE INDEX ON idx(ord);
            INSERT INTO evo."KNOWS" (start_id, end_id, properties)
            SELECT a.id, b.id, agtype_build_map()
              FROM (SELECT i AS s, ((i::int8 * 7919 + 1) % {n})::int AS d FROM generate_series(0,{n-1}) i
                    UNION ALL
                    SELECT i, ((i::int8 * 104729 + 3) % {n})::int FROM generate_series(0,{n-1}) i) r
              JOIN idx a ON a.ord = r.s JOIN idx b ON b.ord = r.d;
            CREATE INDEX knows_start ON evo."KNOWS" (start_id);
            CREATE INDEX knows_end ON evo."KNOWS" (end_id);
            CREATE INDEX person_i ON evo."Person"
                (agtype_access_operator(VARIADIC ARRAY[properties, '"i"'::agtype]));
        """)
        q(c, "VACUUM ANALYZE")
        # AGE's Cypher SET grows faster than linearly (2.6 s at 10k rows, 370 s
        # at 100k on this machine). A scenario that exceeds this is recorded
        # as a timeout rather than left running for hours.
        q(c, f"SET statement_timeout = '{AGE_TIMEOUT_S}s'")

    def cy(self, cypher, cols="(c agtype)"):
        return q(self.c, self.pre + f" SELECT * FROM cypher('evo', $$ {cypher} $$) AS {cols}")

    def run(self, sid):
        c = self.c
        if sid == "S1":
            self.cy("MATCH (p:Person) SET p.fullName = p.name REMOVE p.name")
        elif sid == "S2":
            q(c, self.pre + """
                SELECT create_vlabel('evo','Individual');
                CREATE TEMP TABLE remap AS
                  SELECT id AS old_id,
                         _graphid(_label_id('evo', 'Individual'),
                                  nextval('evo."Individual_id_seq"')) AS new_id
                    FROM evo."Person";
                CREATE UNIQUE INDEX ON remap (old_id);
                ANALYZE remap;
                INSERT INTO evo."Individual" (id, properties)
                  SELECT r.new_id, p.properties FROM evo."Person" p JOIN remap r ON r.old_id = p.id;
                UPDATE evo."KNOWS" k SET start_id = r.new_id FROM remap r WHERE k.start_id = r.old_id;
                UPDATE evo."KNOWS" k SET end_id = r.new_id FROM remap r WHERE k.end_id = r.old_id;
                SELECT drop_label('evo', 'Person');
                CREATE INDEX individual_i ON evo."Individual"
                    (agtype_access_operator(VARIADIC ARRAY[properties, '"i"'::agtype]));
            """)
        elif sid == "S3":
            q(c, self.pre + """
                SELECT create_elabel('evo','FOLLOWS');
                INSERT INTO evo."FOLLOWS" (start_id, end_id, properties)
                  SELECT start_id, end_id, properties FROM evo."KNOWS";
                SELECT drop_label('evo', 'KNOWS');
                CREATE INDEX follows_start ON evo."FOLLOWS" (start_id);
                CREATE INDEX follows_end ON evo."FOLLOWS" (end_id);
            """)
        elif sid == "S4":
            raise NotImplementedError("a vertex has exactly one label in AGE")
        elif sid == "S5":
            self.cy("MATCH (p:Individual) REMOVE p.status")
        elif sid == "S6":
            self.cy("MATCH (p:Individual) SET p.age = toInteger(p.age)")
        elif sid == "S7":
            q(c, self.pre + """
                CREATE UNIQUE INDEX individual_fullname ON evo."Individual"
                    (agtype_access_operator(VARIADIC ARRAY[properties, '"fullName"'::agtype]));
            """)

    def count(self, cypher):
        return int(str(self.cy(cypher)[0][0]))

    def check(self, sid, n):
        # Checks are not timed. Parallel workers are off for them because the
        # dev container's /dev/shm is PostgreSQL's parallel-query shared memory
        # and a 1M-row parallel hash does not fit in Docker's default 64 MB.
        q(self.c, "SET max_parallel_workers_per_gather = 0")
        try:
            return self._check(sid, n)
        finally:
            q(self.c, "RESET max_parallel_workers_per_gather")

    def _check(self, sid, n):
        cc = self.count
        if sid == "S1":
            return (cc("MATCH (p:Person) WHERE p.fullName STARTS WITH 'n' RETURN count(p)") == n
                    and cc("MATCH (p:Person) WHERE p.name IS NOT NULL RETURN count(p)") == 0)
        if sid == "S2":
            return (cc("MATCH (p:Individual) RETURN count(p)") == n
                    and cc("MATCH (a:Individual {i: 7})-[:KNOWS]->(b:Individual) RETURN count(b)") == 2)
        if sid == "S3":
            return cc("MATCH (a:Individual {i: 7})-[:FOLLOWS]->(b:Individual) RETURN count(b)") == 2
        if sid == "S5":
            return cc("MATCH (a:Individual) WHERE a.status IS NOT NULL RETURN count(a)") == 0
        if sid == "S6":
            return cc("MATCH (a:Individual) WHERE a.age > 60 RETURN count(a)") == expected_age_over_60(n)
        if sid == "S7":
            try:
                self.cy("CREATE (:Individual {fullName: 'n7', i: -1})")
                return False
            except psycopg2.Error:
                return True
        return False

    def cost(self):
        return writes(self.c)


SYSTEMS = {s.name: s for s in (Ontological, OntologicalPhysical, Neo4j, Age)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--sizes", default="10000,100000")
    ap.add_argument("--systems", default=",".join(SYSTEMS))
    ap.add_argument("--repeat", type=int, default=1,
                    help="fresh load + full scenario run this many times; the median is reported")
    ap.add_argument("--skip", default="",
                    help="system:S1,S2;system:S5 — scenarios recorded as skipped, not run")
    ap.add_argument("--out", default=os.path.join(HERE, "results", "schema_evolution.json"))
    args = ap.parse_args()
    sizes = [int(x) for x in args.sizes.split(",")]
    names = args.systems.split(",")
    skip = {}
    for part in filter(None, args.skip.split(";")):
        sysname, sids = part.split(":")
        skip[sysname] = set(sids.split(","))

    results = json.load(open(args.out)) if os.path.exists(args.out) else {}
    for n in sizes:
        for name in names:
            runs = {}
            for rep in range(args.repeat):
                sysobj = SYSTEMS[name]()
                print(f"== {name} @ {n:,} nodes / {2*n:,} relationships, run {rep+1}/{args.repeat}", flush=True)
                t = time.perf_counter()
                sysobj.setup(n)
                print(f"   loaded in {time.perf_counter()-t:.1f}s", flush=True)
                before = sysobj.cost()
                for sid, _, _ in SCENARIOS:
                    if sid in skip.get(name, ()):
                        runs.setdefault(sid, []).append({"status": "skipped", "note": "not run at this size"})
                        print(f"   {sid}  skipped", flush=True)
                        continue
                    try:
                        t = time.perf_counter()
                        sysobj.run(sid)
                        ms = (time.perf_counter() - t) * 1000
                    except NotImplementedError as e:
                        runs.setdefault(sid, []).append({"status": "n/a", "note": str(e)})
                        print(f"   {sid}  n/a ({e})", flush=True)
                        continue
                    except psycopg2.errors.QueryCanceled:
                        runs.setdefault(sid, []).append(
                            {"status": "timeout", "note": f"> {AGE_TIMEOUT_S} s, cancelled"})
                        print(f"   {sid}  timeout (> {AGE_TIMEOUT_S} s)", flush=True)
                        before = sysobj.cost()
                        continue
                    after = sysobj.cost()
                    ok = sysobj.check(sid, n)
                    runs.setdefault(sid, []).append({"ok": ok, "ms": round(ms, 1), "writes": after - before})
                    print(f"   {sid}  {ms:10.1f} ms  writes={after-before:>10,}  "
                          f"{'ok' if ok else 'WRONG ANSWER'}", flush=True)
                    before = sysobj.cost()
            for sid, rs in runs.items():
                key = f"{n}/{name}/{sid}"
                if rs[0].get("status") in ("n/a", "timeout", "skipped"):
                    results[key] = rs[0]
                    continue
                ms = sorted(r["ms"] for r in rs)
                results[key] = {
                    # A wrong answer in any run voids the timing (bench/harness.py's rule).
                    "status": "ok" if all(r["ok"] for r in rs) else "WRONG",
                    "ms": ms[len(ms) // 2],
                    "runs_ms": [r["ms"] for r in rs],
                    "writes": rs[-1]["writes"],
                    # Host load when the system finished: the numbers are only
                    # comparable across rows measured on a quiet machine.
                    "loadavg_1m": round(os.getloadavg()[0], 1),
                }
            json.dump(results, open(args.out, "w"), indent=1, sort_keys=True)
    print(f"results: {args.out}")


if __name__ == "__main__":
    sys.exit(main())
