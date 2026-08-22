# 08. 가드레일 — `og_create_role` / `og_apply_role` / `og_add_rule`

> **이 문서가 답하는 질문**
> - 에이전트 역할에 걸 수 있는 상한은 무엇이고, 그중 **실제로 강제되는 것은 무엇인가**?
> - `og_add_rule` 은 가드레일인가? (아니라면 무엇인가)
> - 감사 로그는 무엇을 기록하고, 무엇을 기록하지 못하는가?
> - 에이전트에게 이 DB를 열어줄 때의 최소 안전 구성은?

---

## 1. 사실 — `og_create_role(name, limits jsonb)`

정의: [engine/src/agent/mod.rs:404-412](../../engine/src/agent/mod.rs).

`og_catalog.agent_role` 에 upsert할 뿐이다
([engine/sql/bootstrap.sql:372-375](../../engine/sql/bootstrap.sql)):

```sql
CREATE TABLE og_catalog.agent_role (
    name   text PRIMARY KEY,
    limits jsonb NOT NULL DEFAULT '{}'::jsonb
);
```

**`limits` 의 내용은 검증되지 않는다.** 오타 난 키(`statment_timeout_ms`)도 그대로
저장되고, `og_apply_role` 이 조용히 무시한다.

이것은 PostgreSQL의 `ROLE` 이 아니다. 이름이 같을 뿐 별개의 개념이며, DB 역할·권한과
연결되어 있지 않다.

---

## 2. 사실 — `og_apply_role(name)` 이 실제로 하는 일

정의: [engine/src/agent/mod.rs:415-441](../../engine/src/agent/mod.rs).

```rust
if let Some(ms)   = limits["statement_timeout_ms"].as_i64() { Spi::run(&format!("SET statement_timeout = {ms}")).ok(); }
if let Some(mem)  = limits["work_mem_kb"].as_i64()          { Spi::run(&format!("SET work_mem = {mem}")).ok(); }
if let Some(ro)   = limits["read_only"].as_bool() { if ro { Spi::run("SET default_transaction_read_only = on").ok(); } }
if let Some(rows) = limits["max_rows"].as_i64()             { Spi::run(&format!("SET og.max_rows = {rows}")).ok(); }
```

| 키 | 타입 | 매핑 | **실제로 강제되는가** |
|---|---|---|---|
| `statement_timeout_ms` | 정수 | `SET statement_timeout` | ✅ PostgreSQL이 강제 |
| `work_mem_kb` | 정수 | `SET work_mem` | ✅ PostgreSQL이 강제 (단, 단위 없이 숫자만 → PostgreSQL은 이를 **kB**로 해석) |
| `read_only` | 불리언 | `SET default_transaction_read_only = on` | ⚠️ 부분 — 3.2절 |
| `max_rows` | 정수 | `SET og.max_rows` | ❌ **강제되지 않음** — 3.1절 |

반환값은 `{"role": name, "applied": limits}` 다
([agent/mod.rs:440](../../engine/src/agent/mod.rs)). 이는 "적용을 시도했다"는 뜻이지
"강제된다"는 뜻이 아니다 — 네 `Spi::run` 이 모두 `.ok()` 로 결과를 버린다.

복사-붙여넣기 가능한 예:

```sql
SELECT og_create_role('analyst', '{
  "statement_timeout_ms": 5000,
  "work_mem_kb": 65536,
  "read_only": true,
  "max_rows": 10000
}'::jsonb);

SELECT og_apply_role('analyst');   -- 세션마다 호출해야 한다
```

**세션 스코프**다. 커넥션 풀을 쓰면 커넥션을 빌릴 때마다 다시 호출해야 한다.
`og_apply_role` 을 자동 호출하는 코드는 저장소에 없다 —
Bolt 게이트웨이([bolt/src/session.rs](../../bolt/src/session.rs))도, Studio
백엔드([portal/server/index.js](../../portal/server/index.js))도 호출하지 않는다.

---

## 3. 사실 — 강제되지 않는 상한들

### 3.1 `og.max_rows` 는 어디에서도 읽히지 않는다

