# API 계약 규칙 — 프런트가 무엇에 의존하는가

> **이 문서가 답하는 질문**
> - 각 엔드포인트는 정확히 무엇을 받고 무엇을 돌려주나?
> - 어떤 필드가 `null`일 수 있나?
> - **이 필드를 바꾸면 프런트의 어디가 깨지나?**
> - 엔진의 SQL 함수를 고칠 때 무엇을 함께 확인해야 하나?

---

## 0. 이 문서를 읽어야 하는 사람

`engine/src/` 아래의 다음 함수를 고치는 사람은 **반드시** 이 문서의 §4 표를 확인한다.

`og_cypher` · `og_cypher_sql` · `og_cypher_explain` · `og_schema` · `og_graph_stats` ·
`og_explain_error` · `og_diagnose_empty` · `og_estimate` · `og_node_json` · `og_edge_json` ·
`og_expand` · `ontological_version`

그리고 `bench/harness.py`의 출력 스키마를 바꾸는 사람은 [05_benchmark_report.md](05_benchmark_report.md)를 본다.

---

## 1. 공통 계약

### 1.1 오류 봉투 (Error envelope)

모든 PostgreSQL 오류는 이 형태로 나간다:

```js
// portal/server/index.js:67-75
function pgError(e) {
  return {
    error: e.message,
    code: e.code || null,
    detail: e.detail || null,
    hint: e.hint || null,
    where: e.where || null,
  };
}
```

| 필드 | 타입 | 널 | 프런트가 쓰는가 |
|---|---|---|---|
| `error` | string | 아니오 | **예** — `errorView`가 이것만 표시 (`app.js:283-284`) |
| `code` | string | **예** | 아니오 (표시 안 함) |
| `detail` | string | **예** | 아니오 |
| `hint` | string | **예** | 아니오 |
| `where` | string | **예** | 아니오 |

**중요한 사실**: 프런트는 `code`/`detail`/`hint`/`where`를 **하나도 렌더하지 않는다.**
`errorView`(`app.js:280-310`)는 `err.error` 문자열만 보고, 그 안에서 정규식으로
교정 후보를 뽑는다:

```js
// portal/web/app.js:287
const m = /did you mean:\s*(.+?)\.?$/m.exec(msg);
// portal/web/app.js:289
const wrong = /'([^']+)'/.exec(msg)?.[1];
```

> **계약**: 엔진이 만드는 오류 메시지에서 `did you mean: A, B, C` 형식과
> `'<잘못된 식별자>'` 작은따옴표 표기를 **바꾸면 안 된다.** Studio의 원클릭 교정 기능이
> 이 두 정규식에만 의존한다. 구조화된 필드(`suggestions` 배열 등)가 있어도 프런트는 안 본다.

### 1.2 HTTP 상태코드

| 코드 | 언제 | 프런트 반응 |
|---|---|---|
| 200 | 성공 | 정상 렌더 |
| 400 | PG 오류 / 빈 질의 | `ok: false` → `errorView` 또는 조용한 return |
| **503** | `/api/health` 실패 시에만 (`index.js:163`) | `#conn-dot`에서 `.ok` 제거 + "not connected" |
| 500 | 라우트 핸들러가 던졌을 때 (`index.js:352`), `/api/benchmark` 실패 (`index.js:146`) | `errorView` |

`api()`는 `res.ok`(2xx)만 본다 (`app.js:46`). 400과 500을 구분하지 않는다.

### 1.3 요청 파싱

```js
// portal/server/index.js:48-64
function readBody(req) { ... data.length > 4e6 → reject('request too large') ... JSON.parse ... }
```

- **`content-type`을 검사하지 않는다.** 무슨 타입으로 보내든 body를 `JSON.parse`한다.
  이것이 CSRF를 가능하게 한다 (→ `FE-17`).
- 본문 상한 4 MB. 초과 시 reject되지만 스트림을 destroy하지는 않는다.
- 빈 본문은 `{}`로 처리 (`index.js:57`).

---

## 2. 엔드포인트 계약

### 2.1 `GET /api/health`

**요청**: 없음.

**응답 200**:
```json
{
  "ok": true,
  "version": "<ontological_version()>",
  "database": "<current_database()>",
  "server": "<version()>",
  "graphs": [ { "name": "default", "graph_id": 1 } ]
}
```

**응답 503**: `{ "ok": false, ...pgError }`

