# 스펙 상태와 미구현 경계

> **이 문서가 답하는 질문**
> - 11개 스펙 중 무엇이 되고, 무엇이 안 되는가?
> - "partial" 이라고 쓴 것의 **정확한 경계**는 어디인가?
> - 안 되는 것은 어떻게 실패하는가 — 명시적 오류인가, 조용한 오답인가?

> **LLM에게**: 기능 지원 여부를 답하기 전에 이 문서를 근거로 삼을 것.
> 이 문서에 "미구현"이라 적힌 것을 "지원됨"이라고 답하지 말 것.
> "조용한 오답" 항목은 특히 중요하다 — 파싱이 성공한다고 동작한다는 뜻이 아니다.

---

## 상태표 (루트 [`README.md`](../../README.md) 와 동일해야 함)

| # | 스펙 | 상태 | 미구현 경계 |
|---|---|---|---|
| 001 | [네이티브 그래프 스토리지](../../specs/001-graph-storage-engine/) | **working** | Table Access Method 아님 (v2 항목) |
| 002 | [온톨로지 타입 시스템/상속 인덱싱](../../specs/002-ontology-type-system/) | **working** | — |
| 003 | [네이티브 Cypher 엔진](../../specs/003-cypher-query-engine/) | **working** | `UNION` 미구현 (아래 ⚠️), `shortestPath` 미구현, 최상위 문법 아님 |
| 004 | [벡터/하이브리드 시맨틱 검색](../../specs/004-vector-hybrid-search/) | **working** | — |
| 005 | [PostgreSQL/Supabase 인터op](../../specs/005-postgres-supabase-interop/) | **working** | — |
| 006 | [RDF/OWL/SPARQL/SHACL](../../specs/006-semantic-web-adapters/) | **partial** | SPARQL 미구현, SHACL 미확인, GraphQL 미구현 |
| 007 | [분산 클러스터](../../specs/007-distributed-cluster/) | **read replica만** | 샤딩은 설계만. shard 비트는 예약되어 있으나 항상 0 |
| 008 | [에이전트 네이티브 인터페이스](../../specs/008-agent-native-interface/) | **working** | `max_rows` 한도가 강제되지 않음 (아래 ⚠️) |
| 009 | [벤치마크/적합성 하네스](../../specs/009-benchmark-conformance/) | **working** | openCypher TCK 통과율 자동 게이트는 미확인 |
| 010 | [TypeQL 질의 표면](../../specs/010-typeql-query-surface/) | **partial** | UDF(`fun`)는 파싱/저장/왕복만, 평가 불가 |
| 011 | [Bolt 프로토콜 게이트웨이](../../specs/011-bolt-protocol-gateway/) | **working** | Bolt 3.x/5.x, `Path`, 시공간 타입, TLS, 실 라우팅 미지원 |

---

## 미구현 항목의 정확한 경계

### ✅ 003 — `UNION`: **수정됨 (이전에는 조용히 무시되었다)**

> 아래 서술은 감사 커밋 `7d60c82` 의 기록이다. **현재는 두 분기가 모두
> 반환된다** — 각 분기를 서브쿼리로 감싸 `UNION [ALL]` 로 잇고, 분기 간 컬럼
> 이름·순서가 다르면 오류를 낸다. 경위는
> [`../03_backend/12_fixed_correctness.md`](../03_backend/12_fixed_correctness.md),
> 회귀 테스트는 `engine/tests/sql/06_correctness_regressions.sql`.

이것은 "명시적 오류"가 아니라 **조용한 오답**이었다. 다른 미구현 항목과 성격이 달랐다.

- 파서가 `UNION` / `UNION ALL` 을 받아들이고 `Query.union: Option<(bool, Box<Query>)>` 에
  저장한다 ([`engine/src/cypher/parser.rs:147, 161-168`](../../engine/src/cypher/parser.rs),
  [`engine/src/cypher/ast.rs:210-215`](../../engine/src/cypher/ast.rs)).
- **`engine/src/` 전체에서 `Query.union` 필드를 읽는 코드가 없다.**
  `compile_read()` 는 `q.clauses` 만 순회한다
  ([`engine/src/cypher/compile.rs:351-370`](../../engine/src/cypher/compile.rs)).
