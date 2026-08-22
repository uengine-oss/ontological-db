# 문제 해결

> **이 문서가 답하는 질문**
> - 이 오류 메시지는 무엇을 뜻하는가?
> - 증상별로 어디부터 확인해야 하는가?
> - 무엇이 실제 오류이고 무엇이 "조용한 실패"인가?

---

## 이 문서의 규칙

- **여기 인용된 오류 문자열은 전부 저장소 코드에서 확인된 것이다.** 파일:라인이 붙어 있다.
- 코드에 없는 오류는 이 문서에 없다. 처음 보는 메시지를 만나면
  `grep -rn "<메시지 조각>" engine/src` 로 출처를 먼저 찾을 것.
- **조용한 실패**를 별도로 표기했다. 이 시스템에서 가장 잡기 어려운 부류다.

---

## 0. 진단 1단계 — 어느 층인가

```bash
# 컨테이너
docker ps --filter name=ontological-dev

# PostgreSQL
docker exec ontological-dev bash -lc 'pg_isready -h localhost -p 28816'

# 확장
psql -h localhost -p 28816 -d og -tAc "SELECT ontological_version()"

# Studio
curl -s http://localhost:7474/api/health

# Bolt
docker exec ontological-dev bash -lc 'pgrep -af ontological-bolt'
docker exec ontological-dev bash -lc 'tail -20 /tmp/ontological-bolt.log'

# Studio 로그 (호스트)
tail -50 /tmp/ontological-studio.log
```

로그 위치를 다시 정리하면 — Bolt 로그는 **컨테이너 안**, Studio 로그는 **호스트**다
([02_startup_and_teardown.md](02_startup_and_teardown.md) 참조).

---

## 1. 설치 · 기동

| 증상 | 원인 | 조치 |
|---|---|---|
| `ERROR: extension "ontological" is not available` | `.so`/`.control`이 설치되지 않음. `start.sh`에는 `cargo pgrx install`이 없다 (`start.sh` 전체에 부재) | `cargo pgrx install --features pg16 --no-default-features --pg-config /usr/lib/postgresql/16/bin/pg_config --sudo` — [01_install.md](01_install.md) A-2 |
| `ERROR: required extension "vector" is not installed` | `CASCADE` 없이 설치 시도. `requires = 'vector'` (`engine/ontological.control:7`) | `CREATE EXTENSION ontological CASCADE` |
| **`./start.sh`는 성공했는데 그래프가 비어 있다** | **조용한 실패.** `start.sh:45-52`가 "DB 존재" 하나만 보고, 확장 생성과 `demo.sql` 적재 오류를 `>/dev/null 2>&1`로 버린다. DB는 생겼으므로 재실행 시 이 블록을 건너뛴다 | 확장을 설치한 뒤 DB를 다시 만든다 — [02_startup_and_teardown.md](02_startup_and_teardown.md) "데이터만 초기화" |
| `pg_isready`가 계속 실패 | `cargo pgrx start pg16` 실패. `start.sh:37`이 `|| true`로 삼킨다 | 수동 실행해 오류를 본다: `docker exec ontological-dev bash -lc 'cd /work/engine && cargo pgrx start pg16'` |
| `cargo pgrx start` 가 root 거부 | pgrx는 root로 postgres를 띄우지 않는다 (`docker/Dockerfile.dev:12` 주석) | 컨테이너의 `dev` 사용자로 실행. Dockerfile이 이미 `USER dev`로 끝난다 (`docker/Dockerfile.dev:16`) |
| `cargo pgrx install` 이 권한 오류 | 확장 디렉터리 쓰기 권한 | `--sudo` 를 붙인다. `dev`에게 NOPASSWD sudo가 부여되어 있다 (`docker/Dockerfile.dev:13`) |
| 이미지 재빌드 후 컴파일 실패 | `cargo install cargo-pgrx --locked`에 버전 핀이 없어(`docker/Dockerfile.dev:22`) `engine/Cargo.toml:22`의 `pgrx = "=0.19.2"`와 어긋남 | `cargo install cargo-pgrx --locked --version 0.19.2` — `OPS-04` |
| 코드를 고쳤는데 동작이 그대로 | 열려 있던 백엔드가 옛 `.so`를 잡고 있다 | `cargo pgrx stop pg16 && cargo pgrx start pg16` |

