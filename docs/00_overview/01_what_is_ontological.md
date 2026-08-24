# Ontological이란 무엇인가

> **이 문서가 답하는 질문**
> - 이 시스템은 무엇인가? 한 문장으로 말하면?
> - 어떤 문제를 풀려고 만들었는가?
> - 누가 쓰는가? 사람인가, AI인가?
> - "PostgreSQL 확장"이라는 말이 실제로 무엇을 의미하는가?

---

## 한 문장

**Ontological은 `CREATE EXTENSION` 한 줄로 표준 PostgreSQL 안에 설치되는,
Cypher 중심의 온톨로지 그래프 데이터베이스다.**

```sql
CREATE EXTENSION ontological CASCADE;
```

이 한 줄이 이 프로젝트의 전부를 규정한다. 포크가 아니고, 별도 서버가 아니고,
PostgreSQL 옆에 붙는 사이드카도 아니다. 여러분의 PostgreSQL **안에서** 동작한다.
근거: [`.specify/memory/constitution.md`](../../.specify/memory/constitution.md)
원칙 I ("포크가 아닌 확장이다 — NON-NEGOTIABLE").

---

## 비개발자를 위한 설명

### 그래프 데이터베이스가 필요한 이유

관계형 데이터베이스는 "홍길동의 주문 목록"처럼 **한 단계 건너뛰는 질문**에 강하다.
반면 "홍길동과 같은 프로젝트를 했던 사람들이 참여한 다른 프로젝트에서 쓰인 부품"처럼
**여러 단계를 연달아 건너뛰는 질문**에는 약하다. 단계(홉, hop)마다 JOIN이 하나씩 늘고
비용이 곱해지기 때문이다.

그래프 데이터베이스는 이 "연결을 따라가는" 동작 자체를 저장 구조에 새겨 넣어 해결한다.

### 그런데 왜 PostgreSQL 안에서?

전용 그래프 데이터베이스(Neo4j 등)를 쓰면 그래프 질의는 빨라지지만,
회사가 이미 쓰고 있는 것들을 전부 두 벌로 유지해야 한다 —
백업, 복제, 권한 관리, 모니터링 도구, 드라이버, 마이그레이션 스크립트, 운영 인력.
그리고 "고객 테이블의 이 행"과 "그래프의 이 노드"가 같은 트랜잭션 안에 있지 않다.

Ontological의 선택은 **그래프를 PostgreSQL 안으로 가져오는 것**이다. 그 결과:

| 얻는 것 | 어떻게 |
|---|---|
| 백업 | `pg_dump` 가 그래프까지 함께 뜬다 ([`engine/sql/bootstrap.sql:403-447`](../../engine/sql/bootstrap.sql) 의 `pg_extension_config_dump` 등록) |
| 트랜잭션 | 그래프 변경이 기존 테이블 변경과 같은 트랜잭션에 들어간다 |
| 권한/보안 | PostgreSQL 역할과 RLS가 그래프에도 그대로 적용된다 ([`engine/src/interop/mod.rs:1-8`](../../engine/src/interop/mod.rs)) |
| 복제 | 모든 구조가 평범한 힙 릴레이션이라 스트리밍 복제가 그대로 동작한다 (spec 007 plan.md) |
| 운영 도구 | psql, pgAdmin, Datadog, Supabase — 전부 그대로 |

### AI가 1급 사용자다

이 데이터베이스는 사람만 쓰라고 만들지 않았다.
헌법 원칙 VIII("AI 에이전트가 1급 사용자 — NON-NEGOTIABLE")에 따라,
LLM 에이전트가 직접 질의를 작성하는 것을 전제로 설계되었다.

구체적으로:
- 스키마를 기계가 읽을 수 있는 JSON으로 내주고, 토큰 예산에 맞춰 잘라 준다
  (`og_schema(graph, token_budget)`, [`engine/src/agent/mod.rs:21-63`](../../engine/src/agent/mod.rs))
- 오류가 "무엇이 틀렸는가"뿐 아니라 **"가장 가까운 유효한 대안"** 을 함께 낸다
  (편집 거리 기반 후보 제안, [`engine/src/catalog/types.rs:129-138, 236`](../../engine/src/catalog/types.rs))
- 결과가 비었을 때 "패턴의 어느 지점에서 비었는가"를 짚어 준다
  (`og_diagnose_empty`, [`engine/src/cypher/mod.rs:745-803`](../../engine/src/cypher/mod.rs))