| 필드 | 널 | 프런트 사용처 | 바꾸면 깨지는 곳 |
|---|---|---|---|
| `ok` | 아니오 | `app.js:53-54` | `#conn-dot` 색이 항상 빨강 |
| `version` | 아니오 | `app.js:67` | 사이드바 `engine ontological <ver>` |
| `database` | 아니오 | `app.js:68` | 사이드바 `database <db>` |
| `server` | 아니오 | `app.js:69` — **`.split(',')[0]`을 호출한다** | `server`가 `null`이면 `TypeError`로 `loadHealth()` 전체가 죽고 스키마 사이드바가 영영 비어 있게 된다 |
| `graphs[]` | 아니오 (빈 배열 가능) | `app.js:59-65` | `#graph-select`가 비고, `state.graph`가 `'default'`에 머무름 |
| `graphs[].name` | 아니오 | `app.js:60, 62-64` | 그래프 선택 전체 |
| `graphs[].graph_id` | 아니오 | **사용 안 함** | 없음 |

> **금지**: `server` 필드를 nullable로 만들지 않는다. `app.js:69`에 널 가드가 없다.

---

### 2.2 `GET /api/schema?graph=<name>`

`graph` 쿼리 파라미터 생략 시 `'default'` (`index.js:169`).

**응답 200**: `{ "schema": <og_schema(graph)>, "stats": <og_graph_stats(graph)> }`

`og_schema`의 실제 형태 (`engine/src/agent/mod.rs:69-112`):

```json
{
  "graph": "default",
  "schema_version": 3,
  "entity_types": [
    { "name": "Person", "abstract": false, "extends": ["Agent"],
      "instances": 128, "properties": [ { "name": "born", ... } ] }
  ],
  "relation_types": [
    { "name": "ACTED_IN", "abstract": false, "extends": [],
      "instances": 42, "roles": [...], "properties": [...] }
  ],
  "notes": [ "...", "...", "..." ],
  "truncated": { "shown": 8, "total": 40, "ordered_by": "...", "hint": "..." }
}
```

| 필드 | 널/부재 | 프런트 사용처 | 바꾸면 깨지는 곳 |
|---|---|---|---|
| `entity_types` | `\|\| []` 가드 있음 (`app.js:79`) | 엔티티 칩, 계층 트리 | — |
| `relation_types` | `\|\| []` 가드 있음 (`app.js:80`) | 관계 칩, 계층 트리 | — |
| `.name` | 아니오 | 칩 라벨 + **생성되는 Cypher 문자열** (`app.js:85, 95, 131`) | 이름이 바뀌면 원클릭 질의가 실패. **이스케이프 없이 HTML 속성에 들어간다** → `FE-15` |
| `.instances` | 아니오 | 칩 배지 (`app.js:88, 97`), 계층 옆 숫자 (`app.js:133`) | 배지가 `undefined` 표시 |
| `.abstract` | 아니오 | `∗` 접미 + `title` (`app.js:86-87`), `.abs` 클래스 (`app.js:131`) | 추상 타입 구분 사라짐 |
| `.extends` | `\|\| []` 가드 있음 (`app.js:117`) | 계층 트리의 부모 결정 | 트리가 전부 루트로 평평해짐 |
| `.properties[].name` | `\|\| []` 가드 있음 (`app.js:102`) | 프로퍼티 칩 | 프로퍼티 패널이 `—` |
| `.roles` | — | **사용 안 함** | 없음 |
| `notes` / `schema_version` / `truncated` | — | **사용 안 함** — `truncated`가 있어도 UI에 아무 표시가 없다 | 사용자가 잘린 스키마를 보고 있는지 알 수 없다 |

`og_graph_stats`의 형태 (`engine/src/storage/stats.rs:69-82`):

```json
{ "graph": "...", "nodes": 0, "edges": 0, "types": [...],
  "adjacency": { "segments": 0, "avg_fill": 0.0, "chunk_size": 256,
                 "packing_ratio": 1.0, "chunked_supernodes": 0 } }
```

프런트는 `state.stats`에 넣기만 하고(`app.js:77`) **어디에도 렌더하지 않는다.**
`:schema` 명령의 `Storage` 탭에서 JSON 원문으로만 보인다 (`app.js:796`).

> **결정**: `og_graph_stats`의 형태 변경은 Studio를 깨뜨리지 않는다.
> `og_schema`의 형태 변경은 사이드바 전체를 깨뜨린다.