`SET og.max_rows = {rows}` 는 사용자 정의 GUC를 설정한다
([agent/mod.rs:437-439](../../engine/src/agent/mod.rs)). 저장소 전체에서 이 GUC를
**읽는 코드는 없다**(전수 grep 결과: `agent/mod.rs:437-438` 과 `docs/agents.md:133`
두 곳이 전부. `docs/deep-traversal.md` 의 `max_rows` 는 pgGraph 벤치마크 파라미터로
무관하다 — [bench/harness.py:603, 679](../../bench/harness.py)).

즉 **결과 행 수 상한은 존재하지 않는다.** spec 008 FR-024가 요구하는 "방문 노드/엣지
수, 결과 행 수" 상한
([specs/008-agent-native-interface/spec.md:260-261](../../specs/008-agent-native-interface/spec.md))
중 실제로 걸리는 것은 시간(`statement_timeout`)과 메모리(`work_mem`)뿐이다.

방문 노드 수 상한을 컴파일된 SQL에 주입하는 T011은 미착수다
([specs/008-agent-native-interface/tasks.md:22](../../specs/008-agent-native-interface/tasks.md)).

### 3.2 `read_only` 의 범위

`SET default_transaction_read_only = on` 은 **다음 트랜잭션부터**의 기본값이다.
같은 세션에서 명시적으로 `SET TRANSACTION READ WRITE` 또는
`SET default_transaction_read_only = off` 를 실행하면 해제된다. 즉 **평문 SQL을 실행할
수 있는 주체에게는 우회 가능한 장벽**이다.

Cypher만 보낼 수 있는 경로(Bolt)에서는 우회 수단이 없으므로 유효하다.
`og_cypher` 는 쓰기 절을 만나면 `run_write` 로 진입하고
([engine/src/cypher/mod.rs:101-105](../../engine/src/cypher/mod.rs)), read-only 트랜잭션에서
PostgreSQL이 INSERT/UPDATE를 거부한다.

### 3.3 대량 쓰기·삭제의 사전 확인 (FR-026)

"영향 행 수가 임계치를 넘으면 dry-run 없이는 거부"
([spec.md:263](../../specs/008-agent-native-interface/spec.md))를 구현하는 코드는 없다.
`og_estimate` 는 쓰기 질의를 컴파일조차 하지 못한다
([04_dry_run_and_estimate.md](04_dry_run_and_estimate.md) 6절).

### 3.4 반복 실패 질의 속도 제한 (FR-029)

미착수 (T012, [tasks.md:23](../../specs/008-agent-native-interface/tasks.md)).
에이전트의 무한 재시도 루프를 DB가 막지 않는다.

### 3.5 함수 실행 권한

`engine/sql/bootstrap.sql` 과 `engine/sql/access.sql` 에 `GRANT` / `REVOKE` 구문이
**하나도 없다**(전수 grep 0건). pgrx의 `#[pg_extern]` 은 `CREATE FUNCTION` 을 생성하며,
PostgreSQL은 함수의 `EXECUTE` 를 기본적으로 `PUBLIC` 에 부여한다.

따라서 확장이 설치된 DB에 연결할 수 있는 역할은 원칙적으로 `og_set_setting`,
`og_create_role`, `og_drop_type`, `og_genai_encode` 를 포함한 **모든 `og_*` 함수를
호출할 수 있다.** 실제 접근 가능 여부는 설치 환경의 스키마 권한 설정에 달려 있으며
이 저장소만으로는 확정할 수 없다(환경 의존).

---

## 4. 사실 — 감사 로그 `og_data.og_audit`

### 4.1 스키마

[engine/sql/bootstrap.sql:380-390](../../engine/sql/bootstrap.sql):

```sql
CREATE TABLE og_data.og_audit (
    audit_id    bigserial PRIMARY KEY,
    principal   text        NOT NULL DEFAULT session_user,
    at          timestamptz NOT NULL DEFAULT now(),
    query       text,
    lang        text,                 -- 'cypher' | 'sparql' | 'sql'
    rows_out    int8,
    duration_ms double precision,
    error_code  text
);
CREATE INDEX og_audit_at_idx ON og_data.og_audit (at DESC);
```

