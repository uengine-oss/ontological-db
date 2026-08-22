# 05. 임베딩 파이프라인 — 생성 → 저장 → 갱신

> **이 문서가 답하는 질문**
> - 임베딩은 어디에 어떻게 저장되는가? 별도 벡터 스토어가 있는가?
> - `genai.vector.encode` 의 계약은 정확히 무엇인가 — 설정 키, 요청/응답 형식, 실패 처리?
> - stale 임베딩은 어떻게 탐지되고, 무엇이 탐지되지 않는가?
> - 실패 모드(네트워크 / 키 / 차원 불일치)에서 각각 무슨 일이 벌어지는가?
> - API 키는 어디에 저장되고, 그 노출 위험은?

---

## 1. 결정(Decision) — 임베딩은 별도 스토어가 아니라 컬럼이다

[engine/src/vector/mod.rs:1-12](../../engine/src/vector/mod.rs) 모듈 주석:

> There is no separate embedding store. An embedding is a `vector(N)` property,
> which spec 002 turns into a real column on the type table and spec 001 stores
> like any other column. That single decision is what makes **relationship
> embeddings first class** (FR-002).

결과:

| 얻는 것 | 근거 |
|---|---|
| MVCC / 트랜잭션 가시성 (FR-025) | 일반 컬럼이므로 자동 |
| RLS (spec 005) | 타입 테이블 정책이 그대로 적용 |
| 삭제 즉시 반영 (FR-026) | 행이 사라지면 벡터도 사라짐 |
| 백업 | `pg_extension_config_dump` 대상 테이블 ([engine/sql/bootstrap.sql:405-434](../../engine/sql/bootstrap.sql)) |
| **관계(엣지) 임베딩** | 엣지 타입 테이블도 같은 방식으로 컬럼을 가짐 |

---

## 2. 사실 — 슬롯 선언: `og_add_embedding`

정의: [engine/src/vector/mod.rs:32-75](../../engine/src/vector/mod.rs).

```sql
SELECT og_add_embedding(
  'kb',            -- graph
  'Doc',           -- type_name  (엔티티 또는 관계 타입)
  'emb',           -- prop
  1024,            -- dims
  'cosine',        -- metric: cosine | l2 | ip  (기본 'cosine')
  'body'           -- source_prop (staleness 추적용, 기본 NULL)
);
```

수행하는 일, 순서대로:

1. **차원 검증**: `1..=16000` 밖이면 거부
   ([vector/mod.rs:41-43](../../engine/src/vector/mod.rs)).
2. **메트릭 → opclass 매핑** ([vector/mod.rs:19-26, 46](../../engine/src/vector/mod.rs)):
   | metric | 연산자 | opclass |
   |---|---|---|
   | `cosine` | `<=>` | `vector_cosine_ops` |
   | `l2` / `euclidean` | `<->` | `vector_l2_ops` |
   | `ip` / `inner_product` / `dot` | `<#>` | `vector_ip_ops` |
3. **일반 프로퍼티 경로 재사용**: `og_add_property(graph, type, prop, 'vector(N)', false, false)`
   ([vector/mod.rs:49-53](../../engine/src/vector/mod.rs)). `'vector(N)'` 은
   [engine/src/catalog/types.rs:29-37](../../engine/src/catalog/types.rs) 의 `map_data_type` 이
   허용하는 유일한 동적 타입이다.
4. **서브타입 전체에 HNSW 인덱스 생성**
   ([vector/mod.rs:56-64](../../engine/src/vector/mod.rs)):
   `CREATE INDEX IF NOT EXISTS hnsw_{sub}_{col} ON {table} USING hnsw ({col} {opclass})`
   — HNSW 파라미터(`m`, `ef_construction`)는 지정하지 않으므로 pgvector 기본값이 쓰인다.
5. **카탈로그 등록** (`og_catalog.embedding`, upsert)
   ([vector/mod.rs:66-74](../../engine/src/vector/mod.rs),
   [engine/sql/bootstrap.sql:266-275](../../engine/sql/bootstrap.sql)).