---

### 2.3 `POST /api/cypher` ★ 가장 중요한 계약

**요청**:
```json
{ "graph": "default", "query": "MATCH (n) RETURN n LIMIT 25", "params": {} }
```

- `graph` 기본값 `'default'`, `query` 기본값 `''`, `params` 기본값 `{}` (`index.js:184`).
- `query.trim()`이 비면 서버가 400 `{ "error": "empty query" }` (`index.js:185`).
- 프런트는 **`params`를 항상 `{}`로 보낸다** (`app.js:688`). Studio에는 파라미터 입력 UI가 없다.
- 서버는 `params`를 `JSON.stringify`해 `og_cypher($1,$2,$3)`의 세 번째 인자로 넘긴다
  (`index.js:190-194`). 사용자 값이 SQL 텍스트로 보간되지 않는 경로다.

**응답 200**:
```json
{
  "rows": [ { "n": { "_id": 1, "_type": "Person", "name": "..." } } ],
  "columns": ["n"],
  "elapsed_ms": 12,
  "compiled_sql": "SELECT jsonb_build_object(...) ...",
  "graph": { "nodes": [ {...} ], "edges": [ {...} ] }
}
```

| 필드 | 널 | 만들어지는 곳 | 프런트 사용처 | 바꾸면 깨지는 곳 |
|---|---|---|---|---|
| `rows` | 아니오 (빈 배열 가능) | `index.js:195` — `og_cypher`가 낸 jsonb의 배열 | Table 뷰(`app.js:706`), JSON 뷰(`app.js:707`), 행 수 표시(`app.js:715`) | 전부 |
| `columns` | 아니오 | `index.js:205` — **`rows[0]`의 키만** 본다 | Table 헤더(`app.js:235-237`) | 첫 행에 없는 컬럼은 표에서 사라진다 (아래 §3.1) |
| `elapsed_ms` | 아니오 | `index.js:206` — 서버 왕복 시간 | 상태 줄(`app.js:717`) | `undefined ms` 표시 |
| `compiled_sql` | **예 (`null`)** | `index.js:196-202` — `og_cypher_sql` 실패 시 `null` | **SQL 탭의 존재 여부를 결정** (`app.js:708-710`) | 항상 `null`이면 SQL 탭이 영영 안 나옴 |
| `graph` | 아니오 | `projectGraph(rows)` (`index.js:208, 317-342`) | Graph 탭 존재 여부 + 상태 줄 (`app.js:702, 716`) | `graph.nodes`가 없으면 `app.js:702`에서 `TypeError` |
| `graph.nodes[]` | 아니오 | `_id`가 있고 `_src`/`_dst`가 없는 값 | 시뮬레이션 노드 | [04번 문서](04_graph_rendering.md) |
| `graph.edges[]` | 아니오 | `_id` + `_src` + `_dst`가 모두 있는 값, **양 끝점이 `nodes`에 있을 때만** | 시뮬레이션 엣지 | 동상 |

**응답 400**: `{ ...pgError, "elapsed_ms": <ms> }` (`index.js:211`)

#### 계약의 핵심: `_id` / `_type` / `_src` / `_dst`

이 네 키가 **그래프 시각화의 유일한 계약**이다. 만드는 곳은 엔진이다:

```sql
-- engine/sql/access.sql:231-234 (og_node_json)
RETURN jsonb_build_object('_id', og_node_json.id, '_type', t.name)
       || COALESCE(mapped, '{}'::jsonb)
       || (CASE WHEN jsonb_typeof(raw -> '__ext') = 'object' THEN raw -> '__ext' ELSE '{}'::jsonb END);

-- engine/sql/access.sql:259-263 (og_edge_json)
RETURN jsonb_build_object('_id', og_edge_json.id, '_type', t.name,
                          '_src', t.src, '_dst', t.dst)
       || COALESCE(mapped, '{}'::jsonb) || ...;
```

소비하는 곳:

| 키 | 서버 | 프런트 |
|---|---|---|
| `_id` | `index.js:325, 327, 330` (노드/엣지 판정 + 중복 제거 키) | `app.js:252` (렌더 분기), `358` (시뮬 id), `559` (expand), `635, 652, 656` (inspector) |
| `_type` | — | `app.js:257` (테이블 셀), `323` (범례), `458, 484, 489` (그리기), `633` (inspector) |
| `_src` / `_dst` | `index.js:326, 339` (엣지 판정 + 끝점 검증) | `app.js:367, 574-575` |