### 4.2 기록 지점 — 두 곳뿐

| 함수 | 기록 | 근거 |
|---|---|---|
| `og_cypher` | ✅ 성공/파싱실패 | [engine/src/cypher/mod.rs:96, 107, 122-135](../../engine/src/cypher/mod.rs) |
| `og_typeql` | ✅ 성공/실패 | [engine/src/typeql/mod.rs:59, 63, 115-128](../../engine/src/typeql/mod.rs) |
| `og_cypher_json` | ✅ (내부에서 `og_cypher` 호출) | [engine/src/interop/mod.rs:36-44](../../engine/src/interop/mod.rs) |
| `og_typeql_script` | ❌ **기록 안 됨** | [engine/src/typeql/mod.rs:100-113](../../engine/src/typeql/mod.rs) — `audit` 호출 없음 |
| `og_vector_search` / `og_similar` / `og_hybrid_search` / `og_vector_search_exact` | ❌ | [engine/src/vector/mod.rs](../../engine/src/vector/mod.rs) 전체에 `og_audit` 없음 |
| `og_schema` / `og_schema_for` / `og_estimate` / `og_explain_error` / `og_diagnose_empty` | ❌ | [engine/src/agent/mod.rs](../../engine/src/agent/mod.rs) 전체에 `og_audit` 없음 |
| `og_genai_encode` (외부 HTTP 호출) | ❌ | [engine/src/compat/genai.rs](../../engine/src/compat/genai.rs) 전체에 `og_audit` 없음 |
| DDL 계열 (`og_create_type`, `og_add_embedding`, `og_set_setting` …) | ❌ | — |

Cypher를 통한 벡터 검색(`CALL db.index.vector.queryNodes …`)은 `og_cypher` 를 거치므로
기록된다. **평문 SQL로 `og_vector_search` 를 직접 부르면 기록되지 않는다.**

spec 008 SC-008("모든 질의 실행의 **100%** 가 감사 로그에 기록된다",
[spec.md:311](../../specs/008-agent-native-interface/spec.md))은 충족되지 않는다.

### 4.3 실패한 질의는 남지 않는다

```rust
// engine/src/cypher/mod.rs:93-99
let ast = match parser::parse(query) {
    Ok(a) => a,
    Err(e) => {
        audit(graph, query, 0, started, Some(&e));   // INSERT
        error!("cypher parse error: {e}")            // ← 트랜잭션 중단
    }
};
```

`audit()` 은 `Spi::run_with_args` 로 INSERT를 수행하지만
([cypher/mod.rs:122-134](../../engine/src/cypher/mod.rs)), 바로 다음 줄의 `error!` 가
트랜잭션을 중단시킨다. INSERT는 **롤백된다.** 서브트랜잭션이나 자율 트랜잭션을 쓰는
코드는 없다.

컴파일/실행 실패도 같다 — `run_read` 안의 `error!`
([cypher/mod.rs:140, 149](../../engine/src/cypher/mod.rs))가 발생하면 107행의 `audit` 에
도달하지 못한다.

