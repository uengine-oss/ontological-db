# 프로토콜 진입면 — SQL / Bolt / PostgREST / Studio

> **이 문서가 답하는 질문**
> - 이 시스템에 들어오는 길이 몇 개인가?
> - 각 진입면은 인증·권한·트랜잭션·감사를 어떻게 처리하는가?
> - 어떤 진입면이 무엇을 보장하고, 무엇을 보장하지 **않는가**?
> - 새 진입면을 추가할 때 지켜야 할 규칙은?

---

## 0. 원칙

헌법 원칙 VI: **코어는 하나, 표준은 어댑터로.**
spec 011 설계 결정 2: **두 번째 인증·감사 경로를 만들지 않는다.**

이 두 문장이 모든 진입면의 설계를 규정한다. 진입면은 **전송(transport)** 이지
**의미론(semantics)** 이 아니다.

---

## 1. 네 개의 진입면 한눈에

| | ① SQL 함수 | ② Bolt 4.4 | ③ PostgREST | ④ Studio HTTP |
|---|---|---|---|---|
| 프로세스 | postgres 백엔드 | `ontological-bolt` (별도) | PostgREST (외부) | Node.js (별도) |
| 필수? | ✅ 필수 | 선택 | 선택 | 선택 |
| 클라이언트 | psql, JDBC, psycopg, supabase-js | Neo4j 드라이버 | HTTP/REST | 브라우저 |
| 인증 주체 | PostgreSQL 역할 | **PostgreSQL 역할** (HELLO 자격증명) | PostgREST JWT → PG 역할 | ❌ **없음** — 고정 역할 |
| 권한/RLS | PostgreSQL | PostgreSQL (동일) | PostgreSQL | 풀의 고정 역할 권한 |
| 트랜잭션 | 호출자의 것 | Bolt BEGIN/COMMIT ↔ PG 1:1 | 요청당 암묵 | 요청당 암묵 |
| 감사 | `og_audit` (`session_user`) | `og_audit` (인증 역할) | `og_audit` | `og_audit` (**고정 역할**) |
| 전송 암호화 | PostgreSQL SSL 설정 | ❌ 평문 (양 구간) | 앞단 HTTPS | ❌ 평문 HTTP |
| 스트리밍 | ❌ (전체 수집 후 반환) | ❌ (게이트웨이 메모리에 전체) | ❌ | ❌ |

---

## 2. ① SQL 함수 표면 — 정본(canonical)

**진입점**

| 함수 | 무엇 | 휘발성 |
|---|---|---|
| `og_cypher(graph, query, params)` | Cypher 실행. 행마다 jsonb 객체 하나 | volatile |
| `og_cypher_sql(graph, query)` | 컴파일된 SQL 반환 | `stable` ⚠️ |
| `og_cypher_explain(graph, query, analyze)` | 컬럼 + SQL + 플랜 | volatile |
| `og_cypher_check(query)` | 파싱만. `{ok, clauses, write}` | `immutable, parallel_safe` |
| `og_cypher_columns(query)` | `RETURN` 순서의 컬럼 이름 | `immutable, parallel_safe` |
| `og_cypher_stats()` | 직전 호출이 바꾼 것 (Neo4j 카운터 철자) | `volatile, parallel_unsafe` |
| `og_typeql(graph, query, params)` | TypeQL 실행 | volatile |
| `og_typeql_sql` / `og_typeql_check` / `og_typeql_schema` / `og_typeql_script` | 대응 표면 | — |

전체 78개 `#[pg_extern]` 목록은 [`docs/api.md`](../api.md) 와
[`../00_overview/04_repository_map.md`](../00_overview/04_repository_map.md).

**이 표면이 보장하는 것**
- 호출자 트랜잭션 안에서 실행된다. 롤백하면 그래프 변경도 롤백된다.
- PostgreSQL 역할 권한과 RLS가 그대로 적용된다.
- 결과가 표준 PostgreSQL 타입(`jsonb`)이라 SQL에서 `JOIN` 가능하다.