---

## 2. 그래프 · 타입

| 오류 메시지 | 출처 | 뜻 | 조치 |
|---|---|---|---|
| `graph '{name}' does not exist` | `engine/src/catalog/types.rs:118` | 그래프 이름 오타 또는 미생성 | `SELECT name FROM og_catalog.graph` 로 확인. 필요하면 `og_create_graph('<name>')` |
| `type '{name}' does not exist. did you mean: {...}` | `engine/src/catalog/types.rs:135` | 타입 이름 오타. 유사 이름을 제시한다 | 제시된 이름 중에서 고른다 |
| `type '{name}' does not exist in this graph` | `engine/src/catalog/types.rs:133` | 위와 같으나 유사 후보가 없음 | `SELECT name FROM og_type_view WHERE graph = '<g>'` |
| `NOTICE: label '{name}' does not exist in graph '{graph}' — matching nothing. did you mean: {...}` | `engine/src/catalog/types.rs:168-172` | **오류가 아니라 NOTICE.** 없는 라벨을 매치하면 빈 결과가 정상이므로 실패시키지 않고 힌트만 보낸다 (spec 008 FR-008) | 결과가 비었을 때 이 NOTICE를 먼저 볼 것 |
| `type '{name}' already exists in graph '{graph}'` | `engine/src/catalog/types.rs` | 중복 생성 | 기존 타입을 쓰거나 다른 이름 |
| `'{type_name}' is abstract and cannot be instantiated` | `engine/src/catalog/types.rs` | 추상 타입에 인스턴스를 만들려 함 | 구체 서브타입을 쓴다. `SELECT * FROM og_type_view WHERE is_abstract` |
| `type '{name}' ({kind}) cannot inherit from '{p}' of kind '{pk}'` | `engine/src/catalog/types.rs` | entity/relation/attribute 종류가 다른 타입 간 상속 | 같은 kind 안에서 상속 |
| `type '{name}' has {n} instance(s) (including subtypes). pass cascade => true to remove them` | `engine/src/catalog/types.rs` | 인스턴스가 있는 타입 드롭 | 의도했다면 `og_drop_type(..., cascade => true)` |
| `inheritance cycle detected involving type(s) {...}` | `engine/src/catalog/labeling.rs:142` | 타입 계층에 순환 | 순환을 만든 `og_catalog.type_parent` 행을 제거한 뒤 `og_relabel(graph_id)` — [07_maintenance.md](07_maintenance.md) §3 |
| `inheritance cycle detected while walking the type hierarchy` | `engine/src/catalog/labeling.rs:132` | 위와 동일(탐색 가드에 걸림) | 동일 |
| `inheritance cycle detected at type {id}` | `engine/src/catalog/labeling.rs` | 동일 | 동일 |

---

## 3. Cypher

