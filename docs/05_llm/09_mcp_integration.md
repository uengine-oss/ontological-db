# 09. MCP 연동 — Neo4j MCP 서버를 Bolt 게이트웨이에 붙이기

> **이 문서가 답하는 질문**
> - 전용 MCP 서버가 없는데 어떻게 MCP 클라이언트를 붙이는가?
> - 정확한 설정 파일과 실행 절차는? (복사-붙여넣기)
> - `--database` 는 무엇을 가리키는가? 인증은 어떻게 되는가?
> - 무엇이 동작하고 무엇이 동작하지 않는가?

---

## 1. 사실 — 전용 MCP 서버는 없다. 대신 Neo4j의 것이 그대로 붙는다

spec 008 FR-032는 MCP 호환 서버를 요구하지만, 전용 바이너리는 미구현이다
(T020 미체크, [specs/008-agent-native-interface/tasks.md:37](../../specs/008-agent-native-interface/tasks.md);
[docs/agents.md:158-163](../../docs/agents.md) 도 "not built yet"이라고 명시).

대신 성립하는 경로는 이것이다:

```
MCP 클라이언트  ──stdio──▶  mcp-neo4j-cypher (PyPI, 수정 없음)
                                │ Bolt 4.4
                                ▼
                          ontological-bolt (spec 011)
                                │ postgres wire
                                ▼
                          PostgreSQL + ontological
```

`examples/meeting-rooms/` 전체가 이 경로를 **테스트로** 검증한다
([examples/meeting-rooms/README.md:10-12](../../examples/meeting-rooms/README.md)).
`verify_mcp.py` 는 11개 검사 매트릭스를 출력하고, README는 **11/11 통과**를 보고한다
([README.md:73](../../examples/meeting-rooms/README.md)).

---

## 2. 사실 — 사전 조건

| 필요한 것 | 확인 방법 | 근거 |
|---|---|---|
| PostgreSQL + `ontological` 확장 | `SELECT ontological_version();` | — |
| **Bolt 게이트웨이 실행 중** | `ss -ltn \| grep 7687` | [bolt/src/main.rs:36-57](../../bolt/src/main.rs) |
| Python ≥ 3.10 | `python3 --version` | [examples/meeting-rooms/README.md:66](../../examples/meeting-rooms/README.md) |
| `mcp-neo4j-cypher` | `pip install mcp-neo4j-cypher` | 같은 파일 66행 |
| (선택) 임베딩 모델 | Ollama 등 | 같은 파일 61-63행 |

### 2.1 Bolt 게이트웨이 실행

환경변수 전용이다 ([bolt/src/main.rs:9-15, 37-44](../../bolt/src/main.rs)):

| 변수 | 기본값 |
|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` |
| `OG_BOLT_PGHOST` | `localhost` |
| `OG_BOLT_PGPORT` | `5432` |
| `OG_BOLT_PGDATABASE` | `og` |
| `OG_BOLT_GRAPH` | `default` — 세션이 database를 지정하지 않았을 때 쓰는 그래프 |
| `OG_BOLT_ADVERTISED` | `OG_BOLT_LISTEN` 과 동일 |

```bash
OG_BOLT_PGHOST=localhost \
OG_BOLT_PGPORT=5432 \
OG_BOLT_PGDATABASE=ogstudio \
OG_BOLT_GRAPH=meeting \
./bolt/target/release/ontological-bolt
```

`start.sh` 가 자동으로 띄우는 구성도 있다
([start.sh:54-65](../../start.sh)) — 호스트 포트는 `OG_BOLTPORT`(기본 `28687`)이고
컨테이너 내부 7687로 매핑된다([start.sh:9, 27](../../start.sh)).
`OG_BOLT=0` 이면 게이트웨이를 띄우지 않는다([start.sh:56](../../start.sh)).

### 2.2 인증은 PostgreSQL의 것이다

```rust
// bolt/src/session.rs:165-184
// Authentication is PostgreSQL's: the credentials in HELLO are the role's
// credentials, and failing to connect is what "unauthorized" means here.
let user     = extra["principal"];
let password = extra["credentials"];
cfg.host(...).port(...).dbname(...).user(&user).application_name("ontological-bolt");
```

MCP 설정의 `--username` / `--password` 는 **PostgreSQL 역할의 자격증명**이다.
별도 사용자 저장소가 없다(spec 011 FR-015). 연결 실패는
`Neo.ClientError.Security.Unauthorized` 로 번역된다
([session.rs:182-184](../../bolt/src/session.rs)).

### 2.3 `--database` 는 그래프 이름이다

```rust
// bolt/src/session.rs:221-238
fn select_graph(&mut self, extra: &Value) {
    if let Some(db) = extra["db"] {
        // "neo4j"와 "system"은 드라이버가 "지정 안 함"을 뜻할 때 보내는 값
        if !db.is_empty() && db != "neo4j" && db != "system" { self.graph = db; return; }
    }
    if self.in_tx { return; }             // 트랜잭션 안에서는 BEGIN 때의 그래프 유지
    self.graph = self.config.default_graph.clone();
}
```

- `--database meeting` → 그래프 `meeting`.
- `neo4j` / `system` 은 "기본 그래프"로 해석된다 → `OG_BOLT_GRAPH` 값.
- **PostgreSQL 데이터베이스가 아니다.** 그것은 `OG_BOLT_PGDATABASE` 로 고정된다.

---

## 3. 절차 (복사-붙여넣기)

### 3.1 온톨로지 선언 (SQL — 이 단계만 SQL이다)

이유: 타입·프로퍼티 타입·**role**(관계 양 끝의 이름 붙은 타입 제약)은 Neo4j에 대응
문법이 없다([examples/meeting-rooms/README.md:81-93](../../examples/meeting-rooms/README.md)).

```bash
psql -d ogstudio -f examples/meeting-rooms/schema.sql
```

내용 요약 ([examples/meeting-rooms/schema.sql](../../examples/meeting-rooms/schema.sql)):

```sql
SELECT og_create_graph('meeting');