**⚠️ 휘발성 표기 문제 (Facts)**

`og_cypher_sql` 은 `#[pg_extern(stable)]` 인데
([`cypher/mod.rs:74`](../../engine/src/cypher/mod.rs)),
그 안에서 `views::ensure_view()` 가 `CREATE OR REPLACE VIEW` DDL을 실행할 수 있다
([`cypher/views.rs:135`](../../engine/src/cypher/views.rs)).
`stable` 은 "이 함수는 데이터베이스를 수정하지 않는다"는 계약이다.
→ **ARCH-03**

`og_cypher_stats()` 가 `parallel_unsafe, volatile` 인 것은 올바르다 —
직전 호출이 남긴 백엔드 로컬 상태를 읽으며, **같은 연결에서 다음 질의 전에** 물어야만
의미가 있다 ([`cypher/mod.rs:111-119`](../../engine/src/cypher/mod.rs)).

---

## 3. ② Bolt 4.4 게이트웨이

### 형태

```
Neo4j 드라이버 ──bolt://──> ontological-bolt ──postgres(NoTls)──> PostgreSQL
                            (연결당 스레드)      (세션당 연결 1개)
```

**게이트웨이는 상태를 갖지 않는다**: 파서 없음, 플래너 없음, 캐시 없음, 사용자 저장소 없음
([`bolt/src/main.rs:4-5`](../../bolt/src/main.rs)).

### Neo4j 개념 ↔ 여기의 대응

| Neo4j | 여기 | 근거 |
|---|---|---|
| database (`session(database="x")`) | **graph** `x` | [`session.rs:221-238`](../../bolt/src/session.rs) |
| `HELLO` 의 user/password | **PostgreSQL 역할과 비밀번호**. 두 번째 사용자 저장소 없음 | [`session.rs:168-184`](../../bolt/src/session.rs) |
| `Node` | `og_cypher()` 의 `{_id, _type, …}` 를 Bolt Node로 재인코딩 | [`session.rs:487-545`](../../bolt/src/session.rs) |
| `Relationship` | `{_id, _type, _src, _dst, …}` | 동일 |
| 명시적 트랜잭션 | 세션 연결의 PostgreSQL 트랜잭션 | [`session.rs:208-219`](../../bolt/src/session.rs) |
| `Neo4jError` | 컴파일러 메시지 그대로, `Neo.ClientError.*` 코드 아래 | [`session.rs:571-604`](../../bolt/src/session.rs) |

**`neo4j` 와 `system` 은 그래프 이름이 아니다.** 드라이버가 애플리케이션이
아무것도 지정하지 않았을 때 보내는 값이므로 "기본값"으로 해석한다
([`session.rs:224-229`](../../bolt/src/session.rs)).

**명시적 트랜잭션 안에서는 `db` 가 고정된다.** 드라이버가 `BEGIN` 에 한 번만 이름을 보내고
이후 `RUN` 에서는 생략하므로, `db` 부재는 "기본값"이 아니라 "이 트랜잭션이 시작한 그것"이다
([`session.rs:231-237`](../../bolt/src/session.rs)).

### 게이트웨이가 질의를 보는 유일한 지점

`EXPLAIN` / `PROFILE` 접두사 처리다. 그리고 그때도 파싱하지 않는다 —
`og_cypher_check()` 에 읽기/쓰기를 물어본다.
그래서 클라이언트가 행동을 결정하는 read/write 판정이 **spec 003의 것**이며,
두 번째 전송으로 도달할 뿐 재구현되지 않는다
([`session.rs:256-266`](../../bolt/src/session.rs)).

`EXPLAIN` 은 질의를 **분류만 하고 실행하지 않는다.** 플랜은 없고, 요약의 `type` 이 `r`/`w` 다.

### 필드 순서