| 오류 메시지 | 출처 | 뜻 | 조치 |
|---|---|---|---|
| `cypher parse error: {msg} at offset {pos}, near: …{snippet}…` | `engine/src/cypher/mod.rs:97`, 형식은 `engine/src/cypher/parser.rs:97-101` | 파싱 실패. `offset`은 문자 위치, `snippet`은 앞뒤 문맥 | `SELECT og_cypher_check('<graph>','<query>')` 또는 `og_explain_error`로 구조화된 진단을 받는다 |
| `expected a clause keyword` / `expected a name` / `expected an expression` / `expected a relationship pattern` / `expected '->' or '-' to close the relationship pattern` | `engine/src/cypher/parser.rs` | 각 위치의 문법 기대 | 메시지의 `offset` 지점을 본다 |
| `unexpected clause '{...}'` | `engine/src/cypher/parser.rs` | 지원하지 않거나 위치가 틀린 절 | 지원 범위는 [`docs/cypher.md`](../cypher.md)가 권위 |
| `cypher error: {e}` | `engine/src/cypher/mod.rs:140,189,195,205,224` | 파싱은 됐고 **컴파일**이 실패 | 컴파일 결과를 본다: `SELECT og_cypher_sql('<graph>','<query>')` |
| `cypher execution failed: {e}` + `--- compiled SQL ---` + SQL 전문 | `engine/src/cypher/mod.rs:149` | 컴파일된 SQL이 PostgreSQL에서 실패. **컴파일 결과가 오류에 그대로 붙어 나온다** | 붙어 나온 SQL을 직접 `EXPLAIN` 해본다 |
| `CREATE requires a label per new node` | `engine/src/cypher/compile.rs` | 여기서는 타입이 정체성의 일부다 (spec 002) | `CREATE (n:Type {...})` |
| `CREATE requires exactly one relationship type` | `engine/src/cypher/mod.rs:430` | 관계에 타입이 없거나 여러 개 | `-[:REL_TYPE]->` 하나만 |
| `cannot delete node {id}: it still has {deg} relationship(s). use DETACH DELETE` | `engine/src/cypher/mod.rs:286-289` | Neo4j와 동일한 규칙 | `DETACH DELETE n` |
| `SET refers to unbound variable '{var}'` / `SET/REMOVE refers to unbound variable '{var}'` | `engine/src/cypher/compile.rs` | `MATCH`로 바인딩되지 않은 변수에 대입 | 변수를 먼저 매치 |
| `a schema command cannot be combined with other clauses` | `engine/src/cypher/mod.rs:163` | 인덱스/제약 DDL을 다른 절과 섞음 | 문장을 분리 |
| `failed to enforce uniqueness on '{label}': {e}` | `engine/src/compat/ddl.rs` | 유니크 제약 생성 실패 — 보통 기존 데이터에 중복이 있음 | 중복을 먼저 제거 |
| `an index named '{name}' already exists in graph '{graph}'` / `a constraint named '{name}' already exists…` | `engine/src/compat/ddl.rs` | 이름 충돌 | 다른 이름 또는 기존 것을 드롭 |
| `no {what} named '{name}' in graph '{graph}'` (`{what}` = `index` \| `constraint`) | `engine/src/compat/ddl.rs:334` | 없는 것을 드롭 | `IF EXISTS`를 붙이거나 이름 확인 |
| `CREATE VECTOR INDEX needs OPTIONS {indexConfig: {`vector.dimensions`: N}}` | `engine/src/compat/ddl.rs` | 벡터 인덱스에 차원이 없음 | `OPTIONS`에 차원을 명시 |
| `a vector index needs one property` | `engine/src/compat/ddl.rs` | 프로퍼티가 0개 또는 2개 이상 | 하나만 |

### 결과가 비어 있는데 오류가 없다

이 시스템은 진단 함수를 별도로 제공한다 (spec 008):

```sql
SELECT jsonb_pretty(og_explain_error('default', $$ ... $$));   -- 파싱/컴파일 오류의 구조화된 설명
SELECT jsonb_pretty(og_diagnose_empty('default', $$ ... $$));  -- 왜 비었는가
SELECT jsonb_pretty(og_estimate('default', $$ ... $$));        -- 예상 카디널리티
SELECT og_cypher_sql('default', $$ ... $$);                    -- 컴파일된 SQL
SELECT jsonb_pretty(og_cypher_explain('default', $$ ... $$, false));  -- 계획
```

Studio는 이 셋을 `POST /api/diagnose` 하나로 묶어 노출한다
(`portal/server/index.js:233-253`).

`og_diagnose_empty` / `og_estimate`는 Studio에서 실패해도 `null`로 대체되어
전체 응답을 막지 않는다 (`portal/server/index.js:238-243`).

### 알려진 미지원

`bolt/README.md:74-75`가 명시한다: `UNION`과 `shortestPath`는
**psql 경로와 Bolt 경로에서 동일하게 실패한다.** 지원 범위의 권위는
[`docs/cypher.md`](../cypher.md)이며, 스펙 상태표는 `README.md`에 있다.

---

## 4. TypeQL

| 오류 메시지 | 출처 | 뜻 |
|---|---|---|
| `typeql parse error: {e}` | `engine/src/typeql/mod.rs` | 파싱 실패 |
| `typeql error: {e}` | `engine/src/typeql/mod.rs` | 컴파일 실패 |
| `typeql error in block {} of {}: {e}` | `engine/src/typeql/mod.rs` | 스크립트의 몇 번째 블록에서 실패했는지 |
| `typeql execution failed: {e}` + `--- compiled SQL ---` | `engine/src/typeql/mod.rs` | 컴파일된 SQL 실행 실패 |
| `only read queries compile to a single SQL statement` | `engine/src/typeql/mod.rs:90` | `og_typeql_sql`은 읽기 질의만 SQL로 보여줄 수 있다 |
| `relation '{rel_type}' has no role named '{role}'` | `engine/src/catalog/types.rs` | 역할 이름 오타 — `SELECT * FROM og_role_view` |
| `role '{name}' of relation '{rel_type}' requires a '{expected}', got '{got}'` | `engine/src/typeql/write.rs` 부근 | 역할 플레이어 타입 불일치 |
| `'{rel_type}' is not a relation type; roles only exist on relations` | `engine/src/catalog/types.rs` | entity에 역할을 붙이려 함 |