> **금지**: `_id` `_type` `_src` `_dst` 중 어느 하나라도 **이름을 바꾸거나 널로 만들면 안 된다.**
> `_src`/`_dst`가 사라지면 모든 관계가 **노드로 분류되어** 화면에 동그라미로 그려진다
> (`index.js:326`의 분기는 `undefined` 검사다).
>
> **금지**: 사용자 프로퍼티 이름이 `_`로 시작하도록 허용하면 안 된다.
> `renderValue`(`app.js:254`)와 `showInspector`(`app.js:637`)가 `_` 접두를 내부 필드로 간주해 거른다.

---

### 2.4 `POST /api/explain`

**요청**: `{ graph, query, analyze }` — 기본값 `'default'` / `''` / `false` (`index.js:219`).
프런트는 항상 `analyze: false`를 보낸다 (`app.js:740`) — `:explain`에 `ANALYZE` 옵션이 없다.

**응답 200**: `og_cypher_explain()`의 jsonb 그대로 (`index.js:226`).

```json
{ "columns": ["title", "type"], "sql": "SELECT ...", "plan": [ { "Plan": {...} } ] }
```
(`engine/src/cypher/mod.rs:691-695`)

| 필드 | 널 | 프런트 사용처 | 바꾸면 |
|---|---|---|---|
| `sql` | 아니오 | `SQL` 탭 — `codeView(data.sql, 'sql')` (`app.js:748`) | 빈 탭 |
| `plan` | **예 (`Value::Null`)** — `EXPLAIN`이 실패하면 (`mod.rs:690`) | `Plan` 탭 — `JSON.stringify(data.plan, null, 2)` (`app.js:749`) | `"null"` 문자열 표시 |
| `columns` | 아니오 | 상태 줄 — `(data.columns \|\| []).join(', ')` (`app.js:751`) | 가드 있음 |

---

### 2.5 `POST /api/diagnose`

**요청**: `{ graph, query }` (`index.js:234`).

서버가 세 함수를 **병렬로** 부르고, 뒤 둘은 실패해도 `null`로 채운다:

```js
// portal/server/index.js:236-249
const [err, empty, est] = await Promise.all([
  pool.query('SELECT og_explain_error($1,$2) AS d', ...),          // 실패하면 전체 400
  pool.query('SELECT og_diagnose_empty($1,$2) AS d', ...).catch(() => ({ rows: [{ d: null }] })),
  pool.query('SELECT og_estimate($1,$2) AS d', ...).catch(() => ({ rows: [{ d: null }] })),
]);
json(res, 200, { error: err.rows[0].d, empty: empty.rows[0].d, estimate: est.rows[0].d });
```

**응답 200**: `{ "error": {...}, "empty": {...}|null, "estimate": {...}|null }`

| 필드 | 널 | 프런트 사용처 (`runWhy`, `app.js:755-787`) |
|---|---|---|
| `error.ok` | 아니오 | `data.error?.ok ? 'query compiles' : ...` (`app.js:766`) |
| `error.message` | **예** | 옵셔널 체이닝 + `\|\| ''` 가드 (`app.js:766`) |
| `empty` | **예** | `data.empty?.steps \|\| []` (`app.js:763`) |
| `empty.steps[].description` | **예** (verdict 스텝에는 없음) | `<code>${escapeHtml(s.description)}</code>` (`app.js:773`) |
| `empty.steps[].rows` | **예** | `→ ${s.rows} rows` (`app.js:773`) |
| `empty.steps[].verdict` | **예** | 분기 조건 (`app.js:771`) |
| `empty.steps[].hint` | **예** | `\|\| ''` 가드 (`app.js:772`) |
| `estimate` | **예** | 전체 블록이 조건부 (`app.js:777`) |
| `estimate.estimated_rows` | 아니오 | `Math.round(... \|\| 0)` (`app.js:778-780`) |
| `estimate.estimated_cost` | 아니오 | 동상 |
| `estimate.advice[]` | 아니오 (빈 배열) | `(data.estimate.advice \|\| []).length` (`app.js:781-782`) |
| `estimate.sql` / `would_run` | — | **사용 안 함** |

스텝의 실제 형태는 두 종류다 (`engine/src/cypher/mod.rs:782-799`):
- 진행 스텝: `{ description, rows }`
- 판정 스텝: `{ verdict, hint }`

