# 테스트와 CI

> **이 문서가 답하는 질문**
> - 이 저장소에 어떤 회귀 스위트가 있고 각각 무엇을 실제로 검증하는가?
> - 각 스위트를 어떤 명령으로 돌리는가? 무엇이 사전 준비물인가?
> - 통과/실패는 어떻게 판정되는가? 놓치는 것은 무엇인가?
> - CI는 어디에 설정되어 있는가?

---

## 결론부터 — 스위트 지도

| # | 스위트 | 실행 명령 | 서버 필요 | 검증 범위 |
|---|---|---|---|---|
| 1 | Rust 유닛 테스트 | `cargo test` (`engine/`, `bolt/`) | 없음 | 렉서·파서·식별자 인코딩·RDF 파싱·PackStream |
| 2 | SQL 회귀 스위트 | `tests/run.sh` | **필요** | 확장의 인-데이터베이스 동작 전체 + 백업 왕복 + 무결성 |
| 3 | pg_regress 스캐폴딩 | (미배선) | — | **현재 동작하지 않음** — 아래 §3 |
| 4 | TypeQL 적합성 | `python3 tests/typeql/run.py` | **필요** | TypeDB 상류 예제 파일이 그대로 도는가 |
| 5 | Neo4j Movie Graph | `python3 tests/neo4j-movies/run.py` | **필요** (+ Bolt) | Neo4j 샘플 앱이 그대로 도는가, 드라이버 수준까지 |
| 6 | MCP 호환 | `python3 examples/meeting-rooms/verify_mcp.py` | **필요** (+ Bolt) | 공개 `mcp-neo4j-cypher` 서버가 무수정으로 도는가 |
| 7 | 벤치마크 회귀 게이트 | `python3 bench/harness.py --compare-baseline …` | **필요** | 성능 20% 회귀 — [05_benchmarking.md](05_benchmarking.md) |

> **CI 설정 파일은 존재하지 않는다.** 저장소 루트에 `.github/` 디렉터리가 없고,
> `Makefile`·`Jenkinsfile`·`.gitlab-ci.yml` 류도 없다. 위 스위트는 전부 수동 실행이다.
> → [10_improvements_ops.md](10_improvements_ops.md) `OPS-13`

`engine/src/lib.rs:45-48`이 이 배치를 직접 설명한다:

```
// In-database behaviour is covered by the SQL regression suite in
// `engine/tests/sql/`, run with `tests/run.sh` against a live server. Pure Rust
// units (lexer, parser, identifier encoding, RDF parsing) live beside their
// modules and run under `cargo test`.
```

---

## 1. Rust 유닛 테스트 — `cargo test`

### 실행

```bash
docker exec ontological-dev bash -lc 'cd /work/engine && cargo test'
docker exec ontological-dev bash -lc 'cd /work/bolt   && cargo test'
```

### 무엇이 있는가 (실측)

`#[cfg(test)] mod tests`가 존재하는 파일 전부:

| 파일 | 대상 |
|---|---|
| `engine/src/id.rs:93` | 64bit 식별자 인코딩(shard/type/local) |
| `engine/src/cypher/lexer.rs:266` | Cypher 렉서 |
| `engine/src/cypher/parser.rs:1133` | Cypher 파서 |
| `engine/src/typeql/lexer.rs:279` | TypeQL 렉서 |
| `engine/src/typeql/parser.rs:964`, `:1058` | TypeQL 파서 (모듈 2개) |
| `engine/src/typeql/compile.rs:776` | TypeQL → SQL 컴파일 |
| `engine/src/adapters/rdf.rs:850` | RDF 파싱 |
| `bolt/src/packstream.rs:285` | PackStream 왕복, 청크 경계 (`bolt/README.md:93`) |

### 검증하지 **않는** 것