spec 010은 partial 상태다. `tests/typeql/run.py`는 미지원 항목을 실패가 아니라
`unsupported`로 표시한다 (`tests/typeql/run.py:39-46`).

---

## 5. 순회 · CSR

| 오류 메시지 | 출처 | 뜻 | 조치 |
|---|---|---|---|
| `no compiled graph in this backend — call og_csr_build() first` | `engine/src/storage/traverse.rs:340` | 이 **백엔드**에 CSR이 없다. 다른 세션에서 빌드했어도 소용없다 | 같은 연결에서 `SELECT * FROM og_csr_build(NULL,'o')` |
| `direction must be 'o', 'i' or 'b', not '{...}'` | `engine/src/storage/traverse.rs:39`, `:92` | 방향 인자가 `'o'`(out) / `'i'`(in) / `'b'`(both) 가 아님 | 세 글자 중 하나 |
| `adjacency scan could not be planned: {e}` | `engine/src/storage/traverse.rs:116` | `og_reach`의 SPI 준비 실패 | 보통 카탈로그 손상. `og_check_integrity()` |
| `adjacency scan failed: {e}` | `engine/src/storage/traverse.rs:251` | CSR 컴파일 중 인접 스캔 실패 | 동일 |

### CSR 결과가 최신 데이터와 다르다

**오류가 아니라 설계다.** CSR 스냅샷은 빌드 시점에 얼어붙는다
(`docs/deep-traversal.md:257-259`, `engine/src/storage/traverse.rs:21-23`).
트리거 캡처가 없다. `og_csr_drop()` 후 다시 `og_csr_build()` 할 것.

### CSR 결과에 보이면 안 될 행이 있다

**역시 설계다.** CSR 경로는 **RLS를 참조하지 않는다**
(`docs/deep-traversal.md:260-261`). RLS가 필요하면 `og_reach`(힙 BFS)를 쓸 것.
Cypher 컴파일러가 CSR로 라우팅하지 않는 이유가 이것이다 (`docs/deep-traversal.md:269-272`).

### 깊은 순회가 끝나지 않는다

가변 길이 매치가 **방문집합 BFS로 재작성되지 못하는 형태**면 trail을 열거한다 —
`degree^k` 행이다. 재작성이 막히는 조건 (`docs/deep-traversal.md:201-215`):

- 경로 변수 바인딩 `MATCH p = ...`
- 관계 변수 바인딩 `-[e:K*1..3]->`
- 중복이 관측 가능한 프로젝션 — `count(x)`, `sum`, `avg`, 사용자 정의 집계
- 질의 어디든 `WITH`가 있음

또한 손익분기(`Σ degreeⁱ > |V|`) 미만이면 재작성하지 않는다. 그 판단은
**플래너 통계**에서 읽으며, 통계가 없으면 깊이만 보고 판단한다
(`docs/deep-traversal.md:216-226`).

조치 순서:

```sql
-- 1. 통계부터
ANALYZE og_data.og_adj;
ANALYZE og_data.og_node;
ANALYZE og_data.og_edge;

-- 2. 컴파일 결과에서 og_vlp 인지 og_reach 인지 확인한다
SELECT og_cypher_sql('default', $$ MATCH (a)-[:K*1..6]->(b) RETURN count(DISTINCT b) $$);

-- 3. 질의를 재작성 가능한 형태로 바꾼다 — DISTINCT 를 쓰거나 WITH 를 제거
```

폭주 중인 질의 종료:

```sql
SELECT pid, now() - query_start AS running_for, left(query,120)
  FROM pg_stat_activity WHERE state <> 'idle' AND datname = current_database();
SELECT pg_cancel_backend(<pid>);
```

`statement_timeout`을 미리 걸어두는 것이 근본 대책이다 — [03_configuration.md](03_configuration.md).