> **계약**: `steps` 배열 원소는 `verdict` 유무로 두 형태를 구분한다.
> 두 필드를 한 객체에 같이 넣으면 `app.js:771`이 판정 형태로만 렌더하고
> `description`/`rows`가 사라진다.

---

### 2.6 `POST /api/expand` — ⚠ **현재 깨져 있다**

**요청**: `{ id, limit }` — `limit` 기본 50 (`index.js:257`). 프런트는 `limit: 40` (`app.js:559`).

**서버 SQL** (`index.js:259-269`):

```sql
SELECT e.eid, e.nbr, og_node_json(e.nbr) AS node, og_edge_json(e.eid) AS edge
  FROM og_expand($1::int8, NULL, 'o'::"char") e
 LIMIT $2
 UNION ALL
SELECT e.eid, e.nbr, og_node_json(e.nbr), og_edge_json(e.eid)
  FROM og_expand($1::int8, NULL, 'i'::"char") e
 LIMIT $2
```

**이 SQL은 PostgreSQL에서 파싱되지 않는다.** `LIMIT`은 `UNION` 앞에 올 수 없다 —
집합 연산의 피연산자는 괄호로 감싸야 한다. PostgreSQL 17에서 같은 형태를 실행해 확인했다:

```
ERROR:  syntax error at or near "UNION"
LINE 1: SELECT 1 FROM generate_series(1,10) e LIMIT 3 UNION ALL SELE...
                                                      ^
```

결과적으로 **그래프 뷰의 노드 더블클릭(이웃 확장)은 항상 400을 받는다.**
그리고 프런트는 그것을 무시한다:

```js
// portal/web/app.js:559-560
const { ok, data } = await api('/api/expand', { id: n.d._id, limit: 40 });
if (!ok) return;
```

→ 사용자에게는 "아무 일도 일어나지 않는" 것으로 보인다. (`FE-01`)

**의도했던 응답 200**: `{ "nodes": [...], "edges": [...] }` — `og_node_json` / `og_edge_json`이
`null`인 행은 걸러진다 (`index.js:272-275`).

---

### 2.7 `GET /api/audit`

**요청**: 없음.

**응답 200**: `og_data.og_audit`의 최근 100행 배열 (`index.js:285-289`).

컬럼 (`engine/sql/bootstrap.sql:380-389`):
`audit_id` `principal` `at` `query` `lang` `rows_out` `duration_ms` `error_code`

| 필드 | 프런트 사용처 (`loadAudit`, `app.js:141-156`) | 주의 |
|---|---|---|
| `query` | `String(r.query \|\| '').replace(/^\[[^\]]*\]\s*/, '')` (`app.js:146`) | **`[<graph>] ` 접두를 벗긴다.** 엔진이 `format!("[{graph}] {query}")`로 넣는다 (`engine/src/cypher/mod.rs:128`). 이 형식을 바꾸면 접두가 그대로 노출된다 |
| `error_code` | 있으면 `'error'`, 없으면 `${rows_out} rows` (`app.js:149-150`) | §2.7.1 참조 |
| `rows_out` | 위와 동일 | |
| `duration_ms` | `Number(r.duration_ms).toFixed(1)` (`app.js:151`) | `null`이면 `"NaN ms"` |
| `at` | `new Date(r.at).toLocaleTimeString()` (`app.js:152`) | |
| `audit_id` / `principal` / `lang` | **사용 안 함** | `principal`은 감사에 중요한데 화면에 없다 |

#### 2.7.1 `error_code` 분기는 실질적으로 죽어 있다

`og_cypher`는 두 지점에서만 감사 행을 쓴다:

```rust
// engine/src/cypher/mod.rs:96   — 파싱 실패
audit(graph, query, 0, started, Some(&e));
error!("cypher parse error: {e}")          // ← 트랜잭션 중단 → 위 INSERT 롤백
// engine/src/cypher/mod.rs:107  — 성공
audit(graph, query, rows.len() as i64, started, None);
```

- 파싱 오류: INSERT 직후 `error!()`가 트랜잭션을 중단시키므로 **행이 남지 않는다.**
- 컴파일/실행 오류: `run_read`가 `error!()`를 던지므로(`mod.rs:140`) `audit()`에 도달하지 못한다.