- 사실의 유효 시각과 기록 시각을 보존해 "언제 기준으로 참인가"를 물을 수 있다
  (`og_data.og_history`, [`engine/sql/bootstrap.sql:310-322`](../../engine/sql/bootstrap.sql))

---

## 개발자를 위한 설명

### 무엇으로 만들어졌는가 (Facts)

| 구성 요소 | 실체 | 위치 |
|---|---|---|
| 엔진 | pgrx 0.19.2 기반 Rust cdylib | [`engine/`](../../engine/) — [`Cargo.toml`](../../engine/Cargo.toml) |
| 부트스트랩 스키마 | `CREATE EXTENSION` 시 실행되는 SQL 448줄 | [`engine/sql/bootstrap.sql`](../../engine/sql/bootstrap.sql) |
| 접근 경로 함수 | 전부 `LANGUAGE sql` 338줄 | [`engine/sql/access.sql`](../../engine/sql/access.sql) |
| Bolt 게이트웨이 | 별도 Rust 바이너리 (pgrx 아님) | [`bolt/`](../../bolt/) — [`bolt/src/main.rs`](../../bolt/src/main.rs) |
| Studio 콘솔 | Node.js 백엔드 + 순수 JS 프론트 | [`portal/`](../../portal/) |
| 벤치마크 하네스 | Python | [`bench/harness.py`](../../bench/harness.py) |

의존 확장은 pgvector 하나다: `requires = 'vector'`
([`engine/ontological.control`](../../engine/ontological.control)).

### 무엇을 제공하는가

**하나의 그래프 위에 두 개의 질의 언어**

```sql
-- Cypher
SELECT og_cypher('kb', $$ MATCH (v:Vehicle) RETURN v.model $$);

-- TypeQL — 같은 카탈로그, 같은 저장소, 같은 트랜잭션
SELECT og_typeql('bookstore', $$ match $b isa book; fetch { "t": $b.title }; $$);
```

Cypher는 [`engine/src/cypher/`](../../engine/src/cypher/) (spec 003),
TypeQL은 [`engine/src/typeql/`](../../engine/src/typeql/) (spec 010) 가 담당한다.
둘 다 `og_data.*` 라는 같은 저장소를 읽고 쓴다
([`engine/src/typeql/compile.rs:1-13`](../../engine/src/typeql/compile.rs)).

**노드뿐 아니라 관계(엣지)에도 붙는 벡터**

```sql
SELECT og_add_embedding('kb', 'CITES', 'context_emb', 1536, 'cosine', 'note');
```

임베딩이 별도 저장소가 아니라 **타입 테이블의 `vector(N)` 컬럼**이기 때문에,
엣지 타입 테이블도 노드 타입 테이블과 똑같이 벡터 컬럼과 HNSW 인덱스를 갖는다
([`engine/src/vector/mod.rs:1-12, 32-75`](../../engine/src/vector/mod.rs)).

**Neo4j 드라이버가 URI만 바꿔 접속**

Bolt 4.4 게이트웨이가 별도 프로세스로 뜨고, 받은 Cypher를 `og_cypher()` 로 넘긴다.
게이트웨이는 파서도 플래너도 캐시도 사용자 저장소도 갖지 않는다
([`bolt/src/main.rs:1-15`](../../bolt/src/main.rs), [`bolt/README.md`](../../bolt/README.md)).

---

## 어떤 문제를 푸는가 (Decisions)

### 문제 1 — Apache AGE의 홉 비용

Apache AGE는 노드/엣지를 일반 힙 테이블에 담고 프로퍼티를 `agtype`(JSON)에 넣는다.
이웃 확장이 `degree`회 인덱스 조회 + `degree`회 랜덤 힙 페치가 된다.

**결정**: 인접 정보를 **CSR형 세그먼트**로 저장한다.
한 힙 튜플이 한 노드의 이웃을 최대 256개까지 두 개의 정렬된 `int8[]`로 담는다.
256 × 8B × 2 = 4KB로 8KB 힙 페이지 하나에 들어간다.
근거: [`engine/sql/bootstrap.sql:186-214`](../../engine/sql/bootstrap.sql),
[`engine/src/storage/adjacency.rs:1-15`](../../engine/src/storage/adjacency.rs).

> **주의**: 실측 결과 1홉 인덱스 경로의 페이지 읽기량은 AGE와 큰 차이가 없고,
> AGE의 붕괴는 `*1..n` 가변 길이 연산자에 집중되어 있다.
> 이 뉘앙스는 [`docs/benchmark.md`](../benchmark.md) 와
> [`README.md`](../../README.md) 의 "Four things in that table" 절이 직접 밝히고 있다.
> 이 프로젝트는 그 사실을 감추지 않는다.