- `#[pg_test]` 어트리뷰트가 **한 곳도 없다.** 따라서 `cargo pgrx test`는 실행할 인-데이터베이스
  테스트가 없다. `engine/Cargo.toml:18`의 `pg_test = []` feature와
  `engine/Cargo.toml:36-37`의 `pgrx-tests` dev-dependency는 현재 활용되지 않는다.
- 저장소·인접·순회·카탈로그 등 SPI를 쓰는 경로는 전부 스위트 2가 담당한다.

---

## 2. SQL 회귀 스위트 — `tests/run.sh`

**이 저장소의 핵심 게이트다.**

### 실행

```bash
# 컨테이너 안에서 (권장 — psql/pg_dump 버전 일치)
docker exec ontological-dev bash -lc 'cd /work && ./tests/run.sh'

# 호스트에서
PGHOST=localhost PGPORT=28816 ./tests/run.sh
```

> **적혀 있지 않은 전제: 데이터베이스 로케일.** `initdb` 가 C 로케일로 돈
> 클러스터에서는 `04_neo4j_compat.sql` 이 실패한다. `to_tsvector('simple', …)`
> 가 한글을 토큰화하지 못해 `db.index.fulltext.queryNodes` 가 0행을 내고,
> 그 파일의 단언이 걸린다. 코드 문제가 아니라 환경 문제이고, UTF-8 로케일
> (`initdb --encoding=UTF8 --locale=C.UTF-8`)이면 통과한다.
> `tests/run.sh` 는 이 의존성을 확인하지도, 알려주지도 않는다 —
> 실패했을 때 원인을 찾는 데 시간이 걸리는 종류다.

환경변수 (`tests/run.sh:6-8`):

| 변수 | 기본값 | 의미 |
|---|---|---|
| `PGHOST` | `localhost` | |
| `PGPORT` | `28816` | |
| `OG_TEST_DB` | `og_test` | 테스트마다 DROP/CREATE되는 데이터베이스 이름 |

> **경고**: 스크립트는 `$OG_TEST_DB`와 `${OG_TEST_DB}_restored`를 **무조건 DROP한다**
> (`tests/run.sh:17,44,52`). 기본값 `og_test`를 다른 용도로 쓰고 있다면 반드시 바꿀 것.
> 운영 데이터베이스 `og`는 건드리지 않는다.

### 흐름

**단계 A — 파일별 회귀 (`tests/run.sh:14-34`)**

`engine/tests/sql/*.sql`을 하나씩, 매번 **새 데이터베이스**에서 실행한다:

```bash
psql -d postgres -c "DROP DATABASE IF EXISTS $DB"
psql -d postgres -c "CREATE DATABASE $DB"
psql -d "$DB"    -c "CREATE EXTENSION ontological CASCADE"
out="$(psql -d "$DB" -q -f "$f" 2>&1)"
```

판정 로직 (`tests/run.sh:22-26`):

```bash
expected="$(grep -c 'EXPECT_ERROR' "$f" || true)"
actual="$(printf '%s' "$out" | grep -c '^ERROR\|^psql.*ERROR' || true)"
if [ "$actual" -le "$expected" ]; then  # ok
```

즉 **의도된 오류는 파일 안에 `EXPECT_ERROR` 주석을 그 개수만큼 적어 선언한다.**
실측 개수:

| 파일 | `EXPECT_ERROR` 수 | 검증 대상 |
|---|---|---|
| `engine/tests/sql/01_catalog_storage.sql` | 1 | 타입 계층, 구간 라벨, 저장, 인접 (spec 001 + 002) |
| `engine/tests/sql/02_cypher.sql` | 1 | Cypher 파싱 → 컴파일 → 실행 (spec 003) |
| `engine/tests/sql/03_vector_agent_rdf.sql` | 0 | 벡터 / RDF / 에이전트 (spec 004 · 006 · 008) |
| `engine/tests/sql/04_neo4j_compat.sql` | 0 | Neo4j 호환 표면 — "재작성 없이 도는가" (243줄) |
| `engine/tests/sql/05_reachability.sql` | 0 | `og_reach` / `og_reach_sql` / CSR, 그리고 재작성 스위치. 모든 단언이 **`og_vlp`와의 등가 비교**다 |

