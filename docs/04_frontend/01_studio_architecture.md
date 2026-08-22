# Studio 아키텍처 — 왜 빌드 스텝이 없고, 왜 서버가 얇은가

> **이 문서가 답하는 질문**
> - Studio는 무엇으로 만들어졌고, 파일이 어떻게 나뉘어 있나?
> - 왜 번들러도 프레임워크도 없나? 그게 의도인가 미완성인가?
> - 백엔드 서버는 왜 376줄뿐인가? 무엇을 하고 무엇을 하지 않나?
> - 어떻게 띄우나? 어떤 환경변수를 읽나?

---

## 1. 결정 (Decisions)

### D-1. 서버는 "의도적으로 얇다"

이건 추측이 아니라 코드 최상단에 적힌 설계 의도다.

```js
// portal/server/index.js:2-9
/**
 * Ontological Studio — backend.
 *
 * Deliberately thin: it holds a connection pool, forwards Cypher to
 * `og_cypher()` and exposes the introspection surface the browser needs. All the
 * intelligence lives in the database, which is the point — anything this server
 * can do, a psql session can do too.
 */
```

**귀결**: 이 서버에 비즈니스 로직을 추가하면 안 된다. 새 기능이 필요하면
엔진의 `#[pg_extern]` 함수로 만들고, 서버는 그것을 그대로 전달만 한다.
서버가 할 수 있는 일은 psql 세션도 할 수 있어야 한다는 것이 이 파일의 계약이다.

**예외 1건**: `projectGraph()` (`portal/server/index.js:317-342`)는 순수 서버 로직이다.
결과 행에서 노드/엣지를 추려내는 투영이며, psql에는 대응물이 없다.
자세한 규칙은 [04_graph_rendering.md](04_graph_rendering.md) 참조.

**예외 2건**: `readBenchmarks()` (`portal/server/index.js:86-138`)는 `bench/results/`의
파일들을 병합한다. 데이터베이스를 전혀 건드리지 않는다.

### D-2. 프런트엔드에 빌드 스텝이 없다

`portal/package.json:11`의 의존성은 **`pg` 하나뿐**이다.

```json
"dependencies": { "pg": "^8.13.0" }
```

- 번들러 없음, 트랜스파일 없음, `node_modules`가 브라우저로 나가지 않음.
- `portal/web/index.html:122`는 `<script src="/app.js"></script>` 한 줄이다. 모듈 시스템도 없다.
- 프런트 파일 4개(`app.js`, `style.css`, `benchmark.js`, `benchmark.css`)가 그대로 서빙된다.

의도 역시 코드에 적혀 있다:

```js
// portal/web/app.js:4-6
 * A query stream of result frames, each with Graph / Table / SQL views, over a
 * canvas force layout. No build step and no dependencies: the whole thing is one
 * file you can read.
```

**귀결**: `import` / `export` / JSX / TypeScript를 이 디렉터리에 넣으면 그 순간 동작이 멈춘다.
`'use strict'` 전역 스크립트 하나라는 것이 전제다.

### D-3. HTTP 서버는 `node:http` 직접 사용 — express 없음

`portal/server/index.js:344-366`이 서버 전체다. 라우팅은 객체 하나의 키 조회다:

```js
// portal/server/index.js:346-348
const key = `${req.method} ${url.pathname}`;
if (routes[key]) { ... }
```

**귀결**: 미들웨어 개념이 없다. 인증·CORS·레이트리밋·로깅이 전부 없다는 뜻이기도 하다.
(→ [07_improvements_frontend.md](07_improvements_frontend.md) `FE-17`)

### D-4. 그래프 시각화는 canvas + 직접 구현한 force layout

d3도, cytoscape도, vis.js도 쓰지 않는다. `createSimulation()`
(`portal/web/app.js:354-629`)이 전부 직접 구현이다.
이유는 D-2와 같다 — 의존성이 생기면 빌드 스텝이 필요해진다.

---

## 2. 사실 (Facts) — 파일 구조

```
portal/
  package.json             12줄   의존성 pg 하나. scripts: start / dev
  package-lock.json               (커밋되어 있음)
  server/
    index.js              376줄   HTTP 서버 · pg Pool · 라우트 8개 · projectGraph · readBenchmarks
  web/
    index.html            124줄   Studio 셸 (rail / sidebar / editor / stream / inspector)
    app.js                880줄   프런트 전체 — 상태, 전송, 뷰, force layout, 이벤트 배선
    style.css             213줄   다크 콘솔 스타일. @media 쿼리 0개
    benchmark.html        123줄   벤치마크 리포트 문서 셸
    benchmark.js          378줄   /api/benchmark 소비 · 차트/표/툴팁 렌더
    benchmark.css         183줄   리포트 스타일. @media 쿼리 1개 (max-width: 720px)
```

라인 수는 실측(`wc -l`)이다.

### 2.1 `portal/web/index.html`의 골격