---

## 6. 벡터 · 임베딩

| 오류 메시지 | 출처 | 뜻 | 조치 |
|---|---|---|---|
| `genai.vector.encode is disabled. It makes an outbound HTTP request from the database, so it is off until that is chosen deliberately: SELECT og_set_setting('genai.enabled', 'on')` | `engine/src/compat/genai.rs:102-106` | 기본 비활성 | 메시지가 시키는 대로 |
| `no embedding endpoint is configured. The URL is deliberately not an argument — set it with og_set_setting('genai.endpoint', '…')` | `engine/src/compat/genai.rs:109-112` | 엔드포인트 미설정 | `og_set_setting('genai.endpoint', ...)` |
| `provider '{provider}' is not supported. supported: Ollama, OpenAI, AzureOpenAI …` | `engine/src/compat/genai.rs:122-125` | 지원 provider 3종 외 | 셋 중 하나 |
| `no embedding model configured; set genai.model` | `engine/src/compat/genai.rs:133` | 모델 미설정 | `og_set_setting('genai.model', ...)` |
| `embedding request to '{endpoint}' failed: {e}` | `engine/src/compat/genai.rs:148` | HTTP 요청 실패(연결 거부/타임아웃 등) | 엔드포인트 도달성 확인. `genai.timeout_ms` 기본 5000ms (`:41`) |
| `embedding endpoint returned a body that is not JSON: {e}` | `engine/src/compat/genai.rs:147` | 응답이 JSON이 아님 | 엔드포인트 URL이 맞는지 |
| `embedding endpoint returned no vector in the shape '{provider}' produces` | `engine/src/compat/genai.rs:152` | provider 설정과 실제 응답 형태 불일치. ollama는 `{"embeddings":[[…]]}`, 그 외는 `{"data":[{"embedding":[…]}]}` (`:79-83`) | provider 설정을 실제 서버에 맞춘다 |
| `embedding dimension {dims} is out of range (1..16000)` | `engine/src/vector/mod.rs` | pgvector 한계 | 차원 축소 |
| `query vector has {n} dimension(s) but '{type_name}.{prop}' is declared as vector({dims})` | `engine/src/vector/mod.rs` | 질의 벡터와 선언 차원 불일치 | `og_embedding_stats(graph)`로 선언 차원 확인 |
| `no embedding named '{prop}' is declared on this type` | `engine/src/vector/mod.rs` | 임베딩 미선언 | `og_add_embedding(...)` |
| `unknown metric '{other}' (cosine \| l2 \| ip)` | `engine/src/vector/mod.rs` | 메트릭 이름 오타 | 셋 중 하나 |
| `failed to build HNSW index: {e}` | `engine/src/vector/mod.rs` | HNSW 생성 실패. **pgvector의 HNSW는 2000차원에서 멈춘다** (`engine/src/compat/genai.rs:155-159` 주석) | `genai.dimensions`로 2000 이하 절단 |

> **백엔드가 임베딩 요청 중에 응답하지 않는 현상**은 버그가 아니라 구조다.
> `og_genai_encode`는 블로킹 HTTP 클라이언트를 쓴다 (`engine/Cargo.toml:25-29`).
> `pg_stat_activity`에서 해당 백엔드는 실행 중으로 보인다.

---

## 7. 식별자 고갈

| 오류 메시지 | 출처 | 뜻 |
|---|---|---|
| `local id {local} exhausted the 36-bit space for this type` | `engine/src/id.rs:42` | 한 타입의 로컬 id 공간(36비트)을 다 씀 |
| `type id {type_id} out of range (0..{MAX_TYPE_ID})` | `engine/src/id.rs:39` | 타입 id 공간 초과 |
| `shard id {shard} out of range (0..{MAX_SHARD_ID})` | `engine/src/id.rs:36` | 샤드 id 범위 초과 |

`engine/src/id.rs:31-32`의 주석: "Panics (→ `ereport(ERROR)` via pgrx) on overflow so a
silently truncated id can never reach storage." — **잘린 id가 저장되는 일은 없다.**

할당 워터마크 확인:

```sql
SELECT a.type_id, t.name, a.next_id
  FROM og_data.og_id_alloc a
  JOIN og_catalog.type t ON t.type_id = a.type_id
 ORDER BY a.next_id DESC LIMIT 20;
```