### 문제 2 — 프로퍼티마다 JSON 파싱

**결정**: 선언된 프로퍼티는 **실제 타입 컬럼**이 된다.
타입 카탈로그가 `og_data.n_<type_id>` 테이블을 생성하고,
`og_add_property` 가 `ALTER TABLE ... ADD COLUMN p_<prop> <type>` 을 실행한다
([`engine/src/catalog/types.rs:414-417, 550-553`](../../engine/src/catalog/types.rs)).
미선언 프로퍼티만 `__ext jsonb` 로 떨어진다.

여기서 한 걸음 더 나간 결정이 있다: **Cypher 앱은 아무것도 선언하지 않는다**
(Neo4j에는 선언할 스키마가 없으므로). 그래서 쓰기 시점에 새 프로퍼티를 자동으로
실컬럼으로 승격시키고, 나중에 타입이 충돌하면 **text로 단방향 확장(widening)** 한다
([`engine/src/storage/mod.rs:76-158`](../../engine/src/storage/mod.rs)).
`vector(1536)` 이나 `timestamptz` 처럼 **의도적으로 선언된 타입은 확장 대상에서 제외**된다
(`WIDENABLE = ["bool","int8","float8"]`, [`engine/src/storage/mod.rs:64`](../../engine/src/storage/mod.rs)).

### 문제 3 — 상속 판정의 런타임 재귀

`MATCH (v:Vehicle)` 가 `Car`, `EV`, `Truck` 까지 찾아야 한다.
재귀 CTE로 계층을 펼치면 계층이 깊어질수록 느려진다.
헌법 원칙 IV는 이 방식을 **금지**한다.

**결정**: 계층에 **구간(nested-set) 라벨**을 부여한다.
`X ⊑ Y ⟺ Y.lft ≤ X.lft AND X.rgt ≤ Y.rgt` — 인덱스 범위 비교 한 번이다
([`engine/src/catalog/labeling.rs:1-16`](../../engine/src/catalog/labeling.rs),
[`engine/sql/bootstrap.sql:59-80`](../../engine/sql/bootstrap.sql)).
게다가 이 판정은 **컴파일 타임에** 끝나서, 실행 시점에는 이미
구체 테이블들의 `UNION ALL` 뷰로 바뀌어 있다
([`engine/src/cypher/views.rs:1-17`](../../engine/src/cypher/views.rs)).

### 문제 4 — 깊은 순회의 경로 폭발

Cypher의 가변 길이 매치는 **경로 1개당 1행**을 낸다.
`count(b)` 는 걸음 수를, `count(DISTINCT b)` 는 노드 수를 센다.
평균 차수 20인 그래프에서 홉이 하나 늘 때마다 20배다 — 6홉에 49초.

**결정**: 질의가 경로의 다중도를 **관측할 수 없을 때**만
방문집합 BFS(`og_reach`)로 컴파일한다. 6홉 49,334ms → 71ms
([`docs/deep-traversal.md`](../deep-traversal.md),
[`engine/src/storage/traverse.rs:1-25`](../../engine/src/storage/traverse.rs)).

**보수성**이 이 결정의 핵심이다. 다음 중 하나라도 있으면 적용하지 않는다:
- 경로 변수 또는 관계 변수 바인딩 ([`engine/src/cypher/compile.rs:655-660, 865`](../../engine/src/cypher/compile.rs))
- 중복을 관측할 수 있는 프로젝션 ([`engine/src/cypher/compile.rs:339-349`](../../engine/src/cypher/compile.rs))
- `WITH` 절의 존재 ([`engine/src/cypher/compile.rs:340`](../../engine/src/cypher/compile.rs))
- 플래너 통계로 계산한 손익분기 깊이 미만 ([`engine/src/cypher/compile.rs:34-78`](../../engine/src/cypher/compile.rs))

마지막 항목은 회귀 이력에서 나왔다: 무조건 적용했더니 2홉이 오히려 느려졌다
([`engine/src/cypher/compile.rs:20-33`](../../engine/src/cypher/compile.rs) 주석).

---

## 아직 아닌 것 (Facts)

정직하게 적는다. 자세한 경계는 [`05_spec_status.md`](05_spec_status.md) 를 볼 것.

- **Table Access Method가 아니다.** 힙 릴레이션 위에 얹은 구조다.
  이유와 로드맵은 spec 001 `plan.md`의 Complexity Tracking에 있다.