SELECT og_create_type('meeting', 'MeetingRoom', 'entity');
SELECT og_add_property('meeting', 'MeetingRoom', 'name', 'string', true, true);
-- …

SELECT og_create_type('meeting', 'FOR_ROOM', 'relation');
SELECT og_add_role('meeting', 'FOR_ROOM', 'reservation', 'Reservation', 0);  -- ordinal 0 = source
SELECT og_add_role('meeting', 'FOR_ROOM', 'room',        'MeetingRoom', 1);  -- ordinal 1 = target
```

role의 `ordinal` 이 `og_schema` 의 `position` (`source`/`target`)으로 노출되고
([engine/src/agent/mod.rs:170-174](../../engine/src/agent/mod.rs)),
`apoc.meta.schema()` 의 관계 방향도 여기서 나온다
([examples/meeting-rooms/README.md:89-93](../../examples/meeting-rooms/README.md)).

### 3.2 MCP 클라이언트 설정 파일

[examples/meeting-rooms/mcp.json](../../examples/meeting-rooms/mcp.json) 을 그대로 쓴다.
Claude Code면 프로젝트 루트에 `.mcp.json` 으로, Claude Desktop이면
`claude_desktop_config.json` 의 `mcpServers` 에 병합한다.

```json
{
  "mcpServers": {
    "ontological": {
      "command": "uvx",
      "args": [
        "mcp-neo4j-cypher",
        "--db-url", "bolt://localhost:7687",
        "--username", "dev",
        "--password", "dev",
        "--database", "meeting",
        "--schema-sample-size", "1000"
      ]
    }
  }
}
```

**`--schema-sample-size` 는 선택이 아니다.** `mcp-neo4j-cypher` 의 help는 기본값
1000이라고 하지만 argparse 기본값이 `None` 이고, `None` 이 그대로
`apoc.meta.schema({sample: None})` 로 도달해 `None` 이 변수 이름으로 파싱된다.
**Neo4j 상대로도 똑같이 실패하는 업스트림 버그**이며, 플래그를 넘기는 것이 두 DB
모두에서의 회피책이다
([examples/meeting-rooms/mcp.json:14-17](../../examples/meeting-rooms/mcp.json),
[README.md:75-79](../../examples/meeting-rooms/README.md),
[og_mcp.py:41-47](../../examples/meeting-rooms/og_mcp.py)).

`uvx` 대신 `pip install` 한 실행 파일을 쓰려면 `"command": "mcp-neo4j-cypher"` 로
바꾸고 `args` 에서 첫 항목을 제거한다
([og_mcp.py:34-50](../../examples/meeting-rooms/og_mcp.py) 가 그 형태다).

### 3.3 데이터 적재와 검증

```bash
pip install mcp-neo4j-cypher            # 진짜 Neo4j 서버, 수정 없음
psql -d ogstudio -f examples/meeting-rooms/schema.sql
OG_GRAPH=meeting python3 examples/meeting-rooms/load.py        # 모든 쓰기를 MCP 서버로
OG_GRAPH=meeting python3 examples/meeting-rooms/verify_mcp.py  # 호환성 매트릭스
OG_GRAPH=meeting python3 examples/meeting-rooms/scenario.py    # 질문 종단 실행
```

`og_mcp.py` 가 읽는 환경변수
([examples/meeting-rooms/og_mcp.py:21-28](../../examples/meeting-rooms/og_mcp.py)):

| 변수 | 기본값 |
|---|---|
| `OG_BOLT_URI` | `bolt://localhost:7687` |
| `OG_BOLT_USER` | `dev` |
| `OG_BOLT_PASSWORD` | `dev` |
| `OG_GRAPH` | `meeting` |
| `OG_EMBED_URL` | `http://localhost:11434/api/embed` |
| `OG_EMBED_MODEL` | `qwen3-embedding:latest` |
| `OG_EMBED_DIMS` | `1024` |

