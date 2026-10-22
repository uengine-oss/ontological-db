"""Neo4j compatibility gaps found by running a real application against this
database, and the fixes for them.

Each block is a thing an application actually sent and this engine refused or —
worse — answered wrongly. They came out of pointing Ontology Studio's MCP
runtime at ontological-db instead of Neo4j; every one was found by the
application failing, not by reading the code.

Run against a scratch graph:

    psql -c "SELECT og_create_graph('fix_probe')"
    uv run --with neo4j python engine/tests/neo4j_compat_fixes.py

Covers:
  1. SHOW CONSTRAINTS / SHOW INDEXES — administration commands the parser
     refused outright, so a schema browser saw a database with no constraints.
  2. properties(n) and n[k] — a missing function, and bracket access compiled
     as list indexing even when the key was a string.
  3. Type adoption — a constraint declared a label as a *root* type, after
     which (:_Entity:Label) was rejected forever.
  5. String predicates against jsonb with a parameter — matched nothing,
     silently, because jsonb::text keeps its quotes.
"""

from __future__ import annotations

import sys

from neo4j import GraphDatabase

URI = "bolt://localhost:7687"
AUTH = ("dev", "")
GRAPH = "fix_probe"

failures: list[str] = []


def check(label: str, ok: bool, detail: str = "") -> None:
    if not ok:
        failures.append(f"{label} — {detail}")
    print(f"  [{'PASS' if ok else 'FAIL'}] {label}" + (f"  ({detail})" if detail and not ok else ""))