| 영역 | 라인 | 내용 |
|---|---|---|
| `<aside id="rail">` | 12-31 | 좌측 아이콘 레일. `data-panel` 4개(schema/saved/audit/help) + `/benchmark.html` 링크 + `#conn-dot` |
| `<section id="sidebar">` | 33-100 | 패널 4개. `.panel.hidden`으로 전환 |
| `#entity-chips` / `#rel-chips` / `#prop-chips` / `#hierarchy` | 43-52 | 스키마 패널의 렌더 타깃 (app.js가 innerHTML로 채움) |
| `<main id="main">` | 102-115 | `#editor-wrap`(textarea + run 버튼) + `#stream`(결과 프레임 누적) |
| `<div id="inspector">` | 117-120 | 노드 클릭 시 뜨는 고정 패널 |

레이아웃은 `body`의 3열 그리드 하나다:

```css
/* portal/web/style.css:25 */
body { display: grid; grid-template-columns: 56px 320px 1fr; }
```

`portal/web/style.css` 전체에 미디어 쿼리가 **0개**다 (`grep -c "@media"` = 0).
즉 Studio는 데스크톱 고정 레이아웃이며, 좁은 화면에서 `1fr` 열이 짓눌린다.
`html, body { overflow: hidden }` (`style.css:22`) 때문에 가로 스크롤로 피할 수도 없다.

### 2.2 서버 라우트 8개

| 메서드 | 경로 | 라인 | 실패 시 상태코드 |
|---|---|---|---|
| GET | `/api/benchmark` | 142-148 | 500 |
| GET | `/api/health` | 151-165 | **503** |
| GET | `/api/schema` | 168-179 | 400 |
| POST | `/api/cypher` | 182-215 | 400 |
| POST | `/api/explain` | 218-230 | 400 |
| POST | `/api/diagnose` | 233-253 | 400 |
| POST | `/api/expand` | 256-280 | 400 |
| GET | `/api/audit` | 283-293 | 400 |
| POST | `/api/sql` | 296-308 | 400 |

라우트 키에 없는 경로는 전부 정적 파일로 떨어진다 (`index.js:357-365`).

요청/응답 형태와 널 가능 필드는 [03_api_contract_rules.md](03_api_contract_rules.md)에 있다.

### 2.3 커넥션 풀

```js
// portal/server/index.js:21-29
const pool = new Pool({
  host: process.env.PGHOST || 'localhost',
  port: Number(process.env.PGPORT || 28816),
  database: process.env.PGDATABASE || 'og',
  user: process.env.PGUSER || 'dev',
  password: process.env.PGPASSWORD || undefined,
  max: 8,
  idleTimeoutMillis: 30_000,
});
```

**최대 8 커넥션.** 이 숫자가 중요한 이유:

- `POST /api/cypher`만 `pool.connect()`로 커넥션을 잡아 두 개의 질의
  (`og_cypher` + `og_cypher_sql`)를 같은 커넥션에서 돌린다 (`index.js:188-214`).
  나머지 라우트는 `pool.query()`로 매번 빌려 쓴다.
- **취소 수단이 없다.** 클라이언트에도 `AbortController`가 없고
  (`grep AbortController portal/web/app.js` = 0), 서버도 `statement_timeout`을 걸지 않는다.
  느린 질의 8개면 Studio 전체가 멈춘다.

### 2.4 정적 파일 서빙

```js
// portal/server/index.js:357-365
let file = url.pathname === '/' ? '/index.html' : url.pathname;
const full = path.join(WEB_DIR, path.normalize(file).replace(/^(\.\.[/\\])+/, ''));
if (!full.startsWith(WEB_DIR) || !fs.existsSync(full)) { 404 }
```

- 경로 탈출은 `path.normalize` + `startsWith(WEB_DIR)` 두 겹으로 막는다.
- MIME 테이블은 `.html .js .css .svg .ico` 5종뿐 (`index.js:31-37`).
  그 외 확장자는 `application/octet-stream`으로 나가므로, 예를 들어
  `docs/images/studio.png`를 `portal/web/`에 두면 브라우저가 렌더하지 않고 다운로드한다.
- 캐시 헤더·ETag·압축이 전혀 없다.

---

## 3. 실행 방법 (사실)

### 3.1 저장소 스크립트 — 권장

```bash
./start.sh
```

`start.sh`가 하는 일 (파일 근거):

| 단계 | 라인 | 내용 |
|---|---|---|
| 컨테이너 | 15-31 | `ontological-dev` 컨테이너 생성/시작. PG는 호스트 `28816`, Bolt는 호스트 `28687` |
| PostgreSQL | 34-42 | `cargo pgrx start pg16` |
| 확장 + 데모 | 45-52 | `CREATE EXTENSION ontological CASCADE` + `examples/demo.sql` |
| Bolt 게이트웨이 | 56-65 | 선택 사항 (`OG_BOLT=0`이면 건너뜀) |
| **Studio** | 68-77 | `npm install` → 기존 프로세스 `pkill` → `nohup node portal/server/index.js` |
| 헬스체크 | 79-89 | `/api/health`를 최대 10초 폴링 |