### 3.4 벡터 인덱스는 Neo4j DDL로 만든다

```cypher
CREATE VECTOR INDEX room_name IF NOT EXISTS
FOR (m:MeetingRoom) ON (m.name_vec)
OPTIONS {indexConfig: {
  `vector.dimensions`: 1024,
  `vector.similarity_function`: 'cosine'
}}
```

`write_neo4j_cypher` 로 보내면 된다
([examples/meeting-rooms/load.py:48-65](../../examples/meeting-rooms/load.py)).
내부에서는 `og_add_embedding(graph, label, prop, dims, metric)` 로 변환된다
([engine/src/compat/ddl.rs:211-230](../../engine/src/compat/ddl.rs)).

**⚠️ 이 경로는 `source_prop` 를 넘기지 않는다** → 이 슬롯은
`og_stale_embeddings` 에서 영원히 제외된다. 상세는
[05_embedding_pipeline.md](05_embedding_pipeline.md) 2.3절.

### 3.5 DB 안에서 임베딩하기 (선택)

이걸 켜야 에이전트가 **한 문장으로** 시맨틱 조회를 할 수 있다.
`mcp-neo4j-cypher` 가 노출하는 도구 3개 중에 임베딩 도구가 없기 때문이다
([examples/meeting-rooms/README.md:95-104](../../examples/meeting-rooms/README.md)).

```sql
SELECT og_set_setting('genai.enabled',    'on');
SELECT og_set_setting('genai.endpoint',   'http://localhost:11434/api/embed');
SELECT og_set_setting('genai.provider',   'ollama');   -- 또는 OpenAI 호환
SELECT og_set_setting('genai.model',      'qwen3-embedding:latest');
SELECT og_set_setting('genai.dimensions', '1024');
```

그러면 이 한 문장이 성립한다:

```cypher
CALL db.index.vector.queryNodes('room_name', 3, genai.vector.encode($text))
YIELD node, score
RETURN node.name AS canonical, score
```

`scenario.py` 는 이 형태를 먼저 시도하고, DB가 거절하면 클라이언트 임베딩으로
폴백하며 **어느 경로를 탔는지 출력한다**
([examples/meeting-rooms/scenario.py:98-106](../../examples/meeting-rooms/scenario.py)).

보안 주의: 엔드포인트는 의도적으로 **인자가 아니라 설정**이다 — Cypher를 쓸 수 있는
호출자가 서버로 하여금 임의 URL을 가져오게 만들 수 없다
([engine/src/compat/genai.rs:21-24](../../engine/src/compat/genai.rs)).
다만 평문 SQL로 `og_set_setting` 을 호출할 수 있으면 이 보증은 우회된다
([05_embedding_pipeline.md](05_embedding_pipeline.md) 6절).

---

## 4. 사실 — `verify_mcp.py` 가 검증하는 11개 항목

[examples/meeting-rooms/verify_mcp.py:44-133](../../examples/meeting-rooms/verify_mcp.py)

| # | 검사 |
|---|---|
| 1 | MCP 핸드셰이크 + 도구 발견 |
| 2 | `get_neo4j_schema` (`apoc.meta.schema`) |
| 3 | … 관계 **방향**을 보고하는가 |
| 4 | `read_neo4j_cypher` (`EXPLAIN` 이 `'r'` 을 보고) |
| 5 | 읽기 도구가 쓰기를 거부하는가 |
| 6 | 쓰기 도구가 읽기를 거부하는가 |
| 7 | 파라미터가 바인딩되는가 (보간 아님) |
| 8 | `db.index.vector.queryNodes` + score |
| 9 | 구문 오류가 삼켜지지 않고 보고되는가 |
| 10 | `genai.vector.encode` (DB 안에서 임베딩) |
| 11 | `write_neo4j_cypher` 가 변경 카운터를 반환하는가 |

11번은 생성과 삭제 **양쪽**을 확인한다 — 한쪽만 배선된 카운터도 단일 호출에서는
정상으로 보이기 때문이다([verify_mcp.py:147-160](../../examples/meeting-rooms/verify_mcp.py)).

---

## 5. 사실 — 지원되는 Neo4j 프로시저

[engine/src/compat/procs.rs:154-159](../../engine/src/compat/procs.rs) 의 오류 메시지가
목록 그 자체다:

```
db.index.vector.queryNodes, db.index.fulltext.queryNodes,
apoc.meta.schema, apoc.neighbors.tohop, db.labels, db.relationshipTypes,
db.propertyKeys, dbms.components
```