---

## 8. Studio

| 증상 | 원인 | 조치 |
|---|---|---|
| `start.sh`가 `✗ studio did not come up` 로 끝남 | 10초(20×0.5s) 안에 `/api/health` 응답 없음 (`start.sh:79-90`) | 호스트의 `/tmp/ontological-studio.log` 확인. `start.sh:89`가 마지막 20줄을 자동 출력한다 |
| `/api/health`가 503 + PostgreSQL 오류 | 확장이 없거나 DB 연결 실패. 응답 본문에 `error`/`code`/`detail`/`hint`가 그대로 실린다 (`portal/server/index.js:67-75,163`) | 본문의 오류 코드를 본다 |
| 모든 요청이 멈춤 | 풀 크기가 **8로 하드코딩**되어 있고 질의 타임아웃이 없다 (`portal/server/index.js:27-28`) | `pg_stat_activity`에서 장기 질의를 찾아 `pg_cancel_backend`. 근본 대책은 서버 쪽 `statement_timeout` — `OPS-12` |
| `EADDRINUSE` | 7474 포트가 이미 사용 중 | `pkill -f "portal/server/index.js"` 또는 `OG_PORT`를 바꿔 실행 |
| 스키마 사이드바가 비어 있음 | `GET /api/schema`가 `og_schema` / `og_graph_stats`를 부른다 (`portal/server/index.js:171-175`). 그래프 이름 기본값은 `default` (`:169`) | `SELECT name FROM og_catalog.graph` 로 실제 그래프 이름 확인 |
| 벤치마크 페이지가 비어 있음 | `OG_BENCH_DIR`이 잘못됐거나 파일명이 `/^bench-(\d+)-(\d{8}T\d{6}Z)\.json$/`에 맞지 않음 (`portal/server/index.js:87,91`) | 파일명 규칙 확인 — [05_benchmarking.md](05_benchmarking.md) |
| 벤치마크 페이지가 낡은 숫자를 보여줌 | 스케일별·시스템별 **newest-wins 병합**이라, 새 실행이 그 시스템을 포함하지 않으면 이전 값이 남는다 (`portal/server/index.js:110-114`) | 원하는 시스템을 포함해 다시 실행 |
| `POST /api/cypher`가 200인데 `compiled_sql`이 `null` | 쓰기 질의는 단일 컴파일 문장이 없다 (`portal/server/index.js:200-202` 주석) | 정상 동작 |
| Studio를 `pkill`로 죽였는데 커넥션이 남음 | `SIGINT` 핸들러만 있다 (`portal/server/index.js:374-376`). `pkill` 기본은 `SIGTERM` | `pkill -INT -f "portal/server/index.js"` — `OPS-09` |

---

## 9. Bolt 게이트웨이

| 증상 / 메시지 | 원인 | 조치 |
|---|---|---|
| `ontological-bolt: cannot bind {listen}: {e}` 후 종료 코드 1 | 포트 점유 또는 권한 (`bolt/src/main.rs:49-51`) | `OG_BOLT_LISTEN`을 바꾸거나 기존 프로세스 종료 |
| `ontological-bolt: accept failed: {e}` | accept 실패. 루프는 계속된다 (`bolt/src/main.rs:63-66`) | 파일 디스크립터 한도 확인 |
| `ontological-bolt: session {peer} ended: {e}` | 세션 종료. **드라이버가 연결을 닫는 것은 정상 종료**이며 `UnexpectedEof`는 로그에 남지 않는다 (`bolt/src/main.rs:74-77`) | 대부분 무시 가능 |
| 게이트웨이가 조용히 사라짐 | `docker exec -d`로 띄워 감독자가 없다 (`start.sh:61-63`) | `pgrep -af ontological-bolt`로 확인 후 재기동. `./start.sh` 재실행도 같은 일을 한다 — `OPS-07` |
| 엉뚱한 DB에 붙거나 붙지 못함 | `OG_BOLT_PGPORT` 기본값이 **5432**이며 이 프로젝트의 28816과 다르다 (`bolt/src/main.rs:40`). 파싱 실패 시에도 조용히 5432로 되돌아간다 | 환경변수를 명시적으로 넘긴다 |
| 드라이버 연결 실패(프로토콜 협상) | Bolt **4.4만** 지원. 3.x / 5.x는 말하지 않는다. 드라이버는 범위를 제안하므로 현행 Python 드라이버(6.2)는 4.4로 합의된다 (`bolt/README.md:60-61`) | 드라이버 버전 확인 |
| `Path` 타입이 리스트로 옴 | 설계다 — Path 구조체로 인코딩하지 않고 홉의 리스트로 전달한다 (`bolt/README.md:65`) | 클라이언트에서 처리 |
| 시간/공간 타입 파라미터가 거부됨 | 미지원. **조용히 뭉개지 않고 거부한다** (`bolt/README.md:66`) | 다른 타입으로 전달 |
| TLS 연결 실패 | 게이트웨이는 TLS를 종단하지 않는다 (`bolt/README.md:68`) | 앞단에 TLS 프록시 |
| 인증 실패 | 사용자 저장소가 없다. `HELLO`의 자격증명은 **PostgreSQL 역할과 비밀번호**다 (`bolt/README.md:43`) | PostgreSQL 역할로 psql 접속이 되는지 먼저 확인 |
| `CALL {}` / GDS 오류 | spec 003이 지원하지 않으며 전송 계층이 의미를 추가하지 않는다 (`bolt/README.md:71`) | — |