`05_reachability.sql`의 헤더가 이 파일의 성격을 못 박는다:
"nothing in this file measures anything — it only checks answers."

**단계 B — 백업 왕복 (`tests/run.sh:36-67`)**

별도 게이트로 분리된 이유가 주석에 있다 (`tests/run.sh:37-39`):
확장 소유 테이블은 `pg_extension_config_dump()` 등록 없이는 `pg_dump`에 잡히지 않고,
그것을 틀리면 **조용히 빈 그래프가 복원된다.**

절차:

1. 새 DB + 확장 + `examples/demo.sql`
2. `before` = `og_node_view` 개수 `/` `og_edge_view` 개수
3. `pg_dump -f /tmp/og_roundtrip.dump`
4. `${DB}_restored`에 복원
5. `after` = 같은 카운트, `query_ok` = `og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')`
6. 통과 조건 (`tests/run.sh:60`): `before = after` **그리고** `before`가 비어 있지 않고
   **그리고** `before != "0/0"` **그리고** `query_ok`가 비어 있지 않을 것

**단계 C — 구조 무결성 (`tests/run.sh:69-75`)**

```bash
psql -d "$DB" -tAc "SELECT count(*) FROM og_check_integrity()" | while read -r n; do
    if [ "${n:-0}" = "0" ]; then echo "integrity ok"; else echo "integrity FAIL ($n violations)"; fi
done
```

> **알아둘 것**: 이 블록은 파이프로 `while` 서브셸에 들어가므로 `$fail` 카운터를 증가시키지
> 못한다. 즉 **무결성 위반이 있어도 `tests/run.sh`의 종료 코드는 0이 될 수 있다.**
> 화면의 `integrity FAIL` 문구를 직접 확인해야 한다. → `OPS-14`

### 출력과 종료 코드

```
01_catalog_storage.sql            ok
02_cypher.sql                     ok
...
backup round trip                 ok (69/104 nodes/edges preserved)

-- structural integrity --
integrity                          ok

5 passed, 0 failed
```

실패 시 실패 파일 목록을 출력하고 `exit 1` (`tests/run.sh:79`).
(위 카운트 값은 예시 형식이며 실제 숫자는 `examples/demo.sql`에 달려 있다.)

---

## 3. pg_regress 스캐폴딩 — 현재 동작하지 않음

`engine/tests/pg_regress/`에 있는 파일은 **두 개뿐이다**:

```
engine/tests/pg_regress/sql/setup.sql
engine/tests/pg_regress/expected/setup.out
```

`engine/tests/pg_regress/sql/setup.sql` 전문:

```sql
-- this setup file is run immediately after the regression database is (re)created
-- the file is optional but you likely want to create the extension
CREATE EXTENSION engine;
```

**`engine`이라는 확장은 존재하지 않는다.** 이 확장의 이름은 `ontological`이다
(`engine/ontological.control`, `engine/Cargo.toml:2`). 이 파일은 pgrx가 크레이트
디렉터리 이름(`engine/`)에서 생성한 템플릿 그대로이며, 수정된 적이 없다.

실제 회귀 테스트 파일(`sql/*.sql` / `expected/*.out` 쌍)도 없다.

> **결론**: pg_regress 경로는 스캐폴딩만 있고 배선되어 있지 않다.
> 이 디렉터리를 근거로 "pg_regress 스위트가 있다"고 말해서는 안 된다. → `OPS-03`

---

## 4. TypeQL 적합성 — `tests/typeql/run.py`

### 실행

```bash
python3 tests/typeql/run.py
python3 tests/typeql/run.py --db my_typeql_test
```