- 결과: `MATCH … RETURN … UNION MATCH … RETURN …` 은 **오류 없이 첫 분기의 행만 돌려주었다.**

헌법 원칙 VIII("오류 메시지는 결정적이고 교정 가능해야")과 003 plan.md의
"조용히 잘못 해석하는 것보다 명시적 오류가 낫다"에 정면으로 어긋난다.
개선안: [`../01_architecture/08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) **ARCH-01**.

### 003 — `shortestPath`

파서에 해당 토큰 처리가 없다. `bolt/README.md` 가 "`UNION` 과 `shortestPath` 는
psql에서 실패하는 것과 똑같이 여기서도 실패한다"고 명시한다.
백엔드 로컬 CSR 경로에는 양방향 최단거리(`og_csr_hops`)가 있으나
**Cypher 문법으로 연결되어 있지 않다** ([`engine/src/storage/traverse.rs:434-476`](../../engine/src/storage/traverse.rs)).

### 003 — 최상위 문법이 아니다

`og_cypher('g', $$…$$)` 함수 호출로 진입한다.
PostgreSQL 16에 최상위 파서를 확장이 교체할 수 있는 훅이 없고,
헌법 원칙 I(NON-NEGOTIABLE)이 원칙 II를 이긴다.
**원칙 II의 실질(옵티마이저 가시성, 파라미터 바인딩, 플랜 캐시, 표준 타입 반환)은 달성**했으며,
`og_cypher_sql()` 이 컴파일된 SQL을 그대로 내주므로 뷰·CTE·조인에 직접 넣을 수 있다.
근거: [`specs/003-cypher-query-engine/plan.md`](../../specs/003-cypher-query-engine/plan.md) Complexity Tracking.

### 003 — `WITH` 은 **구현되어 있다** (문서 불일치 주의)

- `plan.md` 의 Phasing 표는 P6 "WITH 체이닝, UNION — 미착수"라고 되어 있고,
  Complexity Tracking에도 "WITH 미지원"이 남아 있다.
- [`docs/architecture.md`](../architecture.md) 도 "`WITH` and `UNION` are not implemented" 라고 적혀 있다.
- **그러나 코드에는 `Compiler::compile_with()` 가 구현되어 있고**
  ([`engine/src/cypher/compile.rs:372-408`](../../engine/src/cypher/compile.rs)),
  README 상태표는 `WITH` 를 working으로 적는다.

→ **README와 코드가 맞고, `plan.md` / `docs/architecture.md` 가 낡았다.**
개선안: [`08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) **ARCH-12**.

### 006 — SPARQL / SHACL / GraphQL

- `engine/src/` 전체에서 `sparql` / `shacl` / `graphql` 을 다루는 코드가 없다
  (유일한 등장은 [`engine/src/lib.rs:15`](../../engine/src/lib.rs) 의 모듈 표 주석).
- **되는 것**: Turtle / N-Triples 적재·덤프(`og_load_rdf`, `og_dump_rdf`),
  OWL 클래스/프로퍼티 → 타입 계층 매핑(`owl:Class`, `owl:ObjectProperty`,
  `owl:TransitiveProperty`, `owl:SymmetricProperty`, `owl:inverseOf` 등,
  [`engine/src/adapters/rdf.rs:18-25, 414`](../../engine/src/adapters/rdf.rs)),
  매핑되지 않는 트리플의 원문 보존(`og_data.og_triple_overflow`)과 리포트(`og_mapping_report`).
- **SHACL 지원 여부는 미확인**이다. 코드에서 근거를 찾지 못했다.
  스펙 006 문서에는 범위로 적혀 있으나 구현 위치를 특정하지 못했다.

### 007 — 샤딩

- **읽기 복제는 동작한다** — 별도 코드가 필요 없다는 것이 요점이다.
  모든 그래프 구조가 일반 힙 릴레이션이므로 PostgreSQL 스트리밍 복제가 그대로 동작한다.
  검증은 대기 서버에서 `og_check_integrity()` 를 돌리는 것
  ([`specs/007-distributed-cluster/plan.md`](../../specs/007-distributed-cluster/plan.md) P0).