Studio를 띄우는 실제 명령은 `start.sh:76-77`:

```bash
PGHOST=127.0.0.1 PGPORT="$PGPORT" PGDATABASE="$DB" PGUSER=dev PORT="$PORT" \
    nohup node "$ROOT/portal/server/index.js" > /tmp/ontological-studio.log 2>&1 &
```

로그는 `/tmp/ontological-studio.log`에 쌓인다.

### 3.2 직접 실행

```bash
cd portal && npm install
PGHOST=127.0.0.1 PGPORT=28816 PGDATABASE=og PGUSER=dev npm start
# → http://localhost:7474
```

`npm run dev`는 `node --watch server/index.js`다 (`package.json:9`).
**주의**: `--watch`는 서버만 재시작한다. 프런트 파일은 브라우저 새로고침이 필요하다.

### 3.3 환경변수 전체

| 변수 | 기본값 | 근거 |
|---|---|---|
| `PORT` | `7474` | `index.js:17` — Neo4j Browser의 기본 포트와 같다 |
| `PGHOST` | `localhost` | `index.js:22` |
| `PGPORT` | `28816` | `index.js:23` |
| `PGDATABASE` | `og` | `index.js:24` |
| `PGUSER` | `dev` | `index.js:25` |
| `PGPASSWORD` | `undefined` | `index.js:26` |
| `OG_BENCH_DIR` | `<repo>/bench/results` | `index.js:19` |

`start.sh`가 추가로 읽는 것: `OG_CONTAINER` `OG_PORT` `OG_PGPORT` `OG_BOLTPORT` `OG_DB` `OG_BOLT`
(`start.sh:6-10, 56`).

### 3.4 종료

`SIGINT` 하나만 처리한다 (`index.js:374-376`). `SIGTERM`은 처리하지 않으므로
`docker stop` 같은 경로에서는 풀이 정리되지 않고 프로세스가 죽는다.

---

## 4. 서버가 하지 않는 것 (중요)

| 없음 | 근거 |
|---|---|
| 인증·인가 | `index.js` 전체에 토큰/세션/헤더 검사가 없다 |
| CORS 헤더 | 응답 헤더는 `content-type`/`content-length`뿐 (`index.js:41-44`) |
| `Origin` 검사 | 없음 → CSRF 가능 (`readBody`는 content-type을 보지 않는다, `index.js:48-64`) |
| 바인드 주소 제한 | `server.listen(PORT, cb)` — 호스트 인자가 없어 `0.0.0.0` (`index.js:368`) |
| 요청 로깅 | 없음 |
| 질의 취소 | 없음 |
| 행 수 상한 | 없음 — `og_cypher`가 낸 모든 행을 JSON으로 만든다 (`index.js:195, 203-209`) |
| `statement_timeout` | 없음 |

이것들이 "의도적으로 얇다"의 대가다. **Studio는 로컬 개발 도구다.**
네트워크에 노출할 물건이 아니다. (→ `FE-17`)

---

## 5. 규칙

### 필수 (Required)

- `portal/web/` 아래 JS는 **전역 스크립트**로 작성한다. `'use strict'` 유지.
- 새 API 로직은 엔진의 SQL 함수로 만들고 서버는 전달만 한다 (D-1).
- 라우트를 추가하면 `routes` 객체의 키를 `"<METHOD> <path>"` 형식으로 정확히 맞춘다
  (`index.js:346`은 문자열 완전 일치다 — 패턴 매칭이 없다).
- 서버가 `catch`한 PostgreSQL 오류는 반드시 `pgError(e)`로 감싼다 (`index.js:67-75`).
  `hint`/`detail`은 엔진이 만든 교정 후보를 담고 있고, 프런트가 그것으로 버튼을 만든다
  (`app.js:286-308`).

### 금지 (Forbidden)

- `portal/web/`에 npm 패키지를 추가하지 않는다. 추가하는 순간 빌드 스텝이 필요해지고 D-2가 깨진다.
- `pool.max`를 늘려서 취소 문제를 회피하지 않는다. 근본 원인은 취소 부재다 (`FE-05`).
- 이 서버를 `0.0.0.0` 이외의 신뢰 경계에 두지 않는다. 인증이 없다.
- 정적 파일 경로 검사(`index.js:359-360`) 두 줄을 "리팩터링"하지 않는다. 경로 탈출 방어다.

---

## 6. 미확인

- `web/index.html`(랜딩 사이트)이 어떻게 배포되는지 — 저장소에 CI/호스팅 설정이 없다.
  Studio 서버는 이 파일을 서빙하지 않는다 (`WEB_DIR`는 `portal/web`이다, `index.js:18`).
- `portal/web/`에 파비콘 파일이 없고 `index.html:8`은 data URI SVG를 쓴다.
  `.ico` MIME 항목(`index.js:36`)이 실제로 쓰이는 경로는 확인하지 못했다.

<!-- affects: frontend, operations -->
<!-- requires-update: docs/04_frontend/03_api_contract_rules.md -->