- **Cypher가 최상위 문법이 아니다.** `og_cypher('g', $$…$$)` 함수 호출로 진입한다.
  PostgreSQL 16에 최상위 파서 대체 훅이 없고, 헌법 원칙 I이 원칙 II를 이긴다
  ([`specs/003-cypher-query-engine/plan.md`](../../specs/003-cypher-query-engine/plan.md) Complexity Tracking).
- **`UNION` 이 동작하지 않는다.** 파서는 받아들이지만
  ([`engine/src/cypher/parser.rs:161-168`](../../engine/src/cypher/parser.rs)),
  컴파일러가 `Query.union` 필드를 읽지 않는다. 자세한 내용은
  [`../01_architecture/08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) ARCH-01.
- **샤딩이 없다.** 읽기 복제만 동작한다.
  식별자에 9비트 shard 필드는 예약되어 있으나 항상 0이 쓰인다
  ([`engine/src/storage/mod.rs:33`](../../engine/src/storage/mod.rs)의 `id::make_id(0, tid, local)`).
- **SPARQL이 없다.** RDF 적재/덤프는 되지만 SPARQL 질의는 미구현이다 (spec 006 partial).
- **TypeQL 함수(`fun`)가 평가되지 않는다.** 파싱·저장·덤프는 되고, 호출은 명시적으로 거부된다
  ([`engine/src/typeql/compile.rs:486-488`](../../engine/src/typeql/compile.rs)).

---

## Decisions vs Facts 요약

### Decisions (설계 판단)
1. PostgreSQL 확장으로만 존재한다. 커널 패치·포크를 채택하지 않는다. (헌법 I)
2. Cypher는 SQL로 **컴파일**된다. 함수 파이프라인으로 해석하지 않는다. (헌법 II, spec 003)
3. 인접 정보는 CSR형 세그먼트로 저장한다. (헌법 III, spec 001)
4. 상속은 구간 라벨로 상수 시간에 판정한다. (헌법 IV, spec 002)
5. 벡터는 pgvector를 재사용하고, 임베딩은 일반 프로퍼티 컬럼이다. (헌법 V, spec 004)
6. 어댑터(RDF/TypeQL/Bolt)는 코어 의미론을 갖지 않는다. (헌법 VI)
7. 읽기는 SQL 한 문장, 쓰기는 Rust 절차 실행. 정확성이 단일 문장보다 우선한다. (spec 003 plan)

### Facts (현재 상태)
- 엔진 Rust 소스 합계 약 12,900줄, Bolt 게이트웨이 약 1,035줄. 실측은 [`04_repository_map.md`](04_repository_map.md).
- 공개 SQL 함수는 `#[pg_extern]` 78개 + `access.sql` 의 `LANGUAGE sql` 함수/뷰.
- PostgreSQL 16 기준 (`default = ["pg16"]`), pg13~pg19 feature 플래그 존재
  ([`engine/Cargo.toml`](../../engine/Cargo.toml)).
- 확장 버전은 `0.1.0` 고정이며 업그레이드 스크립트가 없다
  ([`engine/ontological.control`](../../engine/ontological.control), `engine/sql/` 디렉터리에 2개 파일뿐).

---

## Forbidden / Required

**Forbidden**
- "PostgreSQL을 포크했다", "별도 서버를 띄운다"고 서술하지 말 것. 사실이 아니다.
  단, Bolt 게이트웨이(`bolt/`)만은 **별도 프로세스**다 — 이 구분을 흐리지 말 것.
- 벤치마크 수치를 근거 문서 없이 인용하지 말 것. 반드시
  [`docs/benchmark.md`](../benchmark.md) 또는 [`docs/deep-traversal.md`](../deep-traversal.md) 를 링크할 것.
- "그래프 DB니까 무조건 빠르다"고 쓰지 말 것.
  3홉에서 Neo4j가 11배 빠르고, 손으로 쓴 재귀 CTE가 모든 구간에서 더 빠르다
  ([`README.md`](../../README.md) 벤치마크 절이 직접 밝힘).

**Required**
- 기능 지원 여부를 언급할 때는 [`05_spec_status.md`](05_spec_status.md) 를 근거로 링크할 것.
- 새 결정을 추가하면 ADR로 `99_decisions/` 에 기록할 것.

<!-- affects: overview, architecture, api, llm -->
<!-- requires-update: 00_overview/05_spec_status.md, 01_architecture/01_system_overview.md, 99_decisions/ -->