### 2.1 pgvector HNSW 차원 상한과의 어긋남

코드가 허용하는 차원은 16000까지지만
([vector/mod.rs:41-42](../../engine/src/vector/mod.rs)), 이 저장소 자신이 두 곳에서
"pgvector's HNSW index stops at 2000" 이라고 적고 있다
([engine/src/compat/genai.rs:157-159](../../engine/src/compat/genai.rs),
[examples/meeting-rooms/og_mcp.py:104-107](../../examples/meeting-rooms/og_mcp.py)).

따라서 `og_add_embedding(..., 4096, ...)` 은 3단계까지 성공한 뒤 4단계의
`CREATE INDEX` 에서 실패하고, 메시지는 `failed to build HNSW index: {e}` 다
([vector/mod.rs:62](../../engine/src/vector/mod.rs)) — 상한이 2000이라는 안내는 없다.
spec 004 Edge Cases의 "매우 높은 차원: pgvector 인덱스 차원 상한을 넘는 임베딩 요청 시
명확한 안내"([specs/004-vector-hybrid-search/spec.md:165](../../specs/004-vector-hybrid-search/spec.md))
는 충족되지 않는다.

### 2.2 차원/모델 변경에 마이그레이션 경로가 없다

`og_catalog.embedding` 은 upsert이므로 메타데이터는 갱신된다
([vector/mod.rs:69-71](../../engine/src/vector/mod.rs) — `ON CONFLICT (type_id, prop) DO UPDATE SET dims = …`).
그러나 그 앞의 3단계는 `og_add_property` 이고, 이 함수는
`og_catalog.property` 에 **`ON CONFLICT` 없이 INSERT** 한다
([engine/src/catalog/types.rs:539-545](../../engine/src/catalog/types.rs)):

```rust
Spi::run_with_args(
    "INSERT INTO og_catalog.property
        (prop_id, type_id, name, data_type, column_name, required, is_key)
     VALUES (nextval('og_catalog.property_id_seq')::int4, $1, $2, $3, $4, $5, $6)",
    …)
.expect("property insert failed");
```

`og_catalog.property` 에는 `UNIQUE (type_id, name)` 이 있으므로
([engine/sql/bootstrap.sql](../../engine/sql/bootstrap.sql) `property` 테이블 정의),
같은 `prop` 이름으로 `og_add_embedding` 을 다시 호출하면 유니크 위반으로 실패한다
(`failed to declare embedding property: …`, [vector/mod.rs:53](../../engine/src/vector/mod.rs)).

설령 통과하더라도 컬럼 생성은 `ALTER TABLE … ADD COLUMN IF NOT EXISTS`
([types.rs:550](../../engine/src/catalog/types.rs))이므로 **기존 `vector(N)` 컬럼의 차원은
바뀌지 않는다.**