def main() -> int:
    driver = GraphDatabase.driver(URI, auth=AUTH)
    with driver.session(database=GRAPH) as s:

        # --- 결함 3: 제약이 먼저 만든 루트 타입도 입양돼야 한다 --------------------
        print("결함 3 — 타입 입양 (제약 → 인스턴스 순서)")
        s.run("CREATE CONSTRAINT c_doc IF NOT EXISTS FOR (n:Doc) REQUIRE n.doc_id IS UNIQUE").consume()
        try:
            s.run("CREATE (n:_Entity:Doc {doc_id: 'D1', title: '고정밀 베어링 A형'})").consume()
            check("제약 선행 후 (:_Entity:Doc) 생성", True)
        except Exception as e:
            check("제약 선행 후 (:_Entity:Doc) 생성", False, str(e).splitlines()[0][:110])

        n = s.run("MATCH (n:_Entity:Doc) RETURN count(n) AS c").single()["c"]
        check("멀티라벨 매치로 조회됨", n == 1, f"count={n}")

        # 이미 인스턴스가 있는 타입은 입양하지 않는다 (안전장치)
        s.run("CREATE CONSTRAINT c_used IF NOT EXISTS FOR (n:Used) REQUIRE n.k IS UNIQUE").consume()
        s.run("CREATE (n:Used {k: 'u1'})").consume()
        try:
            s.run("CREATE (n:_Entity:Used {k: 'u2'})").consume()
            check("인스턴스 있는 루트는 입양 거부", False, "입양돼 버렸다")
        except Exception:
            check("인스턴스 있는 루트는 입양 거부", True)

        # --- 결함 2: properties() 와 n[k] -----------------------------------------
        print("\n결함 2 — properties() / 동적 속성 접근")
        try:
            r = s.run("MATCH (n:Doc) RETURN properties(n) AS p").single()
            props = r["p"]
            # Neo4j 의 properties() 는 *맵*을 준다. `_id`/`_type` 이 남아 있으면
            # 드라이버가 노드로 hydrate 해 버리므로, dict 로 오는지까지 본다.
            check("properties(n) 이 맵을 반환", isinstance(props, dict), type(props).__name__)
            check("properties(n) 값 보존", isinstance(props, dict) and props.get("doc_id") == "D1",
                  str(props)[:90])
            check("내부 키 제거", isinstance(props, dict) and "_id" not in props and "_type" not in props,
                  str(props)[:90])
        except Exception as e:
            check("properties(n) 이 맵을 반환", False, str(e).splitlines()[0][:110])

        try:
            r = s.run(
                "MATCH (n:Doc) WHERE any(k IN keys(n) WHERE toLower(toString(n[k])) "
                "CONTAINS toLower($t)) RETURN n.doc_id AS id",
                {"t": "베어링"},
            ).single()
            check("keys() + n[k] 동적 접근", r is not None and r["id"] == "D1", str(r))
        except Exception as e:
            check("keys() + n[k] 동적 접근", False, str(e).splitlines()[0][:110])

        try:
            r = s.run("RETURN ['a','b','c'][1] AS x").single()
            check("리스트 정수 인덱싱 회귀", r["x"] == "b", str(r["x"]))
        except Exception as e:
            check("리스트 정수 인덱싱 회귀", False, str(e).splitlines()[0][:110])

        # --- 결함 1: SHOW CONSTRAINTS / SHOW INDEXES --------------------------------
        print("\n결함 1 — SHOW CONSTRAINTS / SHOW INDEXES")
        try:
            rows = list(s.run("SHOW CONSTRAINTS"))
            names = {r["name"] for r in rows}
            check("SHOW CONSTRAINTS (bare)", "c_doc" in names, str(sorted(names))[:110])
            if rows:
                cols = set(rows[0].keys())
                check("Neo4j 컬럼 형태", {"name", "type", "entityType", "labelsOrTypes",
                                          "properties"} <= cols, str(sorted(cols)))
                r0 = next(r for r in rows if r["name"] == "c_doc")
                check("labelsOrTypes 가 리스트", r0["labelsOrTypes"] == ["Doc"], str(r0["labelsOrTypes"]))
                check("properties 가 리스트", r0["properties"] == ["doc_id"], str(r0["properties"]))
                check("entityType", r0["entityType"] == "NODE", str(r0["entityType"]))
        except Exception as e:
            check("SHOW CONSTRAINTS (bare)", False, str(e).splitlines()[0][:110])

        try:
            rows = list(s.run(
                "SHOW CONSTRAINTS YIELD name, labelsOrTypes, properties "
                "WHERE name STARTS WITH $p RETURN labelsOrTypes, properties",
                {"p": "c_d"},
            ))
            check("YIELD + WHERE + RETURN (스튜디오가 쓰는 형태)", len(rows) == 1, f"{len(rows)}행")
        except Exception as e:
            check("YIELD + WHERE + RETURN (스튜디오가 쓰는 형태)", False,
                  str(e).splitlines()[0][:110])

        try:
            s.run("CREATE INDEX i_doc IF NOT EXISTS FOR (n:Doc) ON (n.title)").consume()
            rows = list(s.run("SHOW INDEXES"))
            names = {r["name"] for r in rows}
            check("SHOW INDEXES", "i_doc" in names, str(sorted(names))[:110])
            if rows:
                check("state 컬럼 존재", "state" in rows[0].keys(), str(sorted(rows[0].keys())))
        except Exception as e:
            check("SHOW INDEXES", False, str(e).splitlines()[0][:110])

        # --- 결함 5: 파라미터가 CALL 뒤의 WHERE 에서 동작해야 한다 ------------------
        print("\n결함 5 — CALL/SHOW 뒤 WHERE 의 파라미터 (선재 결함)")
        try:
            rows = list(s.run(
                "CALL db.labels() YIELD label WITH label WHERE label STARTS WITH $p RETURN label",
                {"p": "Do"},
            ))
            check("CALL + WITH + 파라미터", len(rows) == 1, f"{len(rows)}행")
        except Exception as e:
            check("CALL + WITH + 파라미터", False, str(e).splitlines()[0][:110])

        try:
            rows = list(s.run(
                "MATCH (n:Doc) WHERE n.title CONTAINS $t RETURN n.doc_id AS d", {"t": "베어링"}
            ))
            check("CONTAINS + 파라미터 회귀", len(rows) == 1, f"{len(rows)}행")
        except Exception as e:
            check("CONTAINS + 파라미터 회귀", False, str(e).splitlines()[0][:110])

        # 제약과 인덱스가 서로 섞이지 않아야 한다
        try:
            cons = {r["name"] for r in s.run("SHOW CONSTRAINTS")}
            idx = {r["name"] for r in s.run("SHOW INDEXES")}
            check("제약/인덱스가 분리됨", "i_doc" not in cons and "c_doc" not in idx,
                  f"cons={sorted(cons)} idx={sorted(idx)}")
        except Exception as e:
            check("제약/인덱스가 분리됨", False, str(e)[:110])

    driver.close()

    print("\n" + "=" * 64)
    if failures:
        print(f"FAILURES ({len(failures)}):")
        for f in failures:
            print("  - " + f)
        return 1
    print("엔진 수정 4건 전부 동작")
    return 0


sys.exit(main())
