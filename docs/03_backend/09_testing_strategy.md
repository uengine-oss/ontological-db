# 테스트 전략 — pg_regress SQL 회귀 + cargo test 단위 + `tests/run.sh`

> **이 문서가 답하는 질문**
> - 이 저장소에서 무엇이 어떻게 테스트되는가?
> - `tests/run.sh`는 정확히 무엇을 검증하는가? 그 판정 방식의 한계는?
> - Rust 단위 테스트는 어느 모듈에 있고 어느 모듈에 없는가?
> - 외부 데이터셋 적합성 테스트(Neo4j Movies, TypeDB Bookstore)는 무엇을 확인하는가?
> - 지금 테스트가 없는 곳은 어디인가?

---

## 1. 사실 — 4개 층

| 층 | 실행 | 대상 | 데이터베이스 필요 |
|---|---|---|---|
| ① Rust 단위 테스트 | `cargo test` | 렉서, 파서, 식별자 인코딩, RDF, PackStream, 연결 성분 | ❌ |
| ② SQL 회귀 스위트 | `tests/run.sh` | 확장 전체 (5개 파일) + 백업 왕복 + 무결성 | ✅ |
| ③ Neo4j 적합성 | `python3 tests/neo4j-movies/run.py` | Movie Graph 데이터셋 + 24개 가이드 질의 + Bolt 핸드셰이크 | ✅ (+ 선택적 Neo4j) |
| ④ TypeQL 적합성 | `python3 tests/typeql/run.py` | TypeDB bookstore 예제 원본 파일 + 그 README의 기대 출력 | ✅ |

`engine/src/lib.rs:45-48`이 이 분할을 명시한다:

> In-database behaviour is covered by the SQL regression suite in `engine/tests/sql/`,
> run with `tests/run.sh` against a live server. Pure Rust units (lexer, parser,
> identifier encoding, RDF parsing) live beside their modules and run under `cargo test`.

---

## 2. 사실 — ① Rust 단위 테스트

`#[cfg(test)]` 모듈이 있는 파일 **9개**, `#[test]` 함수 **35개**.

| 파일 | `#[test]` | 무엇을 고정하는가 |
|---|---|---|
| `engine/src/cypher/lexer.rs:266` | 4 | 패턴 토큰, 문자열/파라미터 이스케이프, **유니코드 보존**, 숫자 |
| `engine/src/cypher/parser.rs:1133` | 6 | MATCH/RETURN, 가변 길이 범위, 방향, 집계 판정, **미지원 절 거부**, 문자열 술어 |
| `engine/src/id.rs:93` | 2 | 식별자 왕복, 비트 레이아웃 비중첩 |
| `engine/src/typeql/lexer.rs:279` | 6 | 하이픈 라벨, 무따옴표 datetime 등 |
| `engine/src/typeql/parser.rs:964, 1058` | 7 | 스테이지 파이프라인 파싱 |
| `engine/src/typeql/compile.rs:776` | 2 | **연결 성분 분해** (독립 변수 분리 / 공유 변수 유지) |
| `engine/src/adapters/rdf.rs:850` | 2 | RDF 파싱 |
| `bolt/src/packstream.rs:285` | 6 | 모든 정수 폭, 모든 헤더 경계, 청킹, 중첩 구조체, UTF-8 |

**전부 순수 함수 테스트다.** 데이터베이스도 pgrx 런타임도 필요 없다.

`engine/Cargo.toml:12-13`에 `pg_test` 피처와 `pgrx-tests` dev-dependency가 있지만,
`#[pg_test]`로 표시된 테스트는 **소스에 한 건도 없다.**

---

## 3. 사실 — ② SQL 회귀 스위트

### 3.1 테스트 파일

`engine/tests/sql/`:

| 파일 | 크기 | 대상 스펙 | 내용 |
|---|---|---|---|
| `01_catalog_storage.sql` | 3.0K | 001, 002 | 타입 계층, 구간 라벨, 스토리지 테이블, 인접 세그먼트, 롤 검증 |
| `02_cypher.sql` | 2.9K | 003 | 파싱 → 컴파일 → 실행, 추상 타입, 라벨 오타 힌트 |
| `03_vector_agent_rdf.sql` | 3.3K | 004, 006, 008 | 벡터/하이브리드 검색, RDF, 에이전트 표면 |
| `04_neo4j_compat.sql` | 11.3K | Neo4j 호환 | **애플리케이션이 Neo4j에 보낼 Cypher 그대로** |
| `05_reachability.sql` | 5.1K | 003 | `og_vlp` / `og_reach_sql` / `og_reach` / CSR **동치성** + 컴파일러 전환 조건 |

`04_neo4j_compat.sql:1-4`가 그 파일의 취지를 밝힌다:

> Every statement here is Cypher an application would send to Neo4j unchanged.
> The point of the file is that none of it needs rewriting to run here.

`05_reachability.sql:1-6`도 마찬가지로 명확하다:

> Every assertion here is an *equality against og_vlp*. The faster paths are only
> worth having if they return what the slow one returns, so nothing in this file
> measures anything — it only checks answers.

### 3.2 `tests/run.sh`가 하는 일

`tests/run.sh:13-38` — 파일마다:

```
DROP DATABASE IF EXISTS og_test
CREATE DATABASE og_test
CREATE EXTENSION ontological CASCADE
psql -f <file>  →  출력 캡처
expected = grep -c 'EXPECT_ERROR' <file>
actual   = 출력에서 '^ERROR' 또는 '^psql.*ERROR' 줄 수
actual <= expected  →  ok
```

**파일마다 완전히 새 데이터베이스**를 쓴다. 파일 간 격리가 보장된다.

### 3.3 ★ 판정 방식의 한계

**행 수를 세는 것이지 출력을 비교하는 것이 아니다.**

`engine/tests/pg_regress/expected/`에는 `setup.out` 하나뿐이고,
`engine/tests/sql/`에는 대응하는 `.out` 파일이 아예 없다.
즉 **기대 출력 diff가 없다.**

결과:

- 질의가 **틀린 답**을 돌려줘도, 오류만 안 나면 통과한다.
- `EXPECT_ERROR` 개수가 맞으면 **오류가 어디서 났는지는 보지 않는다.**
  현재 `EXPECT_ERROR` 주석은 2건뿐이다
  (`01_catalog_storage.sql:62`, `02_cypher.sql:53`).
  파일에서 오류 1건이 예상되는데 **다른 곳**에서 1건이 나도 통과한다.
- `05_reachability.sql`처럼 `LIKE '%og_reach(%' AS count_distinct_uses_reach` 같은
  **불리언 컬럼**을 출력하는 테스트는, 그 값이 `f`여도 오류가 아니므로 통과한다.

→ `CODE-27`. 이것이 현재 테스트 스위트의 가장 큰 구조적 약점이다.

### 3.4 백업 왕복 게이트

`tests/run.sh:39-71`. 별도 검사로 분리한 이유가 스크립트에 적혀 있다:

> Extension-owned tables are invisible to pg_dump unless they are registered with
> pg_extension_config_dump(), and getting that wrong restores an empty graph in
> silence — so it gets its own gate.

절차:

```
CREATE DATABASE og_test → CREATE EXTENSION → examples/demo.sql 적재
before = og_node_view 개수 || '/' || og_edge_view 개수
pg_dump → 새 DB에 복원
after  = 같은 계산
query_ok = og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')
before == after && before != "0/0" && query_ok 비어있지 않음  →  ok
```

이건 **실제 값 비교**를 하는 유일한 게이트다.

### 3.5 무결성 게이트

`tests/run.sh:73-80`:

```sql
SELECT count(*) FROM og_check_integrity()
```

0이면 통과. `og_check_integrity()`(`engine/src/storage/stats.rs`)가 검사하는 4가지:

| 종류 | 검사 |
|---|---|
| `dangling_adjacency` | 인접 배열이 존재하지 않는 엣지를 가리킴 |
| `missing_adjacency` | 엣지가 양 끝점 중 한쪽에서 도달 불가 |
| `segment_length_mismatch` | `n` ≠ `array_length(nbr,1)` 또는 `nbr`/`eid` 길이 불일치 |
| `orphan_node` | 노드가 없는 타입을 참조 |

각 검사는 `LIMIT 100`이 걸려 있다.

### 3.6 pg_regress 디렉터리는 현재 깨져 있다

`engine/tests/pg_regress/sql/setup.sql`:

```sql
-- this setup file is run immediately after the regression database is (re)created
CREATE EXTENSION engine;
```

확장 이름은 `ontological`이다 (`engine/ontological.control`, `engine/Cargo.toml:2`).
`CREATE EXTENSION engine`은 존재하지 않는다.

이 디렉터리는 `cargo pgrx test`가 쓰는 스캐폴드인데, `#[pg_test]`가 하나도 없으므로
**현재 아무것도 실행되지 않는다.** → `CODE-28`.

---

## 4. 사실 — ③ Neo4j Movies 적합성

`tests/neo4j-movies/run.py` (12.4K). 헤더가 네 가지 질문을 명시한다:

1. **어느 포트가 Bolt를 말하는가** — PostgreSQL 포트는 말하지 않고 앞으로도 안 한다
   (spec 003 FR-024). Bolt 게이트웨이는 말한다 (spec 011).
   둘 다 **원시 핸드셰이크로 확인**한다(주장이 아니라).