따라서 `app.js:149-150`의 `error` 배지는 `og_cypher` 경로로는 나타날 수 없다.
*(코드 구조 근거. 라이브 DB 실측은 하지 않음.)*

---

### 2.8 `POST /api/sql`

**요청**: `{ sql }` — 기본값 `''` (`index.js:297`).

**응답 200**: `{ rows, columns, rowCount }` (`index.js:300-304`)

| 필드 | 출처 | 프런트 |
|---|---|---|
| `rows` | `pg` 드라이버의 `r.rows` | Table + JSON 뷰 (`app.js:732-733`) |
| `columns` | `r.fields.map(f => f.name)` — **`fields`가 없으면 `[]`** (`index.js:302`) | Table 헤더 |
| `rowCount` | `r.rowCount` — **`null` 가능** (DDL 등) | `<span class="ok">${data.rowCount} rows</span>` (`app.js:735`) → `"null rows"` 표시 |

> **보안**: 이 라우트는 풀 사용자(`PGUSER`, 기본 `dev`)의 권한으로 **임의 SQL을 실행한다.**
> 인증이 없고 `Origin` 검사도 없다. 상세는 `FE-17`.

---

### 2.9 `GET /api/benchmark`

[05_benchmark_report.md](05_benchmark_report.md)에서 따로 다룬다.

---

## 3. 프런트가 만드는 암묵적 가정 (문서화된 함정)

### 3.1 `columns`는 첫 행에서만 뽑는다

```js
// portal/server/index.js:205
columns: rows.length ? Object.keys(rows[0]) : [],
```

`og_cypher`가 행마다 다른 키 집합을 낸다면(예: `OPTIONAL MATCH`로 어떤 행에만 컬럼이 존재),
**첫 행에 없는 컬럼은 표에서 통째로 사라진다.** JSON 탭에는 보인다.

현재 컴파일러는 고정 컬럼 집합을 내므로 실제로 발생하지 않는 것으로 보이지만,
계약으로 못 박아 둔다: **모든 행은 같은 키 집합을 가져야 한다.**

### 3.2 `caption()`의 프로퍼티 이름 우선순위

```js
// portal/web/app.js:441-446
for (const k of ['name', 'title', 'label', 'model', 'id']) {
  if (d[k] != null) return String(d[k]);
}
return d._type || String(d._id);
```

그래프 노드의 캡션은 이 5개 이름 중 **먼저 발견되는 것**이다. 도메인 모델이
`name`을 다른 뜻으로 쓰면 엉뚱한 값이 노드에 찍힌다.

### 3.3 `highlightSql`의 리터럴 규칙은 동작하지 않는다

```js
// portal/web/app.js:274-277
return escapeHtml(sql)                                  // ' → &#39;
  .replace(kw, '<span class="kw">$1</span>')
  .replace(/\b(jsonb_build_object|...)\b/g, '<span class="fn">$1</span>')
  .replace(/('(?:[^']|'')*')/g, '<span class="lit">$1</span>');  // ← ' 가 이미 없음
```

`escapeHtml`(`app.js:804-808`)이 `'`를 `&#39;`로 바꾸므로 마지막 규칙은 **절대 매칭되지 않는다.**
`style.css:186`의 `pre.code .lit` 규칙은 죽은 CSS다. (`FE-18`)

### 3.4 `/api/cypher`의 SQL 컴파일은 **두 번째 왕복**이다

```js
// portal/server/index.js:190-202
const r = await client.query('SELECT og_cypher($1,$2,$3) AS row', [...]);
...
const p = await client.query('SELECT og_cypher_sql($1,$2) AS sql', [graph, query]);
```

같은 커넥션에서 컴파일을 한 번 더 시킨다. `elapsed_ms`(`index.js:206`)는 두 왕복을 모두 포함한다.
즉 Studio가 보여주는 서버 시간은 **순수 질의 시간이 아니다.**
`og_cypher_sql`이 실패하면(쓰기 질의 등) `catch`가 삼키고 `plan = null`이 된다 (`index.js:200-202`).

---

## 4. 변경 영향표 — "이걸 바꾸면 여기가 깨진다"