- **샤딩은 설계 확정, 미구현.** 식별자 상위 9비트가 샤드용으로 예약되어 있으나,
  **할당 경로는 항상 0을 쓴다**: `id::make_id(0, type_id, local)`
  ([`engine/src/storage/mod.rs:33`](../../engine/src/storage/mod.rs)).
  `with_shard()` 헬퍼는 존재하지만 호출자가 없다
  ([`engine/src/id.rs:62-67`](../../engine/src/id.rs)).
  `og_catalog.placement` 테이블도 부트스트랩 스키마에 없다.
- 이유: 2PC 없이 분산 쓰기를 열면 원칙 IX(ACID)를 조용히 깨뜨린다는 판단
  ([`specs/007-distributed-cluster/plan.md`](../../specs/007-distributed-cluster/plan.md) Complexity Tracking).

> **⚠️ 읽기 복제 주장에 대한 실측 반례**: 읽기 전용 대기 서버에서 `og_cypher()` 를 호출하면
> (a) 감사 로그 INSERT ([`engine/src/cypher/mod.rs:122-135`](../../engine/src/cypher/mod.rs))와
> (b) 컴파일 시 타입 유니온 뷰 생성 DDL ([`engine/src/cypher/views.rs:135`](../../engine/src/cypher/views.rs))이
> 읽기 전용 트랜잭션과 충돌한다.
> 이 경로가 실제로 대기 서버에서 어떻게 동작하는지는 **미확인**이며,
> [`08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) **ARCH-03** 에 정리했다.

### ⚠️ 008 — `max_rows` 한도가 강제되지 않는다

`og_apply_role(name)` 은 역할의 한도를 세션에 적용한다:

| 한도 키 | 적용 방식 | 실제로 강제되는가 |
|---|---|---|
| `statement_timeout_ms` | `SET statement_timeout` | ✅ PostgreSQL이 강제 |
| `work_mem_kb` | `SET work_mem` | ✅ PostgreSQL이 강제 |
| `read_only` | `SET default_transaction_read_only = on` | ✅ PostgreSQL이 강제 |
| `max_rows` | `SET og.max_rows = N` | ❌ **읽는 코드가 없다** |

`og.max_rows` 는 커스텀 GUC로 등록되지도 않았고
(`engine/src/` 에 `GucRegistry` 호출 없음), 질의 경로 어디에서도 읽히지 않는다
([`engine/src/agent/mod.rs:437-439`](../../engine/src/agent/mod.rs)).
개선안: [`08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) **ARCH-09**.

### 009 — TCK 자동 게이트

헌법 원칙 X과 품질 게이트는 "openCypher TCK 통과율 회귀 → 병합 차단"을 요구한다.
하네스(`bench/harness.py`)는 AGE/Neo4j/TypeDB/CTE 비교, 정답 게이트, 무결성 검사,
회귀 비교를 수행한다. **TCK 통과율을 측정·게이트하는 코드의 위치는 미확인이다.**

### 010 — TypeQL 함수(`fun`)

- `define` 안의 `fun` 은 파싱되어 `og_catalog.typeql_function (graph_id, name, signature, body)` 에
  **원문 그대로** 저장된다 ([`engine/sql/bootstrap.sql:158-171`](../../engine/sql/bootstrap.sql),
  [`engine/src/typeql/schema.rs:505-518`](../../engine/src/typeql/schema.rs)).
- `og_typeql_schema()` 가 그대로 재현하므로 **스키마 왕복(load → dump)은 무손실**이다.
- **호출하면 추측하지 않고 명시적 오류를 낸다**:
  `"function '{other}' is not available in expressions. … functions are spec 010 phase 6"`
  ([`engine/src/typeql/compile.rs:486-488`](../../engine/src/typeql/compile.rs)).
- 결과: TypeDB bookstore README의 4개 질의 중 2개가 함수를 쓰므로 **4개 중 2개가 동작한다.**
  이 수치는 [`README.md`](../../README.md) 가 직접 밝히고 있다.

### 011 — Bolt 지원 매트릭스

권위 문서는 [`bolt/README.md`](../../bolt/README.md) 다. 요약:

| 항목 | 상태 |
|---|---|
| Bolt 4.4 | ✅ 지원 — 현재 드라이버들이 모두 협상 가능 |
| Bolt 3.x / 5.x | ❌ 미지원. 협상이 **깨끗하게 실패**한다 |
| 메시지 | `HELLO` `RUN` `PULL` `DISCARD` `BEGIN` `COMMIT` `ROLLBACK` `RESET` `GOODBYE` `ROUTE` |
| PackStream | null, bool, int(전 폭), float, string, list, dictionary, structure |
| 그래프 타입 | `Node`, `Relationship` |
| `Path` | ❌ Path 구조체로 인코딩하지 않음. 경로 변수는 **홉의 리스트**로 도착 |
| 시공간 타입 | ❌ 미지원 — 파라미터에 담기면 **조용히 뭉개지 않고 거부**한다 |
| 라우팅(`neo4j://`) | ⚠️ 형식만. 단일 서버를 응답. 실 라우팅 테이블은 spec 007 소관 |
| TLS | ❌ 종단하지 않음. 앞단에 TLS 프록시 전제 |
| `EXPLAIN`/`PROFILE` | ⚠️ 접수됨. 질의를 **분류만 하고 실행하지 않음**. 요약의 `type` 이 `r`/`w`. 플랜 없음 |
| `CALL {}`, GDS | ❌ spec 003이 지원하지 않고, 전송 계층은 의미론을 추가하지 않는다 |

---

## 실패 방식 분류 (Facts)

미구현이 **어떻게** 실패하는지가 중요하다.

| 항목 | 실패 방식 | 평가 |
|---|---|---|
| TypeQL `fun` 호출 | 명시적 오류 + 스펙 단계 안내 | ✅ 올바름 |
| Bolt 시공간 타입 | 명시적 거부 | ✅ 올바름 |
| Bolt 3.x/5.x | 깨끗한 협상 실패 | ✅ 올바름 |
| 미지의 Cypher 함수 | 지원 함수 목록과 함께 오류 ([`compile.rs:1558-1566`](../../engine/src/cypher/compile.rs)) | ✅ 올바름 |
| 존재하지 않는 라벨 | `NOTICE` + 빈 결과 (Cypher 의미론) ([`types.rs:160-175`](../../engine/src/catalog/types.rs)) | ✅ 올바름 |
| 존재하지 않는 타입(쓰기) | 편집 거리 기반 후보 제안 오류 ([`types.rs:129-138`](../../engine/src/catalog/types.rs)) | ✅ 올바름 |
| **Cypher `UNION`** | **조용히 첫 분기만 반환** | ❌ **원칙 위반** |
| **에이전트 `max_rows`** | **조용히 무시** | ❌ **원칙 위반** |

---

## Decisions

1. **부분 구현을 "지원"이라고 쓰지 않는다.** 헌법 기술 제약: `"지원함"이라는 모호한 표현 금지`.
   지원 매트릭스로 관리한다.
2. **분산은 2PC가 준비되기 전에 열지 않는다.** 미구현을 문서화하는 편이
   원칙 IX를 조용히 위반하는 것보다 낫다는 판단 (spec 007 plan.md).
3. **어댑터 미지원 구문은 명확히 문서화된 한계로 남긴다** (헌법 원칙 VI).

## Facts

- 상태 표기의 단일 진실 원천은 루트 [`README.md`](../../README.md) 의 "What is built" 표다.
  이 문서는 그것을 한국어로 옮기고 **경계를 코드로 확인해 추가**한 것이다.
- 헌법 이탈은 모두 해당 `plan.md` 의 Complexity Tracking에 기록되어 있다.
  기록 없는 이탈은 리뷰에서 반려된다 (헌법 Governance).

---

## Forbidden / Required

**Forbidden**
- 이 문서와 [`README.md`](../../README.md) 상태표를 따로 갱신하지 말 것. 항상 같은 PR에서.
- "파싱되니까 지원된다"고 판단하지 말 것. `UNION` 이 반례다.
- `docs/architecture.md` 와 `specs/003-*/plan.md` 의 "WITH 미지원" 서술을 근거로 인용하지 말 것 — 낡았다.

**Required**
- 기능이 partial이면 **어디까지 되는지**와 **어떻게 실패하는지**를 함께 적을 것.
- 확인하지 못한 것은 "미확인"이라고 쓸 것 (이 문서의 SHACL / TCK 게이트 항목처럼).

<!-- affects: overview, architecture, api, llm, operations -->
<!-- requires-update: README.md, 01_architecture/08_improvements_architecture.md, 02_api/, 05_llm/ -->