**결론(사실)**: 임베딩 모델을 바꿔 차원이 달라지면, 지원되는 경로는 **새 이름의 슬롯을
추가로 선언하는 것**뿐이다. spec 004 FR-005("동일 타입에 서로 다른 차원·모델의 임베딩을
복수 개 운용")가 이를 허용한다. 기존 슬롯의 in-place 차원 변경은 미지원이며,
`og_drop_*` 계열에도 임베딩 슬롯 삭제 함수는 없다(공개 함수 목록에 `og_drop_embedding` 부재).

### 2.3 Neo4j DDL 경로는 `source_prop` 를 잃는다

`CREATE VECTOR INDEX … FOR (m:MeetingRoom) ON (m.name_vec) OPTIONS {…}` 는
[engine/src/compat/ddl.rs:211-230](../../engine/src/compat/ddl.rs) 에서 처리되고,
`og_add_embedding` 을 **인자 5개로** 호출한다:

```rust
// engine/src/compat/ddl.rs:225-228
"SELECT og_add_embedding($1, $2, $3, $4, $5)",
&[graph, label, prop, dims, metric]
```

`source_prop` 가 없으므로 NULL이 되고, **이 슬롯은 `og_stale_embeddings` 에서 영원히
제외된다** (4.1절 참조). `examples/meeting-rooms/load.py:48-55` 가 만드는
`room_name` 인덱스가 정확히 이 경우다.

---

## 3. 사실 — `genai.vector.encode` 계약

정의: [engine/src/compat/genai.rs](../../engine/src/compat/genai.rs) 전체 (177줄).
Cypher 표면에서 `genai.vector.encode(resource, provider, configuration)` 로 호출되며
`og_genai_encode(text, provider, config)` 로 컴파일된다
([engine/src/cypher/compile.rs:1545-1557](../../engine/src/cypher/compile.rs)).

### 3.1 설정 키 전체 (`og_catalog.setting`)

| 키 | 필수 | 기본값 | 근거 |
|---|---|---|---|
| `genai.enabled` | ✅ `'on'` 이어야 함 | 미설정 = 비활성 | [genai.rs:101-107](../../engine/src/compat/genai.rs) |
| `genai.endpoint` | ✅ | 없음 (미설정 시 오류) | [genai.rs:108-113](../../engine/src/compat/genai.rs) |
| `genai.provider` | ❌ | `'ollama'` | [genai.rs:116-120](../../engine/src/compat/genai.rs) |
| `genai.model` | ✅ (인자로 대체 가능) | 없음 | [genai.rs:128-133](../../engine/src/compat/genai.rs) |
| `genai.dimensions` | ❌ | 없음 = 절단 안 함 | [genai.rs:160-163](../../engine/src/compat/genai.rs) |
| `genai.timeout_ms` | ❌ | **5000** | [genai.rs:41, 135-137](../../engine/src/compat/genai.rs) |
| `genai.token` | ❌ | 없음 = `Authorization` 헤더 미부착 | [genai.rs:140-142](../../engine/src/compat/genai.rs) |

복사-붙여넣기 가능한 설정:

```sql
SELECT og_set_setting('genai.enabled',    'on');
SELECT og_set_setting('genai.endpoint',   'http://localhost:11434/api/embed');
SELECT og_set_setting('genai.provider',   'ollama');
SELECT og_set_setting('genai.model',      'qwen3-embedding:latest');
SELECT og_set_setting('genai.dimensions', '1024');
SELECT og_set_setting('genai.timeout_ms', '5000');
```

### 3.2 provider 허용 목록

```rust
// engine/src/compat/genai.rs:121-126
if !matches!(provider.as_str(), "ollama" | "openai" | "azureopenai") { error!(…) }
```

세 값만 허용된다(소문자 접기 후 비교). 그 외는 이름으로 거부된다.

### 3.3 요청 본문 — 두 갈래로 보이지만 실제로는 하나

```rust
// engine/src/compat/genai.rs:70-75
fn request_body(provider: &str, model: &str, text: &str) -> Value {
    match provider {
        "ollama" => json!({ "model": model, "input": text }),
        _        => json!({ "model": model, "input": text }),
    }
}
```

두 분기의 본문이 **동일하다.** 주석은 "Two shapes cover what people actually run"
([genai.rs:66-69](../../engine/src/compat/genai.rs))이라고 하지만, 실제로는 `{"model", "input"}`
하나만 보낸다. Ollama `/api/embed` 와 OpenAI `/v1/embeddings` 가 둘 다 `input` 을
받으므로 결과적으로 동작하지만, 코드와 주석이 어긋난다.

### 3.4 응답 파싱

```rust
// engine/src/compat/genai.rs:77-88
"ollama" => reply["embeddings"][0]          // {"embeddings": [[...]]}
_        => reply["data"][0]["embedding"]   // {"data": [{"embedding": [...]}]}
```

파싱 실패 시: `embedding endpoint returned no vector in the shape '{provider}' produces`
([genai.rs:151-153](../../engine/src/compat/genai.rs)).

### 3.5 후처리 — 절단 후 재정규화

```rust
// engine/src/compat/genai.rs:160-175
let dims = cfg["dimensions"] ?? setting("genai.dimensions");
if let Some(dims) = dims { if dims < vector.len() { vector.truncate(dims); } }
let norm = vector.iter().map(|x| x*x).sum::<f64>().sqrt();
if norm > 0.0 { for x in &mut vector { *x /= norm; } }
```

- 절단은 **`dims < len` 일 때만**. `dims > len` 이면 아무 일도 일어나지 않는다
  (패딩도, 오류도 없음).
- **L2 정규화는 무조건 수행된다** — `metric` 이 `cosine` 이 아닐 때도.
  cosine/l2에서는 순위가 보존되지만, 내적(`ip`, `<#>`)에서는 벡터의 크기(magnitude)를
  버리므로 의미가 달라진다. `og_add_embedding(..., 'ip')` 슬롯과 조합할 때 주의.
- 절단의 정당성은 Matryoshka 학습 모델을 전제한다
  ([genai.rs:155-159](../../engine/src/compat/genai.rs) 주석). 그렇지 않은 모델의 벡터를
  자르면 품질이 무너진다 — 코드는 모델 종류를 확인하지 않는다.

### 3.6 HTTP 호출의 실제 형태

```rust
// engine/src/compat/genai.rs:139-149
let mut request = ureq::post(&endpoint).timeout(Duration::from_millis(timeout));
if let Some(token) = setting("genai.token") {
    request = request.set("Authorization", &format!("Bearer {token}"));
}
match request.send_json(request_body(&provider, &model, resource)) {
    Ok(response) => response.into_json()…,
    Err(e) => error!("embedding request to '{endpoint}' failed: {e}"),
}
```

의존성 선언과 그 이유:
[engine/Cargo.toml:25-29](../../engine/Cargo.toml)

```toml
# The extension's only outbound network, for `genai.vector.encode`. A blocking
# client with no runtime is the right shape here: a PostgreSQL backend is
# already the thread doing the waiting, so an async stack would buy nothing and
# cost a scheduler.
ureq = { version = "2", default-features = false, features = ["json", "tls"] }
```

**없는 것 (전부 코드에서 확인)**

| 항목 | 상태 |
|---|---|
| 재시도 | **없음.** `send_json` 1회. 실패 = 즉시 `error!` ([genai.rs:144-149](../../engine/src/compat/genai.rs)) |
| 백오프 / 지터 | 없음 |
| 레이트 리밋 / 동시성 제한 | 없음 |
| 429 / 5xx 구분 처리 | 없음. `ureq` 는 4xx/5xx를 `Err` 로 주며 모두 동일 메시지로 중단 |
| 배치 요청 | **없음.** 시그니처가 `resource: &str` 단일 문자열 ([genai.rs:96-97](../../engine/src/compat/genai.rs)) |
| 결과 캐시 | 없음 |
| 함수 휘발성 | `#[pg_extern]` — 즉 **VOLATILE** ([genai.rs:95](../../engine/src/compat/genai.rs)). 행마다 재호출된다 |
| 응답 크기 상한 | 없음 |
| 요청 로깅 / 감사 | 없음 (`og_data.og_audit` 에 기록되지 않음) |

`timeout` 만이 유일한 안전장치이고 기본 5초다. N행에 대해 호출하면 최악의 경우
`N × 5초` 동안 백엔드 하나가 네트워크에 묶인다.

---

## 4. 사실 — staleness 추적

### 4.1 `og_stale_embeddings(graph)`

정의: [engine/src/vector/mod.rs:299-355](../../engine/src/vector/mod.rs).

```sql
-- engine/src/vector/mod.rs:308-311 (대상 슬롯 선정)
SELECT e.type_id, t.name, e.prop, e.source_prop
  FROM og_catalog.embedding e JOIN og_catalog.type t ON t.type_id = e.type_id
 WHERE t.graph_id = $1 AND e.source_prop IS NOT NULL
```

**`source_prop IS NOT NULL` 인 슬롯만 대상이다.** 2.3절의 Neo4j DDL 경로로 만든
슬롯은 여기서 빠진다.

각 슬롯의 각 서브타입 테이블에 대해:

```sql
-- engine/src/vector/mod.rs:336-341
SELECT x.id FROM {table} x
  LEFT JOIN og_data.og_embedding_state s
    ON s.entity_id = x.id AND s.prop = '{prop}'
 WHERE x.{source_col} IS NOT NULL
   AND (x.{emb_col} IS NULL
        OR s.source_hash IS DISTINCT FROM md5(x.{source_col}::text))
```

즉 stale의 정의는 **"소스 값이 있는데 (벡터가 없거나, 기록된 소스 해시가 현재 소스의
md5와 다르다)"** 이다.