| 바꾸는 것 | 깨지는 프런트 | 증상 |
|---|---|---|
| `og_node_json`의 `_id` 키 이름 | `index.js:325`, `app.js:252,358,559,635` | 그래프 탭이 영영 안 나옴, inspector 빈 값 |
| `og_edge_json`의 `_src`/`_dst` | `index.js:326,339`, `app.js:367,574` | 관계가 **노드로** 그려짐 |
| `og_node_json`의 `_type` | `app.js:257,323,458,489,633` | 범례 비고, 색이 전부 같아짐, 캡션이 id로 대체 |
| 사용자 프로퍼티에 `_` 접두 허용 | `app.js:254,637` | 프로퍼티가 표/inspector에서 사라짐 |
| `og_schema`의 `entity_types` / `relation_types` 키 이름 | `app.js:79-80` | 사이드바 칩 + 계층 전부 빔 |
| `og_schema`의 `.instances` | `app.js:88,97,133` | 배지 `undefined` |
| `og_schema`의 `.extends` | `app.js:117-122` | 계층 트리가 평평해짐 |
| 오류 메시지의 `did you mean:` 문구 | `app.js:287` | 원클릭 교정 사라짐 |
| 오류 메시지의 `'<ident>'` 따옴표 | `app.js:289` | 교정 칩이 질의를 치환하지 못함 |
| `og_audit.query`의 `[graph] ` 접두 | `app.js:146` | 로그에 접두가 그대로 노출 |
| `og_cypher_explain`의 `sql`/`plan`/`columns` | `app.js:748-751` | `:explain` 탭이 빔 |
| `og_diagnose_empty`의 `steps[].verdict` 유무 규약 | `app.js:771` | `:why` 출력이 잘못된 형태로 렌더 |
| `og_estimate`의 `estimated_rows`/`estimated_cost`/`advice` | `app.js:778-783` | `:why`의 추정 블록이 0 표시 |
| `ontological_version()` 반환형 | `app.js:67` | 버전 표시 |
| `version()`을 널로 만드는 변경 | `app.js:69` | **`loadHealth()` 전체가 예외로 죽음** |
| `og_expand`의 시그니처 `(int8, int4[], "char")` | `index.js:262,265` | 이미 깨져 있음 (§2.6) |
| `bench/harness.py`의 질의 키 이름 | `benchmark.js:22-29` | 차트/표가 빔 → [05번 문서](05_benchmark_report.md) |

---

## 5. 규칙

### 필수 (Required)

- `_id` `_type` `_src` `_dst`는 **불변 계약**이다. 추가는 되지만 이름 변경·삭제는 안 된다.
- 새 필드를 추가할 때는 프런트가 없어도 동작하도록 **항상 옵셔널**로 만든다.
  프런트에는 널 가드가 거의 없다.
- 오류 메시지에 교정 후보를 넣을 때는 `did you mean: A, B` 형식과
  `'<잘못된 값>'` 표기를 유지한다.
- `og_schema`의 배열 필드(`entity_types` `relation_types` `extends` `properties`)는
  **비어 있어도 배열**로 낸다. 프런트가 `|| []`로 가드하는 곳과 안 하는 곳이 섞여 있다.
- 라우트를 추가하면 이 문서의 §2와 §4 표를 함께 갱신한다.

### 금지 (Forbidden)

- `pgError`의 다섯 필드 구조를 바꾸지 않는다 (`index.js:67-75`).
  프런트가 `error`만 쓰더라도, 그 형태가 계약이다.
- `/api/cypher`의 응답에서 `graph` 키를 제거하지 않는다.
  `app.js:702`가 `data.graph.nodes.length`를 널 가드 없이 읽는다.
- `rows` 배열의 행마다 다른 키 집합을 내지 않는다 (§3.1).
- 서버에 인증 없이 새 쓰기 라우트를 추가하지 않는다. `/api/sql`이 이미 최악이다.

---

## 6. 미확인

- `og_schema`의 `properties[]` 원소 형태는 `name` 외의 필드를 확인하지 않았다
  (프런트가 `p.name`만 쓴다, `app.js:102`).
- `roles`(`engine/src/agent/mod.rs:74`)의 형태는 프런트가 쓰지 않으므로 조사하지 않았다.
- `/api/audit`이 `og_data.og_audit`에 SELECT 권한을 요구하는데, `PGUSER=dev`가 이를
  갖는지는 `engine/sql/access.sql`의 GRANT를 확인하지 않았다.

<!-- affects: api, backend, frontend -->
<!-- requires-update: docs/04_frontend/02_state_flow.md, docs/04_frontend/04_graph_rendering.md, docs/04_frontend/05_benchmark_report.md -->