`og_cypher()` 는 jsonb 객체를 돌려주고 jsonb는 키를 정렬하므로,
행에서 `RETURN` 순서를 복원할 수 없다. 그래서 `og_cypher_columns(query)` 에 물어본다
([`session.rs:281-289`](../../bolt/src/session.rs)).
`RETURN *` 처럼 파서가 순서를 모르는 경우는 빈 리스트가 오고,
게이트웨이가 첫 행의 키 순서로 폴백한다 — **순서를 지어내지 않는다**.

### 이 진입면의 실측 한계 (Facts)

| 항목 | 상태 | 근거 |
|---|---|---|
| Bolt 3.x / 5.x | 미지원, 협상이 깨끗하게 실패 | [`bolt/README.md`](../../bolt/README.md) |
| `Path` 구조체 | 인코딩하지 않음. 경로 변수는 홉 리스트로 도착 | 동일 |
| 시공간/공간 타입 | 파라미터에 담기면 **거부**한다 (조용히 뭉개지 않음) | 동일 |
| TLS | 종단하지 않음. 앞단 프록시 전제 | 동일 |
| PG 연결 암호화 | `cfg.connect(NoTls)` | [`session.rs:182`](../../bolt/src/session.rs) |
| 라우팅 | `neo4j://` 가 붙게만 함. 단일 서버 응답 | [`session.rs:387`](../../bolt/src/session.rs) 주석 |
| 스트리밍 | ❌ `pg.query()` 로 전체를 모은 뒤 `PULL n` 을 그 `Vec` 에서 서빙 | [`session.rs:291-322`](../../bolt/src/session.rs) |
| 왕복 | `RUN` 하나당 PG 문장 2개 (`og_cypher_columns` + `og_cypher`) | [`session.rs:283-299`](../../bolt/src/session.rs) |
| 연결 풀 | 없음. 세션 1개 = PG 연결 1개 | [`main.rs:69-79`](../../bolt/src/main.rs) |
| `bookmark` | 항상 `"ontological:0"` 상수 | [`session.rs:218`](../../bolt/src/session.rs) |

→ **ARCH-07**

### 검증 방식

`tests/neo4j-movies/run.py` 가 Neo4j **공식 드라이버**로 Neo4j **공식 Movie 샘플**을
돌려 PostgreSQL 경로 / Bolt 경로 / 실제 Neo4j 세 곳의 답을 대조한다.
"우리가 만든 클라이언트로 우리 서버를 테스트하는 것은 증거가 아니다"
([`specs/011-bolt-protocol-gateway/plan.md`](../../specs/011-bolt-protocol-gateway/plan.md)).

Neo4j 공식 MCP 서버 `mcp-neo4j-cypher` 가 무수정으로 동작한다
([`examples/meeting-rooms/`](../../examples/meeting-rooms/)).

---

## 4. ③ PostgREST / Supabase RPC

```rust
#[pg_extern]
fn og_cypher_json(graph: &str, query: &str, params: default!(JsonB, "'{}'")) -> JsonB {
    // SELECT coalesce(jsonb_agg(r), '[]'::jsonb) FROM og_cypher($1,$2,$3) r
}
```
— [`engine/src/interop/mod.rs:34-54`](../../engine/src/interop/mod.rs)

**한 번 호출, JSON 배열 하나.** PostgREST가 이 함수를 RPC로 노출하면
supabase-js가 그대로 부를 수 있다.

### RLS가 순회 중간에 적용된다

이 진입면의 핵심 보장이다:

```rust
//! The load-bearing observation: a compiled Cypher query reads ordinary tables.
//! So row-level security applied to those tables applies **mid-traversal**, with
//! no enforcement code of our own — a node the caller cannot see simply fails to
//! join, and every path through it disappears (FR-013).
```
— [`interop/mod.rs:3-7`](../../engine/src/interop/mod.rs)