### 4.2 `og_mark_embedded(entity_id, prop)`

정의: [engine/src/vector/mod.rs:358-381](../../engine/src/vector/mod.rs).
현재 소스 값의 md5를 `og_data.og_embedding_state` 에 기록한다.

```sql
INSERT INTO og_data.og_embedding_state (entity_id, prop, source_hash, embedded_at)
SELECT $1, $2, md5(x.{source_col}::text), now() FROM {table} x WHERE x.id = $1
ON CONFLICT (entity_id, prop) DO UPDATE SET source_hash = EXCLUDED.source_hash, embedded_at = now()
```

`source_prop` 가 선언되지 않은 슬롯이면 **조용히 반환한다**
([vector/mod.rs:368](../../engine/src/vector/mod.rs) — `let Some(source) = source else { return }`).
오류도 경고도 없다.

### 4.3 갱신은 폴링이지 트리거가 아니다

spec 004 plan은 "소스 컬럼 변경 시 `og_data.og_embedding_stale` 에 기록하는 트리거를
타입 테이블에 건다"고 적었지만
([specs/004-vector-hybrid-search/plan.md:69-70](../../specs/004-vector-hybrid-search/plan.md)),
구현은 트리거가 아니라 **조회 시점 스캔**이다. `og_data.og_embedding_stale` 테이블은
존재하지 않고 `og_data.og_embedding_state` 만 있다
([engine/sql/bootstrap.sql:279-285](../../engine/sql/bootstrap.sql)).