Bolt 경로 자체를 확인하는 정식 방법은 `tests/neo4j-movies/run.py`다 —
raw 핸드셰이크로 어느 포트가 Bolt를 말하는지 검사한다
([04_testing_and_ci.md](04_testing_and_ci.md) §5).

---

## 10. 테스트 · 벤치마크

| 증상 | 원인 | 조치 |
|---|---|---|
| `tests/run.sh`가 전부 FAIL | 확장 미설치 또는 서버 미기동 | `pg_isready` → `SELECT ontological_version()` 순으로 확인 |
| `backup round trip FAIL (before=0/0 …)` | 데모 데이터가 적재되지 않았다 | `examples/demo.sql` 적재 실패 원인을 본다 |
| `backup round trip FAIL (before=X after=Y)` | `pg_extension_config_dump` 등록 누락 | 새 테이블을 추가했다면 등록을 추가 — [08_backup_and_restore.md](08_backup_and_restore.md) |
| `tests/run.sh`가 종료 코드 0인데 `integrity FAIL`이 출력됨 | **조용한 실패.** 무결성 블록이 파이프 서브셸이라 `$fail`을 증가시키지 못한다 (`tests/run.sh:71-75`) | 출력 문구를 직접 확인. `OPS-14` |
| `cargo pgrx test`가 아무것도 안 함 | `#[pg_test]`가 저장소에 하나도 없다 | `tests/run.sh`를 쓸 것 |
| `CREATE EXTENSION engine` 오류 | `engine/tests/pg_regress/sql/setup.sql:3`의 pgrx 템플릿 잔재. 그런 확장은 없다 | pg_regress 경로는 배선되어 있지 않다 — `OPS-03` |
| `! unknown system '<name>'` | `bench/harness.py`의 `SYSTEMS`에 없는 이름 | `ontological`, `ontological_raw`, `age`, `age_explicit`, `cte`, `pggraph`, `neo4j`, `typedb` |
| `~ skipping <name>: extension not installed` | 해당 시스템의 확장/서버/드라이버 부재. **실패가 아니다** (`bench/harness.py:1026-1029`) | 필요하면 설치 |
| `! <label>: systems disagree {...} — timings for this query are VOID` | 정확성 게이트 실패 | **그 질의의 숫자를 쓰지 말 것.** 답이 왜 다른지 먼저 해결 |
| `<name> crashed the server at <label>` | 그 시스템이 postmaster를 죽였다. 결과로 기록되고 더 깊은 질문은 하지 않는다 (`bench/harness.py:96-104`) | 기록 자체가 결과다 |
| `REGRESSION detected:` 후 종료 코드 1 | baseline 대비 20% 이상 느려짐 (`bench/harness.py:1223`) | baseline과 **같은 `--scale`/`--degree`** 로 실행했는지 먼저 확인 |
| `neo4j` / `typedb`가 계속 skip | 드라이버 미설치 또는 서버 미기동 | `pip install neo4j typedb-driver` + `bench/README.md:61-66`의 docker 명령 |

---

## 11. 조용한 실패 모음 — 오류가 나지 않는 실패