`db.index.vector.queryNodes(indexName, k, vector)` 의 제약
([procs.rs:163-201](../../engine/src/compat/procs.rs)):

- `indexName` 은 **리터럴 문자열**이어야 한다. 컴파일 시점에 해소되므로 파라미터로
  줄 수 없다([procs.rs:168-171](../../engine/src/compat/procs.rs)).
- 인덱스 이름은 `og_catalog.compat_index` 에서 조회된다. 없으면 알려진 인덱스 목록이
  오류에 포함된다([procs.rs:275-291](../../engine/src/compat/procs.rs)).
- 질의 벡터는 파라미터(jsonb 배열) / `genai.vector.encode` 출력(float8[]) / 리터럴
  텍스트 셋 다 받는다 — `translate(x::text, '{}', '[]')` 로 통일된다
  ([procs.rs:182-187](../../engine/src/compat/procs.rs)).
- **`filter` 를 넘기지 않는다** ([procs.rs:188-194](../../engine/src/compat/procs.rs)) —
  이것이 Bolt 경로에서 SQL 인젝션이 불가능한 이유다
  ([06_retrieval_and_rrf.md](06_retrieval_and_rrf.md) 1.3절).

`db.index.fulltext.queryNodes` 는 PostgreSQL의 `simple` 사전을 쓴다 — 어간 분석도
CJK 분절도 하지 않으므로 **한국어 재현율이 Neo4j와 다르다**. 문서화된 차이다
([engine/src/compat/ddl.rs:253-258](../../engine/src/compat/ddl.rs)).

---

## 6. 사실 — 알려진 한계

| 한계 | 근거 |
|---|---|
| Bolt **5.x 미지원**, Path 타입 미지원, TLS 미지원 | README 스펙 상태표 (011 항목) |
| 서버 문자열은 `Neo4j/4.4.0 (ontological-bolt)` 고정 | [bolt/src/session.rs:189](../../bolt/src/session.rs) |
| MCP 연결에 역할·리소스 상한이 자동 적용되지 않음 (FR-033) | `og_apply_role` 을 호출하는 코드가 게이트웨이에 없다 ([08_guardrails_and_roles.md](08_guardrails_and_roles.md) 2절) |
| Cypher `UNION` 미구현 | README 스펙 상태표 (003 항목) |
| 오타 레이블의 교정 후보가 MCP 응답에 오는지 미확인 | PostgreSQL NOTICE로만 나가며, Bolt notification 매핑을 확인하는 테스트가 없다 ([03_correctable_errors.md](03_correctable_errors.md) 1.2절) |

---

## 7. 필수(Required) / 금지(Forbidden)

**필수**

- `--schema-sample-size` 를 **반드시** 넘길 것 (3.2절).
- `--database` 에는 **그래프 이름**을 넣을 것. PostgreSQL 데이터베이스는
  `OG_BOLT_PGDATABASE` 로 지정한다 (2.3절).
- MCP 세션의 `--username`/`--password` 는 PostgreSQL 역할이므로, **에이전트 전용 역할**을
  만들고 권한을 좁힐 것 ([08_guardrails_and_roles.md](08_guardrails_and_roles.md) 6.1절).
- `og_add_role` 로 관계의 양 끝을 선언할 것. 이것이 `apoc.meta.schema` 의 방향 정보이자
  생성 Cypher의 화살표 오류를 줄이는 근거다 (3.1절).

**금지**

- 그래프 이름을 `neo4j` 또는 `system` 으로 짓지 말 것 — 드라이버의 "지정 안 함"과
  구분되지 않는다 ([session.rs:224-229](../../bolt/src/session.rs)).
- Bolt 게이트웨이를 TLS 없이 신뢰 경계 밖에 노출하지 말 것. 자격증명이 평문으로 오간다.
- `db.index.vector.queryNodes` 의 인덱스 이름을 파라미터로 전달하지 말 것 (5절).
- `CREATE VECTOR INDEX` 로 만든 슬롯에 staleness 추적을 기대하지 말 것 (3.4절).

---

## 8. 참고

- 예제 전체: [examples/meeting-rooms/README.md](../../examples/meeting-rooms/README.md)
- 설정 파일: [examples/meeting-rooms/mcp.json](../../examples/meeting-rooms/mcp.json)
- 게이트웨이: [bolt/src/main.rs](../../bolt/src/main.rs), [bolt/src/session.rs](../../bolt/src/session.rs)
- 프로시저 호환: [engine/src/compat/procs.rs](../../engine/src/compat/procs.rs)
- 스펙: FR-032~FR-034
  ([specs/008-agent-native-interface/spec.md:276-281](../../specs/008-agent-native-interface/spec.md))

<!-- affects: llm, api, ops -->
<!-- requires-update: 02_api/00_index.md -->
