# Studio HTTP API 계약

> **이 문서가 답하는 질문**
> - Studio 백엔드가 실제로 노출하는 라우트는 무엇인가?
> - 각 라우트의 요청/응답 스키마와 상태 코드는?
> - 인증이 있는가? (요약: **없다**)
> - 어떤 라우트가 위험한가?
> - 정적 파일은 어떻게 서빙되는가?

---

## 1. 사실 — 서버의 성격

`portal/server/index.js`는 **376줄의 의도적으로 얇은 서버**다. 커넥션 풀을 들고,
Cypher를 `og_cypher()`로 전달하고, 브라우저가 필요로 하는 인트로스펙션 표면을
노출한다. 모든 지능은 데이터베이스에 있다 — 이 서버가 할 수 있는 것은 psql
세션도 할 수 있다는 것이 요점이다([portal/server/index.js:2](../../portal/server/index.js#L2)).

프레임워크가 없다. Node.js `node:http` 위에 라우트 객체 하나
([portal/server/index.js:140](../../portal/server/index.js#L140))와 정적 파일 폴백이 전부다.

### 1.1 설정 — 환경변수

| 환경변수 | 기본값 | 설명 |
|---|---|---|
| `PORT` | `7474` | HTTP 리슨 포트 |
| `PGHOST` | `localhost` | PostgreSQL 호스트 |
| `PGPORT` | `28816` | PostgreSQL 포트 |
| `PGDATABASE` | `og` | 데이터베이스 |
| `PGUSER` | `dev` | 사용자 |
| `PGPASSWORD` | (없음) | 비밀번호 |
| `OG_BENCH_DIR` | `<repo>/bench/results` | 벤치마크 결과 디렉터리 |

([portal/server/index.js:17](../../portal/server/index.js#L17))

커넥션 풀: `max: 8`, `idleTimeoutMillis: 30000`.

### 1.2 ⚠️ 인증이 없다

**모든 라우트가 인증 없이 열려 있다.** 요청은 풀의 단일 자격증명
(`PGUSER`/`PGPASSWORD`)으로 실행된다. 세션·토큰·API 키·CORS 정책·레이트 리밋이
전부 없다.

이 서버에 도달할 수 있는 누구든 `POST /api/sql`로 **임의 SQL을 실행할 수 있다**(§3.8).

> 🔒 **필수**: 이 서버를 신뢰 경계 밖에 노출하지 말 것. 로컬 개발 콘솔로만 쓸 것.
> → [12_improvements_api.md](12_improvements_api.md) **API-29**.

---

## 2. 공통 규약

### 2.1 라우팅

라우트 키는 `"<METHOD> <pathname>"` 문자열의 **정확 일치**다
([portal/server/index.js:346](../../portal/server/index.js#L346)).

- 쿼리스트링은 무시된다(`url.pathname`만 사용).
- 메서드가 다르면 **`405`가 아니라 정적 파일 폴백**으로 떨어져
  `404 not found` (`text/plain`)가 된다.
- 미들웨어·CORS 헤더·`OPTIONS` 처리가 없다.

### 2.2 요청 본문

`readBody()` ([index.js:48](../../portal/server/index.js#L48)):
- `Content-Type`을 **검사하지 않는다.**
- 본문이 비면 `{}`.
- JSON 파싱 실패 → `Error('invalid JSON body')`.
- 누적 길이 `> 4,000,000` 바이트면 reject → 라우트 핸들러가 던지고 **`500`** 이 된다.

### 2.3 오류 응답 형태

DB 오류는 `pgError()`로 정규화된다([index.js:67](../../portal/server/index.js#L67)):

```json
{
  "error":  "role 'actor' of relation 'ACTED_IN' requires a 'Person', got 'Film'",
  "code":   "XX000",
  "detail": null,
  "hint":   null,
  "where":  null
}
```

**결정(Decision)**: PostgreSQL 오류가 엔진이 만든 교정 힌트를 담고 있으므로
**그대로 보존한다**([index.js:66](../../portal/server/index.js#L66)).

> ⚠️ `code`는 거의 항상 `XX000`이다. pgrx의 `error!` 매크로가 SQLSTATE를
> 구분하지 않기 때문 → [11_errors.md](11_errors.md).

### 2.4 상태 코드 규약 (실측)

| 코드 | 언제 |
|---|---|
| `200` | 성공 |
| `400` | **거의 모든 DB 오류** — 문법 오류든 서버 내부 오류든 |
| `500` | 라우트 핸들러가 예외를 던졌을 때 (`{error: e.message}`) |
| `503` | `GET /api/health`에서 DB 연결 실패 |
| `404` | 알 수 없는 경로 (`text/plain` 본문 `"not found"`) |

> ⚠️ 클라이언트 오류와 서버 오류가 모두 `400`이라 재시도 가능 여부를 구별할 수
> 없다 → [12_improvements_api.md](12_improvements_api.md) **API-30**.

---

## 3. 라우트 계약

### 3.1 `GET /api/benchmark`

정의: [portal/server/index.js:142](../../portal/server/index.js#L142)

**무엇을 하는가**: `OG_BENCH_DIR`의 벤치마크 결과 JSON들을 스케일별로 병합해 반환한다. DB를 건드리지 않는다.

**요청**: 본문 없음. 쿼리 파라미터 없음.

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `scales[]` | array | 노드 수 오름차순. 각 항목: `{scale, environment, systems, runs, correctness, generated_at, integrity_violations?}` |
| `source` | string\|null | 읽은 디렉터리 경로. 디렉터리가 없으면 `null`이고 `scales`는 `[]` |

**파일 이름 규칙**: `bench-<scale>-<YYYYMMDDTHHMMSSZ>.json`
([index.js:91](../../portal/server/index.js#L91)). 시각 오름차순으로 읽어
**시스템별 최신 승(newest-wins)** 병합. `queries`가 없는 항목(스킵/실패한 시스템)은
좋은 실행을 덮어쓰지 않는다([index.js:111](../../portal/server/index.js#L111)).

**결정(Decision)**: 리포트 페이지는 하네스가 마지막으로 쓴 것을 렌더링한다.
아무도 지연 시간을 HTML에 다시 타이핑하지 않으므로 사이트가 측정에서 표류할 수
없다([index.js:79](../../portal/server/index.js#L79)).

**응답 `500`**: `{error, scales: []}`.
반쯤 쓰인 파일은 조용히 건너뛴다([index.js:100](../../portal/server/index.js#L100)).

---

### 3.2 `GET /api/health`

정의: [portal/server/index.js:151](../../portal/server/index.js#L151)

**무엇을 하는가**: 서버와 확장의 상태. 연결 배너가 쓴다.

**요청**: 없음.

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `ok` | bool | `true` |
| `version` | string | `ontological_version()` |
| `database` | string | `current_database()` |
| `server` | string | PostgreSQL `version()` 전문 |
| `graphs[]` | array | `{name, graph_id}` — 이름순 |

**응답 `503`**: `{ok: false, ...pgError(e)}`.

```bash
curl -s localhost:7474/api/health | jq
```

---

### 3.3 `GET /api/schema?graph=<name>`

정의: [portal/server/index.js:168](../../portal/server/index.js#L168)

**무엇을 하는가**: 사이드바 내용 — `og_schema()`와 `og_graph_stats()`를 **병렬로** 호출.

**요청**

| 파라미터 | 위치 | 필수 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | query | 선택 | `'default'` | 그래프 이름 |

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `schema` | object | [`og_schema(graph)`](07_agent_interface.md) 결과 그대로 |
| `stats` | object | [`og_graph_stats(graph)`](05_traversal_and_stats.md) 결과 그대로 |

**응답 `400`**: `pgError(e)` — 그래프가 없으면 여기로 온다.

> ⚠️ `og_schema()`는 타입마다 `count(*)`를 돌린다. 사이드바를 열 때마다 비용이 든다.

---

### 3.4 `POST /api/cypher`

정의: [portal/server/index.js:182](../../portal/server/index.js#L182)

**무엇을 하는가**: Cypher를 실행하고 행 + 그래프 투영을 반환한다.

**요청 본문**

| 필드 | 타입 | 필수 | 기본값 | 설명 |
|---|---|---|---|---|
| `graph` | string | 선택 | `'default'` | 그래프 이름 |
| `query` | string | **필수** | `''` | Cypher 질의 |
| `params` | object | 선택 | `{}` | `JSON.stringify(params)` 되어 jsonb로 전달 |

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `rows[]` | array | `og_cypher()`의 jsonb 행들 |
| `columns[]` | array\<string\> | ⚠️ **첫 행의 키 순서** — `og_cypher_columns()`를 쓰지 않는다 |
| `elapsed_ms` | number | 서버 측 왕복 시간 |
| `compiled_sql` | string\|null | `og_cypher_sql()` 결과. 쓰기 질의면 `null` |
| `graph` | object | `{nodes[], edges[]}` — 시각화용 투영 |

**응답 `400`**: `{...pgError(e), elapsed_ms}`.
`query.trim()`이 비면 `{error: "empty query"}`.

**커넥션 처리**: 풀에서 클라이언트 하나를 잡아 `og_cypher`와 `og_cypher_sql`을
**같은 커넥션**에서 실행하고 `finally`에서 반납한다([index.js:188](../../portal/server/index.js#L188)).

> ⚠️ **`columns`가 부정확하다.** jsonb는 키를 정렬하므로 `Object.keys(rows[0])`는
> `RETURN` 순서가 아니다([index.js:205](../../portal/server/index.js#L205)).
> 엔진에 정확한 답(`og_cypher_columns`)이 있는데 쓰지 않는다. 결과가 0행이면
> `[]`가 된다 → [12_improvements_api.md](12_improvements_api.md) **API-30**.

> ⚠️ `params`가 객체가 아니면(예: 문자열) `JSON.stringify`가 JSON 문자열을 만들고
> `og_cypher`는 jsonb 객체를 기대한다. 검증이 없다.

**그래프 투영 규칙** ([index.js:317](../../portal/server/index.js#L317) `projectGraph`):
- 값을 재귀적으로 훑어 `_id`를 가진 객체를 수집.
- `_src`와 `_dst`가 있으면 엣지, 없으면 노드.
- **양 끝점이 화면에 있는 엣지만** 남긴다([index.js:338](../../portal/server/index.js#L338)).

```bash
curl -s localhost:7474/api/cypher \
  -H 'content-type: application/json' \
  -d '{"graph":"default","query":"MATCH (p:Person) RETURN p LIMIT 3","params":{}}' | jq
```

---

### 3.5 `POST /api/explain`

정의: [portal/server/index.js:218](../../portal/server/index.js#L218)

**무엇을 하는가**: `og_cypher_explain()` 결과를 그대로 반환한다.

**요청 본문**

| 필드 | 타입 | 필수 | 기본값 |
|---|---|---|---|
| `graph` | string | 선택 | `'default'` |
| `query` | string | 선택 | `''` |
| `analyze` | bool | 선택 | `false` |

**응답 `200`**: `{columns, sql, plan}` — [03_cypher.md §2](03_cypher.md) 참조.
**응답 `400`**: `pgError(e)`.

> ⚠️ **`analyze: true`는 질의를 실제로 실행한다.** 이 라우트는 인증이 없으므로
> 임의의 읽기 질의를 무제한 실행시키는 경로다.

---

### 3.6 `POST /api/diagnose`

정의: [portal/server/index.js:233](../../portal/server/index.js#L233)

**무엇을 하는가**: 세 진단 함수를 **병렬로** 호출한다.

**요청 본문**

| 필드 | 타입 | 필수 | 기본값 |
|---|---|---|---|
| `graph` | string | 선택 | `'default'` |
| `query` | string | 선택 | `''` |

**응답 `200`**

| 키 | 소스 | 실패 시 |
|---|---|---|
| `error` | `og_explain_error(graph, query)` | 전체 요청이 `400` |
| `empty` | `og_diagnose_empty(graph, query)` | `null` (개별 `.catch()`) |
| `estimate` | `og_estimate(graph, query)` | `null` (개별 `.catch()`) |

([index.js:236](../../portal/server/index.js#L236))

**응답 `400`**: `og_explain_error`가 실패한 경우만.

---

### 3.7 `POST /api/expand`

정의: [portal/server/index.js:256](../../portal/server/index.js#L256)

**무엇을 하는가**: 노드 하나의 이웃을 양방향으로 펼친다. 시각화의 더블클릭 동작.

**요청 본문**

| 필드 | 타입 | 필수 | 기본값 | 설명 |
|---|---|---|---|---|
| `id` | number\|string | **필수** | (없음) | `int8`로 캐스트되는 노드 id |
| `limit` | number | 선택 | `50` | 방향별 상한 |

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `nodes[]` | array | `og_node_json(nbr)` 결과들 |
| `edges[]` | array | `og_edge_json(eid)` 결과들 |

실행되는 SQL은 `og_expand(id, NULL, 'o')`와 `og_expand(id, NULL, 'i')`의
`UNION ALL`이고 **각각에 `LIMIT $2`** 가 붙는다([index.js:259](../../portal/server/index.js#L259)) —
즉 최대 `2 × limit` 행.

**응답 `400`**: `pgError(e)`. `id`가 없거나 숫자가 아니면 캐스트 오류가 여기로 온다.

> ⚠️ `id`와 `limit`은 파라미터로 바인딩되지만(주입 없음) **검증되지 않는다.**
> `limit`에 음수/거대한 값을 넣을 수 있다.

---

### 3.8 🔒 `POST /api/sql` — 임의 SQL 실행

정의: [portal/server/index.js:296](../../portal/server/index.js#L296)

**무엇을 하는가**: 본문의 SQL을 풀 자격증명으로 그대로 실행한다.
소스 주석: *"Raw SQL escape hatch — this is still PostgreSQL underneath."*

**요청 본문**

| 필드 | 타입 | 필수 | 기본값 |
|---|---|---|---|
| `sql` | string | 선택 | `''` |

**응답 `200`**

| 키 | 타입 | 설명 |
|---|---|---|
| `rows[]` | array | 결과 행 |
| `columns[]` | array\<string\> | `r.fields`의 이름들 |
| `rowCount` | number\|null | 영향받은 행 수 |

**응답 `400`**: `pgError(e)`.

> 🔒 **인증·허용목록·읽기전용 제한이 전혀 없다.** `DROP TABLE`, `COPY … FROM
> PROGRAM`(슈퍼유저일 때), `pg_read_file` 등 풀 사용자의 모든 권한이 그대로 노출된다.
>
> **필수**: 프로덕션 배포 시 이 라우트를 제거하거나, 서버 전체를 신뢰 경계 안에
> 두거나, 읽기 전용 role로 풀을 구성할 것
> → [12_improvements_api.md](12_improvements_api.md) **API-29**.

---

### 3.9 `GET /api/audit`

정의: [portal/server/index.js:283](../../portal/server/index.js#L283)

**무엇을 하는가**: `og_data.og_audit`의 최근 100건을 반환한다(spec 008 FR-027).

**요청**: 파라미터 없음. **필터링·페이지네이션 불가.**

**응답 `200`**: 행 배열.

| 컬럼 | 설명 |
|---|---|
| `audit_id` | 감사 id |
| `principal` | 실행 주체 |
| `at` | 시각 (`ORDER BY at DESC`) |
| `query` | `[<graph>] <query text>` |
| `lang` | `'cypher'` \| `'typeql'` |
| `rows_out` | 반환 행 수 |
| `duration_ms` | 소요 시간 |
| `error_code` | 오류 메시지 앞 200자 (성공 시 `null`) |

**응답 `400`**: `pgError(e)`.

> ⚠️ 감사 로그에는 **질의 전문**이 들어 있다. 인증 없는 라우트가 이것을 노출한다.
> 질의 텍스트에 값이 포함되어 있으면(파라미터를 쓰지 않은 경우) 데이터가 샌다.

---

## 4. 정적 파일 서빙

정의: [portal/server/index.js:357](../../portal/server/index.js#L357)

- 루트 `/` → `index.html`
- 파일 루트: `portal/web/`
- 경로 정규화: `path.normalize(file).replace(/^(\.\.[/\\])+/, '')` 후
  `full.startsWith(WEB_DIR)` 확인 → 디렉터리 탈출 방어
- 존재하지 않으면 `404 not found` (`text/plain`)

**MIME 매핑** ([index.js:31](../../portal/server/index.js#L31)):
`.html`, `.js`, `.css`, `.svg`, `.ico`. 그 외는 `application/octet-stream`.

> ⚠️ 캐시 헤더(`Cache-Control`, `ETag`)와 보안 헤더(`Content-Security-Policy`,
> `X-Content-Type-Options`)가 전혀 설정되지 않는다.

---

## 5. 종료

`SIGINT`에서 풀을 닫고 종료한다([index.js:374](../../portal/server/index.js#L374)).
`SIGTERM` 핸들러는 **없다** — 컨테이너 환경에서 graceful shutdown이 되지 않는다.

---

## 6. 라우트 요약표

| 메서드 | 경로 | 인증 | DB 접근 | 위험도 |
|---|---|---|---|---|
| `GET` | `/api/benchmark` | 없음 | 아니오 (파일 시스템) | 낮음 |
| `GET` | `/api/health` | 없음 | 예 | 낮음 |
| `GET` | `/api/schema` | 없음 | 예 | 중간 (비용) |
| `POST` | `/api/cypher` | 없음 | 예 | **높음** (임의 Cypher, 쓰기 포함) |
| `POST` | `/api/explain` | 없음 | 예 | 중간 (`analyze`가 실행) |
| `POST` | `/api/diagnose` | 없음 | 예 | 중간 (부분 패턴 실행) |
| `POST` | `/api/expand` | 없음 | 예 | 낮음 |
| `POST` | `/api/sql` | 없음 | 예 | 🔒 **치명적** (임의 SQL) |
| `GET` | `/api/audit` | 없음 | 예 | 중간 (질의 전문 노출) |
| `GET` | `/*` | 없음 | 아니오 | 낮음 |

---

## 7. 금지 / 필수

- 🔒 **금지**: 이 서버를 공개 네트워크에 노출하지 말 것. 인증이 전혀 없다.
- 🔒 **금지**: `POST /api/sql`을 남긴 채 배포하지 말 것.
- **금지**: `POST /api/cypher` 응답의 `columns`를 `RETURN` 순서로 신뢰하지 말 것 —
  jsonb 키 정렬 결과다.
- **금지**: 상태 코드 `400`을 "클라이언트 잘못"으로 해석하지 말 것 — 서버 오류도 `400`이다.
- **필수**: 로컬 개발 용도로만 쓸 것. 원격 접근이 필요하면 앞단에
  리버스 프록시 + 인증 + TLS를 둘 것.
- **필수**: 풀 사용자(`PGUSER`)를 최소 권한 role로 구성할 것 —
  `/api/sql`의 실질적 권한 상한이 그것이다.
- **필수**: `Content-Type: application/json`을 보낼 것. 서버가 검사하지는 않지만
  본문은 JSON이어야 한다.

---

## 8. 관련 문서

- Cypher 진입 함수 → [03_cypher.md](03_cypher.md)
- 진단 함수 → [07_agent_interface.md](07_agent_interface.md)
- 오류 체계 → [11_errors.md](11_errors.md)
- 개선 제안 → [12_improvements_api.md](12_improvements_api.md)

<!-- affects: api, frontend, security -->
<!-- requires-update: 02_api/12_improvements_api.md -->
