# Neo4j 호환면 — 인덱스 DDL · 프로시저 · Bolt 게이트웨이

> **이 문서가 답하는 질문**
> - Neo4j 애플리케이션이 URI만 바꿔 붙으려면 무엇이 필요한가?
> - `CREATE INDEX` / `CREATE CONSTRAINT`는 실제로 무엇을 만드는가?
> - 어떤 `db.*` / `apoc.*` 프로시저가 있고, 없는 것은 어떻게 거부되는가?
> - `genai.vector.encode`는 어떻게 설정하고 왜 기본 꺼짐인가?
> - Bolt 게이트웨이가 노출하는 계약은 정확히 무엇이고, 무엇을 지원하지 않는가?

---

## 1. 결정(Decision) — 원래 이름으로 제공한다

Neo4j를 상대로 쓰인 Cypher는 질의 언어만 쓰지 않는다. 이름으로 인덱스를 만들고,
`db.*` 프로시저를 부르고, 몇 개의 APOC 헬퍼에 손을 뻗는다. 그중 하나라도 없으면
URI만 바꿔서는 옮길 수 없다. 그래서 이 모듈이 그것들을 **원래 이름 그대로**
제공하되, 아래의 네이티브 표면으로 매핑한다
([engine/src/compat/mod.rs:1](../../engine/src/compat/mod.rs#L1)).

**여기 있는 것은 엔진에 없는 의미론을 더하지 않는다.**
`db.index.vector.queryNodes`는 Neo4j 이름으로 도달한 `og_vector_search`이고,
`CREATE CONSTRAINT`는 `og_add_property(..., is_key := true)` 다.
등가물이 진짜로 없으면 **조용히 근사하지 않고 이름으로 거부**한다 — 전문 검색만
예외이며, 그 차이는 문서화되어 있다.

---

## 2. 인덱스 · 제약 DDL

Cypher DDL 문으로만 진입한다(SQL 함수가 아니다).
구현: [engine/src/compat/ddl.rs:18](../../engine/src/compat/ddl.rs#L18).

### 2.1 지원 구문

| 문 | 결과 |
|---|---|
| `CREATE INDEX [name] [IF NOT EXISTS] FOR (n:Label) ON (n.prop, …)` | `og_create_index` 를 프로퍼티마다 |
| `CREATE TEXT\|RANGE\|POINT INDEX …` | 위와 동일 (`IndexKind::Btree`로 매핑) |
| `CREATE VECTOR INDEX name FOR (n:L) ON (n.p) OPTIONS {indexConfig: {`vector.dimensions`: N, `vector.similarity_function`: 'cosine'}}` | `og_add_embedding` |
| `CREATE FULLTEXT INDEX name FOR (n:L) ON EACH [n.a, n.b]` | GIN + `to_tsvector('simple', …)` |
| `CREATE INDEX … FOR ()-[r:T]-() ON (r.p)` | 관계 인덱스 (`on_relationship = true`) |
| `CREATE CONSTRAINT [name] [IF NOT EXISTS] FOR (n:L) REQUIRE (n.a, n.b) IS UNIQUE \| IS NOT NULL \| IS NODE KEY` | 아래 §2.3 |
| Neo4j 4의 `ASSERT` 형태 | 동일 |
| `DROP INDEX name [IF EXISTS]` / `DROP CONSTRAINT name [IF EXISTS]` | 카탈로그 항목만 삭제 |

**이름 등록**: 모든 항목이 `og_catalog.compat_index`에 기록된다
(`(graph_id, name)` upsert, [ddl.rs:107](../../engine/src/compat/ddl.rs#L107)).
`db.index.*.queryNodes`가 해석하는 이름이 바로 이것이다.

**기본 이름** (이름을 생략했을 때, [ddl.rs:170](../../engine/src/compat/ddl.rs#L170)):
`<kind>_<label>_<prop1>_<prop2>…` — 예: `btree_Person_name`, `constraint_Person_email`.

### 2.2 선언되지 않은 프로퍼티의 자동 선언

인덱스나 제약을 타입이 한 번도 써 본 적 없는 프로퍼티에 거는 것은 **평범한
Cypher**다 — Neo4j에는 선언할 스키마가 없기 때문. 여기엔 있으므로 가는 길에
컬럼을 만든다([ddl.rs:132](../../engine/src/compat/ddl.rs#L132) `ensure_property`).
타입은 `'string'`으로 선언된다.

이미 쓰여 있던 값은 `og_add_property`가 `__ext`에서 컬럼으로 옮긴다
([01_graph_ddl.md §4](01_graph_ddl.md)) — "먼저 쓰고 나중에 인덱스"가 통상적인
순서이기 때문.

### 2.3 제약 — 무엇이 강제되고 무엇이 안 되는가 (중요)

정의: [engine/src/compat/ddl.rs:284](../../engine/src/compat/ddl.rs#L284) `create_constraint`.

| 제약 | 강제됨? | 방식 |
|---|---|---|
| `IS UNIQUE` | ✅ | `CREATE UNIQUE INDEX IF NOT EXISTS uq_<sub>_<name>` — **프로퍼티 집합 전체에 하나의 인덱스** |
| `IS NODE KEY`의 유일성 절반 | ✅ | 위와 동일 |
| `IS NOT NULL` | ❌ **기록만 되고 강제되지 않음** | 컬럼에 `NOT NULL`을 붙이지 않는다 |
| `IS NODE KEY`의 존재성 절반 | ❌ | 위와 동일 |

**결정(Decision) — 존재성을 강제하지 않는 두 가지 이유**
([ddl.rs:310](../../engine/src/compat/ddl.rs#L310) 주석, 둘 다 관측된 근거):

1. PostgreSQL은 `NOT NULL`을 **문 단위로** 검사하고 Neo4j는 **커밋 시점에** 검사한다.
   `MERGE (t:Table {name, schema})` 다음에 같은 트랜잭션에서 `SET t.db = …`를
   하는 것은 Neo4j에서 합법이고 여기서는 실패한다.
2. `IS NODE KEY`는 Neo4j **Enterprise** 기능이다. Community에서는 그 문이 실패하고
   애플리케이션은 그냥 진행한다. 여기서 강제하면 애플리케이션이 실제로 대상으로
   삼는 데이터베이스보다 **더 엄격**해진다.

유일성은 쓰기 시점에 검사 가능하고 `MERGE`의 멱등성이 그것에 의존하므로 강제한다.

**`REQUIRE (a, b, c) IS NODE KEY`는 조합 하나에 대한 유일 인덱스**다.
프로퍼티마다 유일 인덱스를 만드는 것은 훨씬 강한 규칙이 되고, 그것은 이 제약이
허용하려는 통상적 경우(다른 스키마의 같은 이름 테이블)를 거부한다
([ddl.rs:150](../../engine/src/compat/ddl.rs#L150)).

### 2.4 전문 검색 인덱스는 **동등하지 않다**

`build_fulltext` ([ddl.rs:259](../../engine/src/compat/ddl.rs#L259))는
`to_tsvector('simple', coalesce(col1::text,'') || ' ' || …)` 위에 GIN 인덱스를 만든다.

PostgreSQL의 `simple` 사전은 **어간 추출도, CJK 분절도 하지 않는다.**
재현율이 Neo4j의 Lucene 인덱스와 다르며, 한국어에서 가장 눈에 띈다.
숨기지 않고 문서화한 차이다([ddl.rs:255](../../engine/src/compat/ddl.rs#L255),
[docs/cypher.md:260](../../docs/cypher.md)).

### 2.5 `DROP INDEX`의 의미

카탈로그 항목이 사라진다. **밑에 있는 컬럼과 그 인덱스는 남는다**
([ddl.rs:336](../../engine/src/compat/ddl.rs#L336)). 선언된 프로퍼티를 드롭하면 그
데이터가 사라지는데, `DROP INDEX`는 결코 그런 뜻이 아니기 때문.

> ⚠️ 결과적으로 `DROP INDEX` 후에도 물리 인덱스가 남아 쓰기 비용을 계속 낸다
> → [12_improvements_api.md](12_improvements_api.md) **API-26**.

### 2.6 예제

```sql
SELECT og_cypher('default',
  'CREATE INDEX person_name IF NOT EXISTS FOR (p:Person) ON (p.name)');

SELECT og_cypher('default',
  'CREATE CONSTRAINT person_email IF NOT EXISTS FOR (p:Person) REQUIRE p.email IS UNIQUE');

SELECT og_cypher('default', $$
  CREATE VECTOR INDEX room_name FOR (r:MeetingRoom) ON (r.name_vec)
  OPTIONS {indexConfig: {`vector.dimensions`: 1024,
                         `vector.similarity_function`: 'cosine'}}
$$);

SELECT name, kind, type_name, props FROM og_catalog.compat_index;
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 같은 이름 존재 + `IF NOT EXISTS` 없음 | `an index named '<n>' already exists in graph '<g>'` ([ddl.rs:197](../../engine/src/compat/ddl.rs#L197)) |
| 제약 이름 중복 | `a constraint named '<n>' already exists in graph '<g>'` ([ddl.rs:300](../../engine/src/compat/ddl.rs#L300)) |
| `DROP` 대상 없음 + `IF EXISTS` 없음 | `no <index\|constraint> named '<n>' in graph '<g>'` ([ddl.rs:334](../../engine/src/compat/ddl.rs#L334)) |
| VECTOR INDEX에 차원 미지정 | ``CREATE VECTOR INDEX needs OPTIONS {indexConfig: {`vector.dimensions`: N}}`` ([ddl.rs:216](../../engine/src/compat/ddl.rs#L216)) |
| VECTOR INDEX에 프로퍼티 없음 | `a vector index needs one property` ([ddl.rs:224](../../engine/src/compat/ddl.rs#L224)) |

---

## 3. 프로시저 — `CALL … YIELD`

**결정(Decision)**: 프로시저는 해석되지 않고 **계획된다**. 각각은 컴파일러가
`FROM`에 넣는 릴레이션이 되므로 `CALL … YIELD`가 다른 것과 똑같이 조인되고
플래너가 비용을 매긴다. Rust에서 행 단위로 실행되는 것은 아무것도 없다
([engine/src/compat/procs.rs:3](../../engine/src/compat/procs.rs#L3)).

**레지스트리는 닫혀 있다.** 알 수 없는 프로시저는 이름으로 거부된다 —
`apoc.something.exotic`을 부르는 애플리케이션은 "매치 없음"처럼 보이는 빈 결과가
아니라 그 사실을 들어야 한다.

### 3.1 지원 목록 (전체)

정의: [engine/src/compat/procs.rs:80](../../engine/src/compat/procs.rs#L80) `plan()`

| 프로시저 | YIELD 컬럼 | 도달 지점 |
|---|---|---|
| `db.index.vector.queryNodes(indexName, k, vector)` | `node`, `score` | `og_vector_search` |
| `db.index.fulltext.queryNodes(indexName, query)` | `node`, `score` | `ts_rank` + `websearch_to_tsquery('simple', …)` |
| `apoc.neighbors.tohop(node, relFilter, distance)` | `node`, `depth` | `og_vlp(src, NULL, dir, 1, hops)` |
| `apoc.neighbors.tohop.count(...)` | 동일 | 동일 |
| `apoc.meta.schema({sample: n})` | `value` | `og_apoc_meta_schema` |
| `db.labels()` | `label` | `og_catalog.type WHERE kind = 'e'` |
| `db.relationshipTypes()` | `relationshipType` | `og_catalog.type WHERE kind = 'r'` |
| `db.propertyKeys()` | `propertyKey` | `og_catalog.property` DISTINCT |
| `dbms.components()` | `name`, `versions`, `edition` | `('Ontological', [ontological_version()], 'community')` |

### 3.2 무연산 프로시저

정의: [engine/src/compat/procs.rs:72](../../engine/src/compat/procs.rs#L72) `NO_OPS`

`db.awaitIndex`, `db.awaitIndexes`,
`db.index.fulltext.awaitEventuallyConsistentIndexRefresh`,
`db.clearQueryCaches`, `db.resamplIndex`

드라이버 시작 시퀀스를 성공시키기 위해서만 존재한다. 기다릴 것이 없다 —
여기 인덱스는 동기적으로 만들어진다. YIELD 컬럼이 없다.

> ℹ️ `db.resamplIndex`는 Neo4j의 `db.resampleIndex` 오타로 보인다
> (`NO_OPS` 목록은 소문자 비교이므로 `db.resampleIndex`는 **매치되지 않는다**)
> → [12_improvements_api.md](12_improvements_api.md) **API-27**.

### 3.3 인덱스 이름은 반드시 리터럴

`db.index.vector.queryNodes` / `db.index.fulltext.queryNodes`의 첫 인자는 **리터럴
문자열**이어야 한다. 컴파일 시점에 해석되므로 파라미터가 이름을 지정할 수 없다.

```
ERROR:  db.index.vector.queryNodes needs the index name as a literal string —
        it is resolved when the query is compiled, so a parameter cannot name it
```

([procs.rs:168](../../engine/src/compat/procs.rs#L168))

없는 인덱스를 부르면 알려진 이름 목록이 함께 나온다
([procs.rs:275](../../engine/src/compat/procs.rs#L275)):

```
ERROR:  there is no index named 'rooms'. known indexes: room_name, person_name
ERROR:  there is no index named 'rooms'; none have been created in this graph
```

종류가 다르면:
```
ERROR:  index 'person_name' is a btree index, not a vector index
```
([procs.rs:174](../../engine/src/compat/procs.rs#L174))

### 3.4 `db.index.vector.queryNodes`의 벡터 인자

질의 벡터는 표현식이 만든 무엇이든 도착한다 — 파라미터는 jsonb 배열,
`genai.vector.encode`는 `float8[]`, 리터럴은 이미 텍스트. 텍스트 표현의 차이는
PostgreSQL이 배열에 붙이는 중괄호뿐이고 pgvector는 대괄호를 원하므로,
`translate((expr)::text, '{}', '[]')` 로 셋 다 받는다
([procs.rs:187](../../engine/src/compat/procs.rs#L187)).

### 3.5 `apoc.neighbors.tohop`의 relFilter

APOC의 관계 필터는 작은 언어인데, 여기서 걸음을 바꾸는 부분은 **방향 표시자**뿐이다
([procs.rs:250](../../engine/src/compat/procs.rs#L250)):

| 필터 문자열 | 방향 |
|---|---|
| `<`로 시작 | `'i'` |
| `>`를 포함 | `'o'` |
| 그 외 | `'b'` |

> ⚠️ 주석은 "타입 이름을 담은 필터도 수용되고 그 타입 이름이 존중된다"고 말하지만
> ([procs.rs:251](../../engine/src/compat/procs.rs#L251)), 코드는 `og_vlp`의 `etypes`
> 인자에 `NULL::int4[]`을 넘긴다([procs.rs:264](../../engine/src/compat/procs.rs#L264)).
> **타입 필터는 실제로 무시된다** → [12_improvements_api.md](12_improvements_api.md) **API-27**.

### 3.6 `og_apoc_meta_schema(graph text, sample int4 DEFAULT 1000) RETURNS jsonb`

정의: [engine/src/compat/meta.rs:184](../../engine/src/compat/meta.rs#L184) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: APOC의 형태로 스키마를 반환한다. 다만 여기서는 스키마가
**선언되어 있으므로** 대부분 샘플링하지 않는다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | `text` | 필수 | — | 그래프 이름 |
| `sample` | `int4` | 선택 | `1000` | **역할이 선언되지 않은 관계 타입의 끝점 관측 표본 크기만** 제한 |

**반환**: 라벨 이름을 키로 하는 객체.

| 노드 항목 | 관계 항목 |
|---|---|
| `{type: "node", count, labels: [], properties, relationships: {…}}` | `{type: "relationship", count, properties}` |

`relationships` 슬롯: `{direction: "out"\|"in", count, labels: [], properties}`.

**정확성의 근거**: `count`는 추정이 아니라 실제 카운트, 프로퍼티 타입은 카탈로그가
가진 것, 관계 방향은 **선언된 역할**에서 온다. 예외는 Neo4j 방식으로 만들어진
관계 타입(`(a)-[:LINKS]->(b)`을 선언 없이 쓴 것) — 역할이 없으므로 끝점을 엣지에서
읽고 `sample`로 제한한다. 그 쌍들은 APOC식 답이고 APOC의 단서를 함께 진다
([docs/cypher.md:148](../../docs/cypher.md)).

**예제**

```sql
SELECT jsonb_pretty(og_apoc_meta_schema('meeting'));
SELECT og_cypher('meeting', 'CALL apoc.meta.schema({sample: 500}) YIELD value RETURN value');
```

### 3.7 알 수 없는 프로시저

```
ERROR:  procedure 'apoc.path.expand' is not available. supported:
        db.index.vector.queryNodes, db.index.fulltext.queryNodes,
        apoc.meta.schema, apoc.neighbors.tohop, db.labels,
        db.relationshipTypes, db.propertyKeys, dbms.components
```

([procs.rs:154](../../engine/src/compat/procs.rs#L154))

> ⚠️ 이 목록에 무연산 프로시저(`db.awaitIndex` 등)와
> `apoc.neighbors.tohop.count`가 빠져 있다 → API-09.

---

## 4. `genai.vector.encode`

### `og_genai_encode(resource text, provider text DEFAULT NULL, configuration jsonb DEFAULT '{}') RETURNS float8[]`

정의: [engine/src/compat/genai.rs:95](../../engine/src/compat/genai.rs#L95) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: 텍스트를 임베딩 벡터로 바꾼다. **이 확장의 유일한 외부 네트워크 호출**이다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `resource` | `text` | 필수 | — | 인코딩할 텍스트 |
| `provider` | `text` | 선택 | `NULL` | `ollama` / `openai` / `azureopenai` (대소문자 무관). `NULL`이면 `genai.provider` 설정, 그것도 없으면 `ollama` |
| `configuration` | `jsonb` | 선택 | `'{}'` | `model`, `dimensions` 키만 인식. **엔드포인트나 토큰은 받지 않는다** |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `float8[]` | 아니오 | 잘라내기 후 **L2 재정규화된** 벡터 |

**결정(Decision) — 세 가지 안전장치** ([engine/src/compat/genai.rs:13](../../engine/src/compat/genai.rs#L13))

1. **명시적으로 켜기 전까지 꺼져 있다.** `genai.enabled`가 `'on'`이어야 한다.
2. **엔드포인트는 설정이지 인자가 아니다.** Neo4j는 호출이 자기 엔드포인트를
   지정하게 한다. 여기서는 불가능하다 — URL은 `genai.endpoint`에서 온다.
   **질의 권한은 fetch 권한이 아니다.**
3. **제한이 있다.** `genai.timeout_ms`가 대기를 제한하며 기본값은 짧다(5000ms).

**설정 키 (`og_catalog.setting`, `og_set_setting`으로 씀)**

| 키 | 필수 | 기본값 | 설명 |
|---|---|---|---|
| `genai.enabled` | ✅ | 없음(꺼짐) | `'on'` 이어야 동작 |
| `genai.endpoint` | ✅ | 없음 | 임베딩 HTTP 엔드포인트 URL |
| `genai.provider` | 선택 | `ollama` | 요청/응답 형태 결정 |
| `genai.model` | ✅ (또는 `configuration.model`) | 없음 | 모델 이름 |
| `genai.dimensions` | 선택 | 없음 | 잘라낼 차원 수 |
| `genai.token` | 선택 | 없음 | `Authorization: Bearer <token>` |
| `genai.timeout_ms` | 선택 | `5000` | 요청 타임아웃 |

**요청/응답 형태** ([genai.rs:70](../../engine/src/compat/genai.rs#L70), [:77](../../engine/src/compat/genai.rs#L77))

| provider | 요청 본문 | 응답에서 벡터 위치 |
|---|---|---|
| `ollama` | `{"model": …, "input": …}` | `embeddings[0]` |
| `openai` / `azureopenai` | `{"model": …, "input": …}` | `data[0].embedding` |

**차원 잘라내기 + 재정규화** ([genai.rs:160](../../engine/src/compat/genai.rs#L160)):
`dimensions`가 주어지고 벡터보다 짧으면 **앞부분을 자른 뒤** L2 정규화한다.
Matryoshka 학습 모델의 접두사는 유효한 더 작은 임베딩이므로 정당하고,
**pgvector HNSW 인덱스가 2000차원에서 멈추기 때문에** 4096차원 모델에는 필요하다.
자른 뒤 정규화하는 것이 코사인 거리를 유의미하게 유지한다.

**설정 예제** ([docs/cypher.md:182](../../docs/cypher.md))

```sql
SELECT og_set_setting('genai.enabled',    'on');
SELECT og_set_setting('genai.endpoint',   'http://localhost:11434/api/embed');
SELECT og_set_setting('genai.provider',   'ollama');   -- or OpenAI-compatible
SELECT og_set_setting('genai.model',      'qwen3-embedding:latest');
SELECT og_set_setting('genai.dimensions', '1024');     -- truncate, then re-normalise
SELECT og_set_setting('genai.token',      '…');        -- optional bearer token
SELECT og_set_setting('genai.timeout_ms', '5000');
```

**Cypher 한 문장으로 시맨틱 검색** ([examples/meeting-rooms/README.md:107](../../examples/meeting-rooms/README.md)):

```cypher
CALL db.index.vector.queryNodes('room_name', 3, genai.vector.encode($text))
YIELD node, score RETURN node.name, score
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| 비활성 | `genai.vector.encode is disabled. It makes an outbound HTTP request from the database, so it is off until that is chosen deliberately: SELECT og_set_setting('genai.enabled', 'on')` ([genai.rs:102](../../engine/src/compat/genai.rs#L102)) |
| 엔드포인트 미설정 | `no embedding endpoint is configured. The URL is deliberately not an argument — set it with og_set_setting('genai.endpoint', '…')` ([genai.rs:109](../../engine/src/compat/genai.rs#L109)) |
| 지원하지 않는 provider | `provider '<p>' is not supported. supported: Ollama, OpenAI, AzureOpenAI (anything speaking the OpenAI /v1/embeddings shape)` ([genai.rs:122](../../engine/src/compat/genai.rs#L122)) |
| 모델 미설정 | `no embedding model configured; set genai.model` ([genai.rs:133](../../engine/src/compat/genai.rs#L133)) |
| 요청 실패/타임아웃 | `embedding request to '<endpoint>' failed: <e>` ([genai.rs:148](../../engine/src/compat/genai.rs#L148)) |
| JSON 아님 | `embedding endpoint returned a body that is not JSON: <e>` ([genai.rs:147](../../engine/src/compat/genai.rs#L147)) |
| 벡터 없음 | `embedding endpoint returned no vector in the shape '<provider>' produces` ([genai.rs:152](../../engine/src/compat/genai.rs#L152)) |

> ⚠️ **PostgreSQL 백엔드가 남의 HTTP 서버에 블로킹된다.** 그 백엔드는 질의에
> 답하지 않는 백엔드다. 프로덕션에서는 타임아웃을 짧게 유지할 것.

---

## 5. Bolt 4.4 게이트웨이

별도 Rust 바이너리(`bolt/`)로 데이터베이스 옆에서 돈다. **자기 상태가 없다** —
파서도, 플래너도, 캐시도, 사용자 저장소도 없다. Cypher는 여기서 결코 해석되지
않는다. 하나의 질의 경로, spec 003의 것뿐
([bolt/src/main.rs:1](../../bolt/src/main.rs#L1)).

### 5.1 실행과 설정 — **환경변수만**

```
ontological-bolt
```

| 환경변수 | 기본값 | 설명 |
|---|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` | 리슨 주소 |
| `OG_BOLT_PGHOST` | `localhost` | PostgreSQL 호스트 |
| `OG_BOLT_PGPORT` | `5432` | PostgreSQL 포트 |
| `OG_BOLT_PGDATABASE` | `og` | 데이터베이스 이름 |
| `OG_BOLT_GRAPH` | `default` | 세션이 데이터베이스를 지정하지 않을 때 쓸 그래프 |
| `OG_BOLT_ADVERTISED` | `OG_BOLT_LISTEN`과 동일 | `ROUTE` 응답에 광고할 주소 |

([bolt/src/main.rs:36](../../bolt/src/main.rs#L36))

### 5.2 프로토콜 계약

| 항목 | 값 | 근거 |
|---|---|---|
| 프로토콜 버전 | **Bolt 4.4 전용** (`0x0000_0404`) | [session.rs:13](../../bolt/src/session.rs#L13) |
| 협상 | 제안된 버전 중 `major == 4 && minor >= 4`(범위 포함)만 수락, 아니면 `0` 응답 | [session.rs:103](../../bolt/src/session.rs#L103) `speaks` |
| 서버 문자열 | `"Neo4j/4.4.0 (ontological-bolt)"` | [session.rs:189](../../bolt/src/session.rs#L189) |
| TLS | **없음** (`NoTls`) | [session.rs:182](../../bolt/src/session.rs#L182) |
| 동시성 | 연결당 스레드 1개, 세션당 PostgreSQL 연결 1개 | [main.rs:70](../../bolt/src/main.rs#L70) |

**지원 메시지** ([session.rs:17](../../bolt/src/session.rs#L17), [:139](../../bolt/src/session.rs#L139))

| 메시지 | 처리 |
|---|---|
| `HELLO` (0x01) | PostgreSQL로 연결. `principal`/`credentials`가 DB 자격증명 |
| `GOODBYE` (0x02) | 연결 종료 |
| `RESET` (0x0F) | 열린 트랜잭션 롤백, 상태 초기화 |
| `RUN` (0x10) | `og_cypher_check` → `og_cypher_columns` → `og_cypher` |
| `BEGIN` (0x11) | `BEGIN` |
| `COMMIT` (0x12) / `ROLLBACK` (0x13) | 해당 SQL |
| `DISCARD` (0x2F) / `PULL` (0x3F) | 레코드 스트리밍 / 폐기 후 요약 |
| `ROUTE` (0x66) | 단일 서버를 WRITE/READ/ROUTE 모두로 광고 (TTL 300) |
| 그 외 | `Neo.ClientError.Request.Invalid` — `unsupported Bolt message 0x<hex>` |

**결정(Decision) — 인증은 PostgreSQL의 것**: `HELLO`의 자격증명이 곧 role의
자격증명이고, 연결 실패가 여기서 "권한 없음"의 뜻이다. **두 번째 사용자 저장소가
없다**(FR-015, [session.rs:165](../../bolt/src/session.rs#L165)).

**결정(Decision) — 세션의 "데이터베이스"는 그래프**(FR-016,
[session.rs:225](../../bolt/src/session.rs#L225)):
- `db`가 `"neo4j"` 또는 `"system"` 또는 비어 있으면 기본 그래프.
- 명시적 트랜잭션 안에서 `db`가 없으면 **트랜잭션이 시작된 그래프**를 유지한다
  (드라이버가 `BEGIN`에서 한 번만 지정하고 이후 `RUN`에서 생략하기 때문).

### 5.3 `EXPLAIN` / `PROFILE`

둘 다 수락된다. 질의는 **파싱·분류되지만 실행되지 않고**, 결과는 비어 있으며
요약의 `type`이 `"r"` 또는 `"w"`로 설정된다([session.rs:257](../../bolt/src/session.rs#L257)).

그 필드가 드라이버측 도구가 실행 전에 읽기/쓰기를 판별하는 수단이고 —
공식 Neo4j MCP 서버가 두 질의 도구 모두를 정확히 그것으로 게이팅한다 —
그래서 키워드 스캔이 아니라 **파서**(`og_cypher_check`)가 답한다.

**계획은 반환하지 않는다.** Neo4j의 `EXPLAIN`은 자기 연산자를 서술하는데, 평범한
SQL이 되는 질의에 그 등가물을 지어내는 것은 허구가 될 것이기 때문. 문장은
`og_cypher_sql()`로, 둘 다는 `og_cypher_explain()`으로 얻는다.
`PROFILE`은 같은 이유로 `EXPLAIN`처럼 취급된다.

### 5.4 요약(summary) 계약

`PULL`의 마지막 응답 메타 ([session.rs:357](../../bolt/src/session.rs#L357)):

| 키 | 언제 | 값 |
|---|---|---|
| `t_last` | 항상 | `0` |
| `has_more` | 남은 행이 있을 때 | `true` |
| `type` | 스트림 종료 시 | `"r"` \| `"w"` |
| `stats` | `type == "w"` 일 때 | `og_cypher_stats()` 결과 |
| `db` | 스트림 종료 시 | 현재 그래프 이름 |
| `bookmark` | 트랜잭션 밖일 때 | 항상 `"ontological:0"` |

> ⚠️ `t_first` / `t_last`가 항상 `0`이고 `bookmark`가 항상 상수다.
> 인과적 일관성(causal consistency)을 북마크로 기대하는 드라이버 기능은 동작하지
> 않는다 → [12_improvements_api.md](12_improvements_api.md) **API-28**.

### 5.5 값 매핑

`og_cypher()`는 노드를 `{_id, _type, …props}`, 관계를 `{_id, _type, _src, _dst, …props}`로
서술한다. 게이트웨이는 이를 Bolt의 `Node`(0x4E) / `Relationship`(0x52) 구조체로
바꾼다([session.rs:495](../../bolt/src/session.rs#L495)).

**필드 순서**는 행이 아니라 파서에서 온다 — jsonb가 키를 정렬하므로 행은 질의가
요구한 순서를 알려줄 수 없다(FR-010). `og_cypher_columns()`가 빈 배열을 주면
(`RETURN *` 등) 첫 행의 키로 폴백한다([session.rs:283](../../bolt/src/session.rs#L283)).

**파라미터**는 jsonb로 넘어간다 — 질의 텍스트에 결코 보간되지 않는다.
주입 보장은 spec 003 FR-026 그대로다([session.rs:546](../../bolt/src/session.rs#L546)).

### 5.6 지원하지 않는 것

| 항목 | 실제 동작 |
|---|---|
| **Bolt 5.x** | 협상 실패 — `0` 응답, 드라이버가 깨끗한 협상 실패로 보고 |
| **TLS / `bolt+s://` / `neo4j+s://`** | 없음(`NoTls`). 평문만 |
| **`Path` 구조체** | Bolt `Path` 타입을 만들지 않는다 |
| **시간/공간 구조체** | 드라이버가 보내면 `"<unsupported struct 0x<sig>>"` **문자열로 변환**되어 전달된다 ([session.rs:559](../../bolt/src/session.rs#L559)) |
| **실제 라우팅 테이블** | 단일 서버를 모든 역할로 광고. 진짜 라우팅은 spec 007 |
| **결과 스트리밍** | `RUN` 시점에 **모든 행을 메모리로 가져온다**([session.rs:294](../../bolt/src/session.rs#L294)). `PULL n`은 이미 가져온 것을 나눠줄 뿐 |

> ⚠️ 시간/공간 값이 조용히 문자열이 되는 것은 "조용히 뒤틀지 않겠다"는 주석의
> 의도와 어긋난다 — 오류가 아니라 대체 값이다
> → [12_improvements_api.md](12_improvements_api.md) **API-28**.

> ⚠️ `RUN`이 전체 결과를 메모리에 적재하므로 큰 결과 집합에서 게이트웨이 메모리가
> 결과 크기에 비례한다.

### 5.7 오류 코드 매핑

Bolt는 PostgreSQL 오류를 Neo4j 코드로 바꾼다. **SQLSTATE가 아니라 메시지
부분 문자열로** 판정한다([session.rs:578](../../bolt/src/session.rs#L578)):

| 메시지에 포함된 문자열 | Neo4j 코드 |
|---|---|
| `not supported` / `expected` / `unknown label` / `is not defined` | `Neo.ClientError.Statement.SyntaxError` |
| `does not exist` | `Neo.ClientError.Database.DatabaseNotFound` |
| `permission denied` | `Neo.ClientError.Security.Forbidden` |
| 그 외 | `Neo.ClientError.Statement.ArgumentError` |

메시지는 **그대로 전달**된다 — 컴파일러 메시지가 구조를 지명하고 대안을 제시하는
가치 있는 부분이고 에이전트가 그것으로 재시도하기 때문(원칙 VIII).

> ⚠️ `graph 'x' does not exist`(그래프 없음)와 `type 'x' does not exist`(타입 없음)가
> 둘 다 `DatabaseNotFound`가 된다. 후자는 데이터베이스 문제가 아니다
> → [11_errors.md](11_errors.md), API-28.

파싱 실패는 `og_cypher_check`가 먼저 잡아 `Neo.ClientError.Statement.SyntaxError`로
보고한다([session.rs:455](../../bolt/src/session.rs#L455)).

---

## 6. 금지 / 필수

- **금지**: `IS NOT NULL` / `IS NODE KEY`의 존재성 절반을 강제된 제약으로 믿지 말 것.
- **금지**: `db.index.*.queryNodes`의 인덱스 이름을 파라미터로 넘기지 말 것 — 리터럴이어야 한다.
- **금지**: `apoc.neighbors.tohop`의 relFilter에 타입 이름을 넣고 필터링을 기대하지 말 것 — 무시된다.
- **금지**: Bolt 게이트웨이를 평문으로 신뢰 경계 밖에 노출하지 말 것 — TLS가 없다.
- **금지**: Bolt로 매우 큰 결과 집합을 스트리밍하려 하지 말 것 — 전부 메모리에 적재된다.
- **필수**: `genai.*`를 켜기 전에 타임아웃을 확인할 것. 백엔드가 블로킹된다.
- **필수**: `genai.dimensions`로 2000 이하로 자를 것 — pgvector HNSW의 한계.
- **필수**: Bolt 드라이버는 4.4 이하를 협상하도록 설정할 것.
- **필수**: `DROP INDEX` 후에도 물리 인덱스가 남는다는 점을 운영에서 감안할 것.

---

## 7. 관련 문서

- Cypher 문법 경계 → [03_cypher.md](03_cypher.md)
- 벡터 검색 네이티브 표면 → [06_vector_search.md](06_vector_search.md)
- 오류 코드 체계 → [11_errors.md](11_errors.md)
- 원문 → [docs/cypher.md](../../docs/cypher.md) "The Neo4j surface"

<!-- affects: api, backend, security -->
<!-- requires-update: 02_api/11_errors.md, 02_api/12_improvements_api.md -->