2. **샘플 데이터셋이 적재되는가** — `neo4j-graph-examples/movies`의 `movies.cypher`를
   **문장 단위 그대로** `og_cypher`에 넣는다.
3. **샘플 질의가 도는가, Neo4j와 답이 같은가** — `queries.py`의 가이드 질의 24개를
   여기서 실행하고, Neo4j가 도달 가능하면 거기서도 실행해 **행 수를 비교한다.**
4. (헤더 4번째 항목 — 소스 참조)

`tests/neo4j-movies/queries.py`(4.0K)에 질의 목록이 있고,
`sample_app.py`(1.3K)는 드라이버 수준 검증이다.

---

## 5. 사실 — ④ TypeQL Bookstore 적합성

`tests/typeql/run.py` (19.6K). 헤더:

> The point of this suite is that nothing in it is our own invention. The schema,
> the data and the queries are the upstream TypeDB files in
> `examples/typedb/bookstore/`, unmodified; the expected results are the ones
> printed in that example's own README. If this passes, a TypeDB user's files ran
> here and produced what TypeDB's documentation says they produce.

구조:

```
tests/typeql/
  run.py
  queries/   q1.tql  q2.tql     ← 예제 README의 질의
  expected/  q1.json q2.json    ← 예제 README가 인쇄한 결과
```

README(저장소 루트)가 이 스위트를 "27가지 속성"까지 확인한다고 말한다.
`run.py`에는 `record(name, ok, detail, unsupported)` 헬퍼가 있어
**미지원 항목을 실패와 구분해 집계**한다 — 부분 구현 스펙(010)에 맞는 설계다.

`compare_sets(actual, expected)` / `canonical(obj)` 헬퍼가 있으므로
**순서 무관 집합 비교**를 한다.

---

## 6. 사실 — 커버리지 공백

### 6.1 Rust 단위 테스트가 **전혀 없는** 모듈

`#[cfg(test)]` grep 결과로 확인:

| 모듈 | 라인 | 비고 |
|---|---|---|
| **`cypher/compile.rs`** | **1,591** | ★ 이 프로젝트의 핵심. 테스트 0 |
| `cypher/mod.rs` | 823 | 쓰기 실행, 플랜 캐시, `fold_aggregates` |
| `cypher/eval.rs` | 296 | 순수 함수인데 테스트 0 |
| `cypher/views.rs` | 177 | |
| `catalog/types.rs` | 711 | `column_name`, `edit_distance`, `map_data_type`은 순수 함수 |
| `catalog/labeling.rs` | 250 | `assign`(구간 라벨 배정)은 순수 함수 |
| `storage/mod.rs` | 559 | `infer_column_type`, `type_accepts`는 순수 함수 |
| `storage/adjacency.rs` | 97 | |
| `storage/traverse.rs` | 476 | `pack`(CSR 패킹), `mark`(비트셋)은 순수 함수 |
| `storage/stats.rs` | 263 | |
| `typeql/schema.rs` | 572 | `value_type_sql`은 순수 함수 |
| `typeql/write.rs` | 688 | `typed_literal`, `json_literal`은 순수 함수 |
| `typeql/mod.rs` | 529 | `agg_sql`은 순수 함수 |
| `typeql/dump.rs` | 133 | |
| `compat/*` | 1,095 | `apoc_type`, `fulltext_expr`, `sanitize`, `default_name`은 순수 함수 |
| `vector/mod.rs` | 442 | |
| `interop/mod.rs` | 219 | |
| `agent/mod.rs` | 545 | |
| `stats.rs` | 110 | `snapshot()`은 순수 함수 |
| `spiu.rs` | 48 | SPI 필요 |
| **`bolt/src/session.rs`** | **606** | ★ `split_plan_prefix`, `to_bolt`, `to_json`, `speaks`, `record`, `Failure::from_pg`는 **전부 순수 함수** |

**`compile.rs`와 `session.rs`에 테스트가 없다는 것이 가장 큰 공백이다.**
두 파일 합쳐 2,197줄이고, 둘 다 사용자 입력을 직접 다룬다.

특히 `session.rs`의 `speaks()`, `split_plan_prefix()`, `to_bolt()`, `to_json()`,
`Failure::from_pg()`는 **의존성이 전혀 없는 순수 함수**라서 지금 당장 테스트할 수 있다.

`compile.rs`도 `blind_expr`, `multiplicity_blind`, `mentions_alias`, `quote_ident`,
`sql_str`은 `Compiler` 인스턴스 없이 테스트 가능하다.

→ `CODE-29`.

### 6.2 테스트되지 않는 동작