- 옵션은 `--db` **하나뿐**이다 (`tests/typeql/run.py:95`, 기본값 `og_typeql_test`).
- 환경변수: `PGHOST`(localhost), `PGPORT`(28816) (`tests/typeql/run.py:25-26`).
- 스크립트가 `DROP DATABASE IF EXISTS <db>` 후 `CREATE DATABASE` → `CREATE EXTENSION
  ontological CASCADE` → `og_create_graph('bookstore')`를 수행한다
  (`tests/typeql/run.py:100-110` 부근).

### 무엇이 검증되는가

docstring이 원칙을 명시한다 (`tests/typeql/run.py:4-9`):
스키마·데이터·질의가 전부 상류 TypeDB 예제 파일(`examples/typedb/bookstore/`)이며 **무수정**,
기대 결과는 그 예제의 README에 인쇄된 값이다.

판정은 순서 무관 집합 비교이고, 부동소수는 소수점 6자리로 반올림해 비교한다
(`tests/typeql/run.py:63-71`).

결과는 세 상태로 표시된다: `ok` / `FAIL` / `unsupported`
(`tests/typeql/run.py:39-46`). `unsupported`는 실패가 아니라 **미구현 표기**다 —
spec 010이 partial 상태인 것과 대응한다.

---

## 5. Neo4j Movie Graph — `tests/neo4j-movies/run.py`

`bolt/README.md:97-100`이 이 스위트를 "the real gate"라 부른다:
Neo4j의 **자체 드라이버**로 구동하기 때문이다 — "Testing our server with our own client
would not be evidence."

### 실행

```bash
# Ontological 양쪽 경로(PostgreSQL + Bolt)만
python3 tests/neo4j-movies/run.py

# 살아 있는 Neo4j와 비교까지
docker run -d --name bench-neo4j -p 27687:7687 \
  -e NEO4J_AUTH=neo4j/benchpass123 neo4j:5
python3 tests/neo4j-movies/run.py --neo4j bolt://localhost:27687

# 공식 샘플 애플리케이션 — URI만 바꾸고 나머지 그대로
python3 tests/neo4j-movies/sample_app.py
```

(`tests/neo4j-movies/README.md:32-43`)

### 옵션과 환경변수 (`tests/neo4j-movies/run.py:226-237`)

| 옵션 | 기본값 (환경변수) |
|---|---|
| `--dsn` | `OG_DSN`, 기본 `host=localhost por…`(psycopg2 DSN) |
| `--pg-host` | `localhost` |
| `--pg-port` | `OG_PGPORT` |
| `--bolt-user` | `OG_BOLT_USER`, 기본 `dev` |
| `--bolt-password` | `OG_BOLT_PASSWORD` |
| `--no-bolt` | Bolt 단계 건너뜀 |
| `--neo4j` | 비교 대상 Neo4j URI (기본 `bolt://localhost:27687`) |
| `--neo4j-user` | `NEO4J_USER`, 기본 `neo4j` |
| `--neo4j-password` | `NEO4J_PASSWORD` |
| `--no-neo4j` | Neo4j 비교 건너뜀 |

### 무엇이 검증되는가 (`tests/neo4j-movies/README.md:8-25`)

1. **어느 포트가 Bolt를 말하는가** — raw 핸드셰이크로 확인한다. PostgreSQL 포트는
   Bolt를 말하지 않으며 앞으로도 그럴 것이다 (spec 003 FR-024). 게이트웨이는 말한다.
2. **데이터셋** — 상류 `movies.cypher`를 문장 단위로 `og_cypher`에 통과. 차이는 하나:
   타입을 먼저 선언한다(`og_create_type`), 여기서는 타입이 정체성의 일부이기 때문(spec 002).
   가이드의 `CREATE CONSTRAINT` 두 문장은 제외된다.
3. **질의 24개** — 가이드 문서의 Cypher를 그대로, PostgreSQL 경로 / Bolt 경로 /
   살아 있는 Neo4j 세 경로에서 실행하고 **행 수를 비교**. 실행은 되는데 개수가 다르면 실패다.