`og_enable_rls(graph, type_name, policy_expr)` 가 타입과 **모든 서브타입**의
스토리지 테이블에 정책을 건다 ([`interop/mod.rs:18-32`](../../engine/src/interop/mod.rs)).

> ⚠️ `policy_expr` 은 SQL 불리언 표현식으로 **그대로 보간된다**
> (`CREATE POLICY og_policy ON {table} USING ({policy_expr})`).
> 이것은 DDL 인자이고 호출자가 이미 DDL 권한을 가진 상황이지만,
> 사용자 입력을 여기 넘기면 안 된다. `07_security/` 담당이 다룰 항목이다.

### 관계형 노출

| 방향 | 수단 |
|---|---|
| 그래프 → SQL | `og_node_view`, `og_edge_view`, `og_type_view`, `og_property_view`, `og_role_view` ([`access.sql:81-126`](../../engine/sql/access.sql)) |
| SQL → 그래프 | `og_map_table()` — 기존 테이블을 복사 없이 노드 타입으로 노출 |
| TypeQL 매핑 노출 | `og_typeql_attribute`, `og_typeql_role` ([`access.sql:297-338`](../../engine/sql/access.sql)) |

---

## 5. ④ Studio HTTP

```
브라우저 ──HTTP:7474──> portal/server/index.js ──pg Pool(max=8)──> PostgreSQL
```

### 엔드포인트

| 메서드 · 경로 | 무엇 |
|---|---|
| `GET /api/benchmark` | `bench/results/` JSON |
| `GET /api/health` | 상태 |
| `GET /api/schema` | `og_schema()` 프록시 |
| `GET /api/audit` | `og_audit` 조회 |
| `POST /api/cypher` | `og_cypher()` 실행 + 그래프 투영 |
| `POST /api/explain` | `og_cypher_explain()` |
| `POST /api/diagnose` | `og_diagnose_empty()` |
| `POST /api/expand` | 노드 확장 |
| `POST /api/sql` | **임의 SQL 실행** |

— [`portal/server/index.js:140-309`](../../portal/server/index.js)

### ⚠️ 이 진입면의 보안 모델 (Facts)

```js
const pool = new Pool({
  host: process.env.PGHOST || 'localhost',
  user: process.env.PGUSER || 'dev',
  password: process.env.PGPASSWORD || undefined,
  max: 8,
});
```
— [`portal/server/index.js:21-29`](../../portal/server/index.js)

- **HTTP 인증 계층이 없다.** 요청자 신원 확인 코드가 없다.
- **모든 요청이 하나의 고정 역할로 실행된다.** 따라서 RLS가 요청자별로 적용되지 않고,
  `og_audit.principal` 도 그 고정 역할로 기록된다.
- **`POST /api/sql` 이 임의 SQL을 그 역할로 실행한다** (`pool.query(sql)`,
  [`index.js:296-308`](../../portal/server/index.js)).
- **평문 HTTP** (`http.createServer`).

이것은 **로컬 개발 콘솔의 위협 모델**이다.
`README.md` 의 실행 예시도 `PGHOST=127.0.0.1` 로컬을 전제한다.
→ 네트워크에 노출하면 안 된다. `07_security/` 와 `08_operations/` 담당이
배포 가드를 정의해야 한다.

> 다만 이 서버의 설계 의도는 명시되어 있다:
> *"Deliberately thin: … All the intelligence lives in the database, which is
> the point — anything this server can do, a psql session can do too."*
> ([`index.js:2-9`](../../portal/server/index.js))
> 즉 **추가 권한을 만들지 않았다**는 것이 방어 논리이고, 그 논리는
> 고정 역할이 최소 권한일 때만 성립한다.

---

## 6. 진입면 공통 불변식

모든 진입면이 지켜야 하는 것:

| 불변식 | 왜 |
|---|---|
| Cypher/TypeQL은 **한 곳에서만** 해석된다 (spec 003 / 010) | 두 번째 구현은 두 번째 의미론이 된다 |
| 인증은 **PostgreSQL 역할** 하나뿐이다 | 두 번째 사용자 저장소는 권한·RLS·감사를 분리시킨다 |
| 트랜잭션은 **PostgreSQL 트랜잭션**에 1:1 대응한다 | 헌법 원칙 IX |
| 오류 메시지는 **엔진의 것**을 그대로 전달한다 | 헌법 원칙 VIII (교정 가능한 오류) |
| 감사는 **`og_audit` 하나**에 남는다 | 감사 경로가 갈라지면 감사가 아니다 |

### 현재 위반 (Facts)

| 진입면 | 위반 | 심각도 |
|---|---|---|
| Studio | 인증이 PostgreSQL 역할이 아니라 "없음". 감사가 고정 역할로 기록됨 | **높음** (네트워크 노출 시) |
| Bolt | 인증은 지켜짐. 전송 암호화가 두 구간 모두 없음 | 중 (배포 가이드로 완화) |

---

## Decisions

| # | 결정 | 근거 |
|---|---|---|
| D-1 | Bolt는 별도 프로세스 | SPI가 스레드 안전하지 않아 배경 워커는 세션을 직렬화 (spec 011 plan 결정 1) |
| D-2 | Bolt는 자체 인증 저장소를 만들지 않는다 | 권한·RLS·감사가 하나로 유지 (spec 011 plan 결정 2) |
| D-3 | 게이트웨이는 필드 이름을 파서에서 가져온다 | jsonb가 키를 정렬해 행에서 복원 불가 (결정 3) |
| D-4 | 연결 하나에 스레드 하나 | 동시성 한계를 PostgreSQL 연결 수 한계와 같게 (결정 4) |
| D-5 | Bolt 5.x를 조용히 절반만 구현하지 않는다 | 지원 매트릭스에 "미지원"으로 명시 (FR-020) |
| D-6 | TLS는 앞단에 맡긴다 | 문서화된 한계 (spec 011 Complexity Tracking) |
| D-7 | Studio는 의도적으로 얇다 | 이 서버가 할 수 있는 것은 psql 세션도 할 수 있다 |

## Facts

- 네 진입면 중 **필수는 SQL 함수 하나**다. 나머지를 다 꺼도 데이터베이스는 완전하다.
- `ontological-bolt` 를 멈춰도 `og_cypher()` 호출자는 영향받지 않는다.
- Bolt 세션 하나가 PostgreSQL 연결 하나를 점유한다 → Bolt 동시 세션 상한 = `max_connections`.
- Studio 풀은 `max: 8` 이다.

---

## Forbidden / Required

**Forbidden**
- ❌ 어떤 진입면에도 Cypher/TypeQL 파서·플래너·캐시를 추가하는 것.
- ❌ 두 번째 사용자/비밀번호 저장소를 만드는 것.
- ❌ 진입면에서 오류 메시지를 재작성하는 것 — 엔진의 교정 제안이 소실된다.
- ❌ 진입면별로 별도 감사 로그를 만드는 것.
- ❌ Studio를 네트워크에 노출하는 것 (현재 인증 계층 없음).
- ❌ `og_enable_rls` 의 `policy_expr` 에 사용자 입력을 전달하는 것.

**Required**
- ✅ 새 진입면을 추가하면 위 "진입면 공통 불변식" 표의 5개 항목을 모두 만족시킬 것.
- ✅ 지원/미지원을 **매트릭스**로 문서화할 것. "지원함"이라는 모호한 표현 금지 (헌법 기술 제약).
- ✅ 진입면의 지원 범위가 코어보다 좁아질 수는 있어도 **넓어져서는 안 된다**.
- ✅ 새 SQL 노출 API에는 RLS/권한 테스트를 함께 추가할 것 (헌법 품질 게이트).

<!-- affects: architecture, api, security, operations, frontend -->
<!-- requires-update: 02_api/, 07_security/, 08_operations/, 04_frontend/ -->