비용: `og_stale_embeddings` 는 슬롯 × 서브타입마다 **전체 테이블 스캔**을 돈다
([vector/mod.rs:330-348](../../engine/src/vector/mod.rs)) — `md5(source::text)` 계산이
행마다 발생하므로 인덱스로 회피할 수 없다.

### 4.4 운영 루프 (복사-붙여넣기 가능)

```sql
-- 1. 재생성 대상 확인
SELECT entity_id, type_name, prop FROM og_stale_embeddings('kb');

-- 2. DB 안에서 재생성 (genai 활성화된 경우)
--    주의: 행마다 HTTP 1회. 배치 없음. 큰 집합에는 부적합.
UPDATE og_data.n_5 x
   SET p_emb = og_genai_encode(x.p_body)::text::vector
 WHERE x.id IN (SELECT entity_id FROM og_stale_embeddings('kb') WHERE prop = 'emb');

-- 3. 최신 상태로 표시
SELECT og_mark_embedded(entity_id, prop) FROM og_stale_embeddings('kb');
```

3번은 2번과 **같은 트랜잭션에서 실행해야** 의미가 있다. 그 사이에 소스가 바뀌면
해시가 어긋난 상태로 "최신"으로 기록된다.

`og_embedding_stats(graph)` 는 선언된 슬롯 목록(type, property, dims, metric,
source_property)만 반환하며 **stale 비율·개수·인덱스 크기는 반환하지 않는다**
([vector/mod.rs:383-408](../../engine/src/vector/mod.rs)). spec 004 FR-029가 요구하는
"개수, 차원, stale 비율, 인덱스 크기" 중 차원만 충족된다.