4. **드라이버 수준** — `Node`/`Relationship` 하이드레이션, 필드 순서, 파라미터 바인딩,
   실패 후 `RESET` 복구, 명시적 트랜잭션 commit/rollback(반대 경로에서 확인), 동시 세션 8개.

의존성: `psycopg2` (필수), `neo4j` (Bolt·Neo4j 단계). `movies.cypher`는 최초 실행 시
`raw.githubusercontent.com`에서 내려받아 파일 옆에 캐시한다
(`tests/neo4j-movies/run.py:40-43`) — **네트워크가 필요하다.**

종료 코드 0의 조건: 데이터셋이 깨끗이 적재되고, 가능한 모든 경로에서 모든 질의가 일치하고,
드라이버 검사가 통과할 때 (`tests/neo4j-movies/README.md:49-50`).

---

## 6. MCP 호환 — `examples/meeting-rooms/verify_mcp.py`

```bash
python3 examples/meeting-rooms/verify_mcp.py
```

공개된 `mcp-neo4j-cypher` 서버를 **실제로 stdio로 띄워** MCP 프로토콜로 구동하고,
각 검사를 pass/fail 행렬로 출력한다 (`examples/meeting-rooms/verify_mcp.py:1-9`).
Bolt 게이트웨이가 떠 있어야 한다 (`bolt/README.md:82-88`).

---

## 표준 검증 순서 (권장)

```bash
# 0. 서버가 살아 있는지
docker exec ontological-dev bash -lc 'pg_isready -h localhost -p 28816'

# 1. 순수 Rust — 가장 빠르고 서버가 필요 없다
docker exec ontological-dev bash -lc 'cd /work/engine && cargo test'
docker exec ontological-dev bash -lc 'cd /work/bolt   && cargo test'

# 2. 확장을 재설치하고 SQL 회귀 스위트
docker exec ontological-dev bash -lc 'cd /work/engine && \
  cargo pgrx install --features pg16 --no-default-features \
    --pg-config /usr/lib/postgresql/16/bin/pg_config --sudo'
docker exec ontological-dev bash -lc 'cd /work && ./tests/run.sh'

# 3. 언어 표면 적합성
python3 tests/typeql/run.py
python3 tests/neo4j-movies/run.py

# 4. 성능 회귀
python3 bench/harness.py --compare-baseline bench/results/baseline.json
```

> `cargo pgrx install` 후에는 이미 열려 있는 백엔드가 옛 `.so`를 잡고 있을 수 있다.
> 확실히 하려면 `cargo pgrx stop pg16` → `cargo pgrx start pg16`으로 재기동할 것.

---

## 금지 / 필수

### 금지 (Forbidden)

- `cargo pgrx test`를 회귀 게이트로 쓰지 말 것 — `#[pg_test]`가 없어 실행할 것이 없다.
- `engine/tests/pg_regress/`를 동작하는 스위트로 소개하지 말 것 (§3).
- `tests/run.sh`의 종료 코드만으로 무결성 통과를 판단하지 말 것 — 단계 C는 종료 코드에
  반영되지 않는다.
- 운영 데이터베이스가 있는 서버에서 `OG_TEST_DB` 기본값을 그대로 쓰지 말 것.

### 필수 (Required)

- 확장 코드를 고쳤으면 **`cargo pgrx install` → `tests/run.sh`** 순서로 돌릴 것.
  재설치 없이 돌린 결과는 옛 바이너리에 대한 결과다.
- 새 SQL 회귀 파일에서 오류를 의도했다면 `EXPECT_ERROR`를 그 개수만큼 적을 것.
- `tests/run.sh` 출력의 `-- structural integrity --` 절을 눈으로 확인할 것.

---

<!-- affects: ops, backend -->
<!-- requires-update: docs/08_operations/05_benchmarking.md, docs/08_operations/10_improvements_ops.md -->