| 항목 | 왜 문제인가 |
|---|---|
| 동시성 (동시 append, 동시 MERGE) | 4.2/4.3절의 위험이 전부 미검증 |
| 플랜 캐시 무효화 | `CODE-01` 버그가 테스트로 잡히지 않는다 |
| 타입 승격/확장 엣지 케이스 | `int8` → `float8` → `text` 경로 (`CODE-06`) |
| 읽기 전용 트랜잭션에서의 동작 | `og_cypher_sql`의 STABLE 위반 (`CODE-02`) |
| Bolt 세션 상태 기계 | `session.rs`에 테스트 0 |
| 큰 결과 / 슈퍼노드 | 메모리 동작 미검증 |
| `og_reorganize()` | 실행 경로가 회귀 스위트에 없음 |
| RLS와 순회의 상호작용 | `interop/mod.rs:3-7`이 주장하는 핵심 성질 |

### 6.3 성능 회귀 감시

`bench/harness.py`(53KB)와 `bench/results/`가 있고 Studio가 `/benchmark.html`로 렌더링한다.
하지만 이건 **리포트**이지 게이트가 아니다. CI에서 성능 회귀를 실패시키는 장치는 없다.

`05_reachability.sql:93-96`의 `shallow_keeps_vlp`가 유일하게 **플래너 결정**을 고정하는데,
이것도 3.3절의 이유로 실질적 어서션이 아니다.

---

## 7. 사실 — 실행 방법

```bash
# ① Rust 단위 테스트 (DB 불필요)
cd engine && cargo test
cd bolt   && cargo test

# ② SQL 회귀 (라이브 서버 필요)
PGHOST=localhost PGPORT=28816 tests/run.sh

# ③ Neo4j Movies
python3 tests/neo4j-movies/run.py

# ④ TypeQL Bookstore
python3 tests/typeql/run.py [--db NAME]
```

`tests/run.sh`의 기본값 (`run.sh:6-9`): `PGHOST=localhost`, `PGPORT=28816`, `OG_TEST_DB=og_test`.
포트 28816은 `docker/`의 개발 컨테이너 포트다.

종료 코드: `tests/run.sh:82-84` — 실패가 있으면 실패 목록을 찍고 `exit 1`.

---

## 8. 결정 요약

| 결정 | 근거 | 대가 |
|---|---|---|
| 인-데이터베이스 동작은 SQL 회귀로 | `lib.rs:45-48` | 출력 diff가 없어 답이 틀려도 통과 (`CODE-27`) |
| 파일마다 새 데이터베이스 | `run.sh:17-20` | 느림 (파일당 DROP/CREATE/CREATE EXTENSION) |
| 순수 함수만 `cargo test` | `lib.rs:47-48` | 순수 함수인데도 테스트 없는 곳이 많음 (`CODE-29`) |
| 외부 데이터셋을 **수정 없이** 사용 | `tests/typeql/run.py:3-8`, `tests/neo4j-movies/run.py:9-13` | 상류 변경에 취약. 강력한 신뢰 신호 |
| 백업 왕복을 별도 게이트로 | `run.sh:39-43` | — (좋은 결정) |
| `#[pg_test]` 미사용 | 관례 | pgrx 통합 테스트 인프라가 유휴 상태 (`CODE-28`) |

---

## 금지 / 필수

- **금지**: `engine/tests/sql/`에 오류를 내는 문장을 추가하면서 `-- EXPECT_ERROR:` 주석을
  달지 않는 것. 파일 전체가 FAIL로 뒤집힌다.
- **금지**: `bootstrap.sql`에 사용자 데이터 테이블을 추가하면서
  `pg_extension_config_dump` 등록을 빠뜨리는 것. `tests/run.sh`의 백업 게이트가 잡지만,
  잡히기 전에 프로덕션에서 데이터가 사라질 수 있다.
- **금지**: `engine/tests/sql/05_reachability.sql`의 어서션을 지우면서 컴파일러의
  도달성 전환 조건을 바꾸는 것.
- **필수**: `compile.rs` / `session.rs`처럼 순수 함수를 가진 파일에 로직을 추가하면
  같은 파일 `#[cfg(test)]`에 테스트를 추가한다. DB가 필요 없으므로 변명의 여지가 없다.
- **필수**: 도달성 전환 조건(`multiplicity_blind` / `prefer_reachability`)을 바꿀 때는
  **먼저** `05_reachability.sql`에 케이스를 추가한다.
- **필수**: 새 Neo4j 호환 표면을 추가하면 `04_neo4j_compat.sql`에
  **Neo4j에 그대로 보낼 수 있는 Cypher**로 케이스를 추가한다.

<!-- affects: backend, operations -->
<!-- requires-update: 08_operations/ -->