---

## 5. 사실 — 실패 모드 표

| 실패 | 발생 위치 | 증상 | 복구 |
|---|---|---|---|
| `genai.enabled` 미설정 | [genai.rs:101-107](../../engine/src/compat/genai.rs) | `genai.vector.encode is disabled…` 오류, 트랜잭션 중단 | `og_set_setting('genai.enabled','on')` |
| `genai.endpoint` 미설정 | [genai.rs:108-113](../../engine/src/compat/genai.rs) | `no embedding endpoint is configured…` | 설정 |
| 미지원 provider | [genai.rs:121-126](../../engine/src/compat/genai.rs) | `provider 'x' is not supported…` | ollama/openai/azureopenai 중 하나로 |
| `genai.model` 미설정 | [genai.rs:128-133](../../engine/src/compat/genai.rs) | `no embedding model configured` | 설정 또는 `configuration` 인자 |
| 네트워크 실패 / 타임아웃 / 4xx / 5xx | [genai.rs:148](../../engine/src/compat/genai.rs) | `embedding request to '{endpoint}' failed: {e}` — **전부 동일 경로, 재시도 없음** | 애플리케이션이 재시도 |
| JSON이 아닌 응답 | [genai.rs:147](../../engine/src/compat/genai.rs) | `embedding endpoint returned a body that is not JSON` | 엔드포인트 확인 |
| 응답 shape 불일치 | [genai.rs:151-153](../../engine/src/compat/genai.rs) | `returned no vector in the shape '{provider}' produces` | provider 값 교정 |
| 질의 벡터 차원 ≠ 선언 차원 | [vector/mod.rs:134-139](../../engine/src/vector/mod.rs) | `query vector has N dimension(s) but 'T.p' is declared as vector(D)` | 차원 맞추기 |
| 저장 벡터 차원 불일치 | pgvector | PostgreSQL의 `expected N dimensions, not M` | — |
| HNSW 차원 상한 초과 | [vector/mod.rs:58-62](../../engine/src/vector/mod.rs) | `failed to build HNSW index: …` (상한 안내 없음) | `genai.dimensions` 로 2000 이하 절단 |
| 슬롯 재선언(차원 변경) | [types.rs:539-545](../../engine/src/catalog/types.rs) | `failed to declare embedding property: …` (유니크 위반) | 새 이름의 슬롯 추가 (2.2절) |

**차원 검사의 형태에 주의**: `og_vector_search` 의 검사는 문자열의 콤마 개수다
([vector/mod.rs:134-138](../../engine/src/vector/mod.rs)):

```rust
if query.matches(',').count() + 1 != dims as usize { error!(…) }
```

`'[]'` (빈 벡터)는 콤마 0개 → 1차원으로 계산되어 오탐할 수 있다.

---

## 6. 사실 — API 키 저장과 노출 위험

`genai.token` 은 `og_catalog.setting` 테이블에 **평문 text**로 저장된다
([engine/src/compat/genai.rs:140](../../engine/src/compat/genai.rs),
[engine/sql/bootstrap.sql:252-255](../../engine/sql/bootstrap.sql)).

```sql
CREATE TABLE og_catalog.setting ( key text PRIMARY KEY, value text NOT NULL );
```

확인된 사실:

1. **pg_dump에 포함된다.** 덤프 대상 필터는 시드 4개 키만 제외한다
   ([engine/sql/bootstrap.sql:420-422](../../engine/sql/bootstrap.sql)):
   ```sql
   SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
       'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                          ''inference_max_depth'', ''schema_version'')');
   ```
   `genai.token` 은 제외 목록에 없으므로 **백업 파일에 평문으로 들어간다.**