이 절이 이 문서에서 가장 중요하다. 아래 항목들은 **성공한 것처럼 보인다.**

| # | 무엇이 조용히 실패하는가 | 근거 | 어떻게 알아채는가 |
|---|---|---|---|
| 1 | `start.sh`의 확장 생성 + 데모 적재 | `start.sh:51` — 블록 전체가 `>/dev/null 2>&1` | `SELECT ontological_version()` 및 노드 수 확인 |
| 2 | `start.sh`의 PostgreSQL 기동 | `start.sh:37` — `|| true` | `pg_isready` |
| 3 | `start.sh`의 Bolt 빌드 | `start.sh:60` — `|| true` | `pgrep -af ontological-bolt` |
| 4 | `og_apply_role`의 모든 `SET` | `engine/src/agent/mod.rs:427,430,434,438` — 전부 `.ok()` | `SHOW statement_timeout` 등으로 직접 확인 |
| 5 | `og.max_rows` 제한 | 설정만 되고 읽는 코드가 없다 | 행 수가 제한되지 않는다 — `OPS-11` |
| 6 | 씨앗 설정 키 4종의 변경 | 읽는 코드가 없다 | 동작이 바뀌지 않는다 — `OPS-10` |
| 7 | 감사 로그 기록 | `engine/src/cypher/mod.rs:134` — `.ok()` | `og_data.og_audit` 행 수가 호출 수와 맞는지 |
| 8 | 타입 별칭 뷰 생성 | `engine/src/catalog/types.rs:96` — `pgrx::log!`로 남기고 계속 진행. "Never fatal: the view is a convenience" | PostgreSQL 서버 로그에서 `could not create the alias view` 검색 |
| 9 | `tests/run.sh`의 무결성 결과 | `tests/run.sh:71-75` — 서브셸 | 출력 문구 직접 확인 |
| 10 | 백업의 그래프 내용 (등록 누락 시) | `engine/sql/bootstrap.sql:396-398` | 복원 후 노드/엣지 카운트 대조 |
| 11 | Studio의 반쯤 쓰인 벤치 JSON | `portal/server/index.js:100-102` — `catch { continue; }` | 페이지에 특정 실행이 안 보임 |

---

## 12. PostgreSQL 로그에서 이 확장을 찾기

**확장이 PostgreSQL 로그에 남기는 것은 거의 없다.** 실측:

| 레벨 | 사용 횟수 | 위치 |
|---|---|---|
| `pgrx::log!` (LOG) | **1** | `engine/src/catalog/types.rs:96` — 별칭 뷰 생성 실패 |
| `pgrx::notice!` (NOTICE) | **1** | `engine/src/catalog/types.rs:168` — 없는 라벨 힌트 |
| `error!` (ERROR) | 약 115곳 | 전 모듈 |
| `warning!` / `info!` / `debug1!` | **0** | — |

즉 **기동·종료·스키마 변경·재정리 같은 수명주기 이벤트에 대한 로그가 없다.**
운영 관측은 `og_data.og_audit`와 `pg_stat_*`에 의존해야 한다.
→ [10_improvements_ops.md](10_improvements_ops.md) `OPS-18`

```sql
-- 서버 로그 위치
SHOW log_directory;
SHOW log_destination;
SHOW logging_collector;
```

---

## 금지 / 필수

### 금지 (Forbidden)

- 이 문서에 없는 오류 메시지를 지어내지 말 것. 출처는 `grep -rn` 으로 확인할 것.
- `agree: false`인 벤치마크 셀의 숫자를 근거로 조치하지 말 것.
- CSR이 오래된 답을 준다고 버그로 처리하지 말 것 — 설계다.
- 무결성 위반 상태에서 계속 쓰지 말 것.

### 필수 (Required)

- 문제를 만나면 **§0의 5개 층 확인**부터 할 것.
- 조용한 실패(§11) 목록을 정기 점검 항목에 포함할 것.
- Cypher 문제는 `og_cypher_sql` → `og_explain_error` → `og_diagnose_empty` 순으로 좁힐 것.
- 재현 절차를 남길 때는 `og_data.og_audit`의 해당 행을 함께 첨부할 것.

---

<!-- affects: ops, backend, frontend -->
<!-- requires-update: docs/08_operations/06_monitoring.md, docs/08_operations/10_improvements_ops.md -->