**결과: `error_code` 컬럼은 사실상 항상 NULL이고, 실패한 에이전트 질의는 감사 로그에
남지 않는다.** 이는 원문 [docs/agents.md:138-140](../../docs/agents.md) 및
[docs/api.md:184-185](../../docs/api.md) 의 서술("records every `og_cypher` call with …
error code")과 어긋난다.

### 4.4 파라미터가 기록되지 않는다

```rust
// engine/src/cypher/mod.rs:128
format!("[{graph}] {query}").into(),
```

기록되는 것은 `[graph] query text` 이고, `params jsonb` 는 기록되지 않는다.
설계상 사용자 값은 전부 파라미터로 들어가므로
([engine/src/cypher/compile.rs:1156-1157](../../engine/src/cypher/compile.rs)),
**감사 로그만으로는 에이전트가 실제로 무엇을 조회했는지 재구성할 수 없다.**

역설적으로 이것이 FR-028(민감정보 마스킹)의 부분적 대체 효과를 낸다 — 값이 애초에
기록되지 않으므로. 그러나 마스킹 정책을 설정할 수단은 없고, 질의 텍스트 안에
리터럴로 박힌 값은 그대로 기록된다.

### 4.5 조회

```sql
-- 최근 100건 (Studio가 쓰는 질의와 동일: portal/server/index.js:285-288)
SELECT audit_id, principal, at, query, lang, rows_out, duration_ms, error_code
  FROM og_data.og_audit ORDER BY at DESC LIMIT 100;
```

`og_audit` 은 `pg_extension_config_dump` 대상이므로 백업에 포함된다
([engine/sql/bootstrap.sql:431](../../engine/sql/bootstrap.sql)). **보존 정책·회전·삭제
함수는 없다.**

---

## 5. 사실 — `og_add_rule` 은 가드레일이 아니다

정의: [engine/src/catalog/types.rs:656-682](../../engine/src/catalog/types.rs).
스펙 소속은 **002 FR-027**(온톨로지 타입 시스템)이지 008이 아니다.

```sql
SELECT og_add_rule('kb', 'PART_OF', 'transitive');
SELECT og_add_rule('kb', 'MARRIED_TO', 'symmetric');
SELECT og_add_rule('kb', 'PARENT_OF', 'inverse', 'CHILD_OF');
```

- 허용 값: `transitive` | `symmetric` | `reflexive` | `inverse`
  ([types.rs:667](../../engine/src/catalog/types.rs)). 그 외는 거부.
- `inverse` 는 `target_type` 이 필수 ([types.rs:670-672](../../engine/src/catalog/types.rs)).
- 저장 위치: `og_catalog.rule`
  ([engine/sql/bootstrap.sql:146-156](../../engine/sql/bootstrap.sql)).
- RDF/OWL 임포트도 같은 테이블에 쓴다
  ([engine/src/adapters/rdf.rs:428-435](../../engine/src/adapters/rdf.rs)).

**중요한 사실: `og_catalog.rule` 을 읽는 코드가 저장소에 없다.**
전수 검색 결과 이 테이블에 대한 접근은 INSERT 두 곳
([types.rs:675](../../engine/src/catalog/types.rs),
[rdf.rs:434](../../engine/src/adapters/rdf.rs))과 백업 등록
([bootstrap.sql:410, 443](../../engine/sql/bootstrap.sql))뿐이다. `SELECT` 는 없다.
시드된 설정 키 `inference_max_depth = '16'`
([bootstrap.sql:259](../../engine/sql/bootstrap.sql))도 읽는 코드가 없다.

즉 **선언은 저장되지만 추론은 수행되지 않는다.** `PART_OF` 를 transitive로 선언해도
`MATCH (a)-[:PART_OF]->(b)` 가 전이 폐포를 반환하지 않는다. RDF 왕복
(`og_dump_rdf`)을 위한 메타데이터 보존이 현재의 실질적 용도다.

**에이전트 경계 관점의 의미**: 규칙은 에이전트가 볼 수 있는 데이터를 넓히지도 좁히지도
않는다. 가드레일 문서에서 다루는 이유는 "이 함수가 가드레일이 아니다"를 명시하기
위해서다.

---

## 6. 사실 — 실제로 쓸 수 있는 경계 장치

에이전트 격리에 실제로 기여하는 것은 **PostgreSQL 본체의 기능**이다.

| 장치 | 함수/방법 | 근거 |
|---|---|---|
| 실행 시간 상한 | `og_apply_role` 의 `statement_timeout_ms` | [agent/mod.rs:426-428](../../engine/src/agent/mod.rs) |
| 메모리 상한 | `og_apply_role` 의 `work_mem_kb` | [agent/mod.rs:429-431](../../engine/src/agent/mod.rs) |
| 읽기 전용 | `og_apply_role` 의 `read_only` (Bolt 경로에서 유효) | [agent/mod.rs:432-436](../../engine/src/agent/mod.rs) |
| 행 수준 보안 | `og_enable_rls(graph, type, policy_expr)` | [engine/src/interop/mod.rs:19-32](../../engine/src/interop/mod.rs) |
| 함수 실행 권한 | PostgreSQL `REVOKE` — **직접 걸어야 한다** | 3.5절 |
| 감사 | `og_data.og_audit` — 4절의 한계 인지 필요 | |

### 6.1 최소 안전 구성 (권장, 복사-붙여넣기 가능)

```sql
-- 1. 에이전트 전용 PostgreSQL 역할
CREATE ROLE agent_ro LOGIN PASSWORD '…';

-- 2. 위험한 함수의 PUBLIC 실행 권한 회수
REVOKE EXECUTE ON FUNCTION og_set_setting(text, text)            FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_genai_encode(text, text, jsonb)    FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_create_role(text, jsonb)           FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_enable_rls(text, text, text)       FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_vector_search(text,text,text,text,int4,text) FROM PUBLIC;  -- filter 인젝션 (06 문서 1.3절)
-- 설정 테이블 자체도 차단
REVOKE ALL ON og_catalog.setting FROM PUBLIC;

-- 3. 에이전트 역할 상한
SELECT og_create_role('agent', '{
  "statement_timeout_ms": 5000,
  "work_mem_kb": 65536,
  "read_only": true
}'::jsonb);

-- 4. 세션 진입 시마다
SELECT og_apply_role('agent');
```

**주의**: 위 `REVOKE` 문의 시그니처는 실제 설치본에서 `\df og_*` 로 확인해야 한다.
이 저장소에 위 `REVOKE` 를 수행하는 코드는 없다 — 운영자가 직접 해야 한다.

행 수 상한이 필요하면 애플리케이션 쪽에서 `LIMIT` 를 강제하거나, Bolt 게이트웨이
앞단에서 잘라야 한다. DB는 하지 않는다 (3.1절).

---

## 7. 필수(Required) / 금지(Forbidden)

**필수**

- 모든 에이전트 세션에서 `og_apply_role` 을 호출할 것. 자동 호출 지점이 없다 (2절).
- `statement_timeout_ms` 를 **반드시** 설정할 것. 저장소에서 실제로 강제되는 유일한
  폭주 방지 장치다.
- 위험 함수의 `EXECUTE` 를 `PUBLIC` 에서 회수할 것 (6.1절).
- 재시도 상한과 속도 제한은 **에이전트 쪽에서** 구현할 것 (3.4절).
- 감사가 규제 요건이라면 별도 계층(프록시, `log_statement`, `pgaudit`)을 둘 것.
  `og_data.og_audit` 만으로는 실패 질의도, 파라미터도, 벡터 검색도 남지 않는다 (4절).

**금지**

- `og_apply_role` 의 반환값 `{"applied": …}` 를 강제 성공으로 해석 금지. 네 `SET` 이
  모두 `.ok()` 로 결과를 버린다 (2절).
- `max_rows` 로 결과 크기가 제한된다고 가정 금지 (3.1절).
- `read_only: true` 를 평문 SQL 실행 권한이 있는 주체에 대한 보증으로 쓰지 말 것 (3.2절).
- `og_add_rule` 로 선언한 특성이 질의 결과를 바꾼다고 가정 금지 (5절).
- 에이전트가 `og_data.og_audit` / `og_catalog.setting` / `og_catalog.agent_role` 에
  쓰기 권한을 갖게 하지 말 것.

---

## 8. 참고

- 원문: [docs/agents.md:126-140](../../docs/agents.md) "Constrain what it can do"
- 함수 계약: [docs/api.md:178, 184-185](../../docs/api.md)
- 스펙: FR-024~FR-029, SC-007/SC-008
  ([specs/008-agent-native-interface/spec.md:258-267, 309-311](../../specs/008-agent-native-interface/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-15, LLM-17

<!-- affects: llm, security, backend, ops -->
<!-- requires-update: 02_api/00_index.md, 03_backend/00_index.md -->