2. **GRANT/REVOKE가 없다.** `engine/sql/bootstrap.sql` 과 `engine/sql/access.sql` 에
   `GRANT`/`REVOKE`/`ROW LEVEL SECURITY` 구문이 **하나도 없다**(전수 grep 결과 0건).
   따라서 테이블 권한은 PostgreSQL 기본값을 따르며, 확장 소유자 외의 접근 여부는
   설치 환경의 스키마 권한에 달려 있다(환경 의존 — 이 저장소만으로는 미확인).
3. **`og_set_setting` 에 권한 검사가 없다**
   ([engine/src/compat/genai.rs:55-63](../../engine/src/compat/genai.rs)). pgrx의
   `#[pg_extern]` 은 기본적으로 `EXECUTE` 권한이 `PUBLIC` 이므로, 이 함수를 호출할 수
   있는 역할은 `genai.endpoint` 를 임의 URL로 바꿀 수 있다.

   모듈 주석은 "a caller who can write Cypher cannot make the server fetch a URL of
   their choosing"([genai.rs:22-24](../../engine/src/compat/genai.rs))라고 말하고, 그 범위
   안에서는 참이다 — `genai.vector.encode` 는 엔드포인트를 인자로 받지 않는다.
   그러나 **평문 SQL로 `og_set_setting` 을 호출할 수 있으면 그 보증은 우회된다.**
   Bolt 게이트웨이는 Cypher만 받으므로 Bolt 경로에서는 우회할 수 없으나,
   Studio의 `POST /api/sql` 은 임의 SQL을 받는다
   ([portal/server/index.js:296](../../portal/server/index.js)).

---

## 7. 필수(Required) / 금지(Forbidden)

**필수**

- 임베딩 슬롯을 만들 때 `source_prop` 를 **반드시** 지정할 것. 없으면
  `og_stale_embeddings` 가 그 슬롯을 영원히 무시한다 (4.1절).
- `CREATE VECTOR INDEX` (Neo4j DDL)로 슬롯을 만든 경우, 이후에 staleness 추적이
  필요하면 `og_catalog.embedding.source_prop` 를 직접 갱신하거나 `og_add_embedding` 으로
  재선언할 것 (2.3절 — 단, 2.2절의 유니크 제약에 주의).
- `genai.dimensions` 를 명시할 것. HNSW 상한(2000) 아래로 맞추는 유일한 지점이다.
- 대량 임베딩 재생성은 **애플리케이션 쪽 배치**로 할 것. DB 안의
  `og_genai_encode` 는 행당 HTTP 1회이고 재시도가 없다 (3.6절).
- `og_genai_encode` 를 쓸 세션에는 반드시 `statement_timeout` 을 걸 것
  (`og_apply_role` 의 `statement_timeout_ms`).

**금지**

- `genai.token` 을 담은 데이터베이스의 `pg_dump` 산출물을 비암호화 저장소에 두는 것 금지 (6절).
- 에이전트/애플리케이션 역할에 `og_set_setting` 실행 권한을 남기는 것 금지 (6.3절).
- 내적(`ip`) 메트릭 슬롯에 `genai.vector.encode` 의 출력을 그대로 넣는 것 금지 —
  무조건 L2 정규화되어 크기 정보가 사라진다 (3.5절).
- Matryoshka 학습이 아닌 모델의 벡터에 `genai.dimensions` 절단을 적용하지 말 것 (3.5절).
- `og_mark_embedded` 를 재생성과 다른 트랜잭션에서 호출하지 말 것 (4.4절).

---

## 8. 참고

- 원문: [examples/meeting-rooms/README.md:95-129](../../examples/meeting-rooms/README.md) "Where the text becomes a vector"
- 함수 계약: [docs/api.md:126-140](../../docs/api.md)
- 스펙: FR-004/FR-005/FR-006/FR-022~FR-024/FR-029
  ([specs/004-vector-hybrid-search/spec.md:179-185, 219-234](../../specs/004-vector-hybrid-search/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-06, LLM-07, LLM-08, LLM-11, LLM-12

<!-- affects: llm, data, backend, security -->
<!-- requires-update: 02_api/00_index.md, 05_llm/06_retrieval_and_rrf.md -->
