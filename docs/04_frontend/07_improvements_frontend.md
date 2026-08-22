# 프런트엔드 개선 포인트

> **이 문서가 답하는 질문**
> - 지금 프런트엔드에서 실제로 깨져 있는 것은 무엇인가?
> - 무엇부터 고쳐야 하나?
> - 각 항목의 근거는 어느 파일 몇 번째 줄인가?

**모든 항목은 실제 코드를 읽고 작성했다.** 일부 항목(`FE-01`)은 PostgreSQL 17에서
같은 형태의 SQL을 실행해 확인했다. 확인하지 못한 것은 항목 안에 "미확인"으로 표시했다.

---

## 요약 표

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| FE-01 | `/api/expand` SQL이 PostgreSQL 문법 오류 | **High** | `portal/server/index.js:259-269` · `portal/web/app.js:559-560` | `LIMIT`이 `UNION ALL` 앞에 있어 파싱 실패. 그래프 노드 더블클릭(이웃 확장)이 100% 400을 받고, 프런트가 조용히 삼킴 | 각 분기를 `(SELECT … LIMIT $2)`로 괄호화하거나 두 질의로 분리. `if (!ok)` 경로에서 사용자에게 표시 | 문서화된 기능 1개 복구 | 괄호화 시 방향별 상한이 되어 총 반환 행이 `2 × limit`이 됨 — 의도 확인 필요 |
| FE-02 | 벤치마크 렌더러가 하네스 출력 키와 어긋남 | **High** | `portal/web/benchmark.js:12-29` vs `bench/results/bench-250000-20260817T052859Z.json` | 결과는 `reach10hop…reach500hop` + `pggraph`, 렌더러는 `1hop/2hop/3hop` + `typedb`. **기본 화면(250,000)에서 차트 막대 0개, 표 대부분 `—`, 산문 지표 대부분 `—`** | `QUERIES`/`SYSTEMS`를 payload에서 유도(키 → 라벨 매핑만 유지). `HEADLINE`은 존재하는 키 중에서 선택 | 리포트 페이지가 다시 값을 표시 | 라벨/주석(`note`)은 사람이 붙여야 하므로 미지의 키는 키 자체를 라벨로 |
| FE-03 | 정답 게이트를 서버가 하네스와 다르게 재계산 | **High** | `portal/server/index.js:127` vs `bench/harness.py:1089` | 하네스는 `error:`로 시작하는 답을 빼고 비교하는데, 서버는 그대로 집합에 넣음 → 하네스가 `agree: true`로 기록한 실행이 화면에서 **"systems disagreed, those timings are void"** 빨간 배너로 표시 | `Object.values(kept).filter(v => !String(v).startsWith('error'))` 후 비교. 오류 셀은 별도 표기 | 게이트가 하네스와 일치 | 없음. 다만 "타임아웃도 불일치로 본다"는 정책이었다면 하네스 쪽을 고쳐야 함 |
| FE-04 | 대량 결과 렌더 — DOM 폭발, 가상 스크롤 없음, 서버·클라 양쪽 상한 없음 | **High** | `portal/web/app.js:236-243, 707` · `portal/server/index.js:195, 203-209, 317-342` | `rows.map(...).join('')` 전체 문자열 → `innerHTML`. JSON 탭은 `JSON.stringify(전체)`. 세 뷰가 **동시에** DOM에 존재(`app.js:217-221`). `MATCH (n) RETURN n`에 아무 상한도 없음 | 서버에 응답 행 상한(예 5,000) + `truncated` 플래그, 테이블 가상 스크롤, JSON 탭은 지연 생성(`onShow`) | 수만 행 질의에서 탭 정지·OOM 방지 | 잘림이 눈에 보이지 않으면 사용자가 오해 → `truncated` UI 필수 |
| FE-05 | 질의 취소(abort) 수단이 전혀 없음 | **High** | `portal/web/app.js:39-47` (AbortController 0건) · `portal/server/index.js:21-29, 188-214` | 클라이언트에 취소 없음, 서버에 `statement_timeout` 없음, 풀 `max: 8`. 느린 질의 8개면 Studio 전체가 응답 불가 | 프레임에 ✕ 취소 버튼 + `AbortController`, 서버는 요청 abort 시 `client.query('SELECT pg_cancel_backend(pg_backend_pid())')` 또는 세션 `statement_timeout` 설정 | 폭주 질의로 콘솔이 죽지 않음 | 취소된 쓰기 질의의 트랜잭션 상태를 명확히 정의해야 함 |
| FE-06 | force 시뮬레이션 O(n²) + 프레임마다 독립 rAF + window 리스너 누수 | **High** | `portal/web/app.js:392-413, 534-551, 602-616` | 반발 계산이 매 프레임 모든 쌍 (1,000노드 = 499,500회). 저자 주석이 상한을 "a few hundred nodes"로 명시(`app.js:352`). `window`의 `mousemove`/`mouseup` 리스너가 **절대 제거되지 않음** — 프레임을 닫아도 클로저가 `nodes` 배열을 붙잡음. `draw()`는 alpha가 식어도 계속 호출(`app.js:610`) | Barnes-Hut 쿼드트리 또는 노드 수 상한 + 경고. 루프 종료 지점(`app.js:611-614`)에서 `removeEventListener`. alpha 소진 시 `draw()`도 중단 | 수백 노드 이상에서 사용 가능, 장시간 세션 메모리 안정 | 쿼드트리 도입은 레이아웃 결과가 미세하게 달라짐 |
| FE-07 | 사이드바/도움말/계층의 클릭 대상이 `span`·`div`·`pre` — 키보드 접근 불가 | **High** | `portal/web/app.js:84-105, 131-134` · `portal/web/index.html:72-85` · `portal/web/style.css:66-78` | 원클릭 질의는 `[data-run]` 위임 클릭(`app.js:862`)뿐. 엔티티 칩·관계 칩·타입 계층·도움말 스니펫 **전부 포커스 불가**. 즉 Studio의 탐색 기능 전체가 마우스 전용 | `<button type="button" data-run=…>`로 교체하고 `.chip`/`.t`/`.snippet` 스타일을 버튼에 맞춤 | 키보드·스크린리더 사용자가 스키마 탐색 가능 | 마크업 변경 폭이 큼. `#hierarchy`의 `white-space: pre` 들여쓰기와 버튼 기본 스타일 충돌 |
| FE-08 | 결과 프레임에 탭 시맨틱·라이브 리전 없음 | Med | `portal/web/app.js:199-224` · `portal/web/index.html:114` | `.tab`은 `<button>`이지만 `role="tab"`·`aria-selected`·`aria-controls`가 없고, 패널에 `role="tabpanel"`이 없음. `#stream`에 `aria-live` 없어 결과 도착이 알려지지 않음. 탭↔패널이 **배열 인덱스로만** 결합(`app.js:212`) | `role`/`aria-selected`/`aria-controls` + id 부여, `#stream`에 `aria-live="polite"`, ←/→ 로빙 포커스 | 스크린리더로 결과 확인 가능 | 인덱스 결합을 id 결합으로 바꾸는 리팩터 필요 |
| FE-09 | 명도 대비 미달 (WCAG 1.4.3 4.5:1 기준) | Med | `portal/web/style.css:116` · `portal/web/app.js:133` · `portal/web/style.css:72, 78` · `portal/web/benchmark.css:12, 88` | 실측: 에디터 placeholder **2.35:1**(`rgba(147,161,179,.45)` on `#191d24`) — 유일한 사용법 안내다. 계층 인스턴스 수 `opacity:.5` **2.61:1**. 추상 타입 `opacity:.7` **3.83:1**. 칩 배지 `opacity:.55` **2.1~2.8:1**(색상별). 벤치마크 `--fg-mute #6b7787` on `#191d24` **3.71:1**(12.5px 본문) | `opacity` 대신 명시적 토큰 사용, `--fg-mute`를 4.5:1 이상으로 올림, placeholder를 `--fg-dim`(6.43:1)으로 | WCAG AA 충족, 저조도/저시력 환경 가독성 | 시각적 위계가 평평해짐 — 크기·굵기로 보완 |
| FE-10 | Studio에 반응형이 전무 | Med | `portal/web/style.css:22, 25` (`@media` 0건) | `body { grid-template-columns: 56px 320px 1fr }` 고정 + `html,body { overflow: hidden }`. 좁은 화면에서 결과 영역이 짓눌리고 가로 스크롤로 피할 수도 없음. `.graph-view { height: 460px }` 고정(`style.css:155`) | 최소 2개 브레이크포인트(사이드바 접힘 → 오버레이 드로어), `.graph-view`를 `clamp()`나 `vh` 기반으로 | 태블릿·좁은 창에서 사용 가능 | 레일/사이드바 토글 UI가 새로 필요 |
| FE-11 | `web/index.html`에 doctype·charset·viewport·lang이 전부 없음 | **High** | `web/index.html:1` (파일 1행이 `<title>`) · grep: `<!doctype` `<meta` `lang=` 각 0건 | ① charset 미선언 — 한국어 UTF-8 문서가 서버 헤더에만 의존, 실패 시 전체 mojibake. ② **viewport 미선언 — 파일 안의 `@media` 13개가 실제 모바일에서 한 번도 발동하지 않음**(가상 뷰포트 ≈980px). ③ `lang` 없음 — 스크린리더가 한국어를 영어 음성으로 읽음 | 파일 앞에 `<!doctype html><html lang="ko"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">` 추가 + `<meta name="description">`/OG 태그 | 모바일 레이아웃이 실제로 동작, 인코딩 안전, SEO/공유 카드 | 없음. 가장 비용 대비 효과가 큰 항목 |
| FE-12 | 랜딩 페이지 수치가 파일 안에 두 번 하드코딩 + 푸터 날짜 불일치 | Med | `web/index.html:739-773, 949-972` (44개 `data-ms` + 44개 텍스트) · `web/index.html:1656` | 막대 길이(`data-ms`)와 표시 숫자(`.vv`)가 독립 문자열. 한쪽만 고치면 조용히 어긋남. 푸터는 `모든 수치: 2026-08-06`인데 03.4~03.6 수치는 `bench-*-20260817T*.json` 계열로 보임 | ① 텍스트를 `data-ms`에서 생성(`paint()`에서 함께 채움) → 진실을 하나로. ② 장기적으로 `bench/results/`를 읽어 빌드 시 주입 | 드리프트 제거, 갱신 비용 절반 | ②는 빌드 스텝을 도입 — 현재 "파일 하나" 원칙과 충돌 |
| FE-13 | `#audit-refresh` 버튼에 핸들러가 없음 (죽은 UI) | Low | `portal/web/index.html:63` · `portal/web/app.js` (grep 0건) | Query log 패널의 `refresh` 버튼을 눌러도 아무 일도 일어나지 않음 | `$('#audit-refresh').onclick = loadAudit;` 한 줄 | 신뢰도 회복 | 없음 |
| FE-14 | 상태가 전역·DOM·localStorage에 흩어짐 + 최상위 `JSON.parse`가 무방비 | Med | `portal/web/app.js:13-20, 187-197, 202-224` | ① `localStorage.getItem('og.saved')`가 손상되면 `app.js:17`이 최상위에서 던져 **Studio 전체가 백지**(스크립트 파싱은 되지만 실행이 중단). ② 결과·실행 상태가 DOM에만 존재해 복원 불가. ③ `og.saved` 상한이 저장 경로에만 적용되어(`app.js:192` vs `:874`) 50개 초과 후 항목을 지우면 잘렸던 항목이 되살아남 | `JSON.parse`를 try/catch로 감싸고 실패 시 `[]`, 상한을 `state.saved` 갱신 시점에 적용, localStorage 접근을 한 모듈로 모음 | 손상된 상태에서도 부팅, 저장 질의 동작 일관성 | 최소 변경이면 리스크 없음 |
| FE-15 | 타입 이름이 이스케이프 없이 HTML 속성에 삽입 — 저장형 XSS 면 | **High** | `portal/web/app.js:85, 95, 131` · `engine/sql/bootstrap.sql:35-46` | `data-run="MATCH (n:${t.name}) RETURN n LIMIT 50"` — `t.name`에 `escapeHtml`이 없다(같은 줄의 라벨 출력에는 있다: `app.js:86`). `og_catalog.type.name`은 `text NOT NULL`이고 **문자 클래스 제약이 없어** `"`나 `<`를 담을 수 있다(`og_create_type`의 `create_type_inner`에도 검증 없음, `engine/src/catalog/types.rs:363-400`). 따라서 악의적 타입 이름 하나로 사이드바를 렌더하는 모든 사용자에게 스크립트를 주입할 수 있다 | 세 지점 모두 `escapeHtml(...)` 적용. 나아가 `data-run` 조립을 `dataset.run = ...`(DOM API)로 바꿔 문자열 결합 자체를 제거 | 저장형 XSS 차단 | 없음 |
| FE-16 | 실패를 조용히 삼키는 경로가 다수 — 복구 불가 상태가 남음 | Med | `portal/web/app.js:75, 143, 560, 791` | `if (!ok) return;`이 네 곳. 특히 `runSchemaCmd`(`app.js:789-791`)는 **프레임을 이미 만든 뒤** 실패해 프레임이 `<div class="empty">running…</div>`(`app.js:185`)에서 영구 정지. `loadSchema` 실패 시 사이드바가 이유 없이 텅 빔 | 프레임을 만든 뒤 실패하는 경로는 전부 `errorView`로. 사이드바 실패는 패널에 인라인 오류 표시 | 사용자가 무엇이 실패했는지 알 수 있음 | 없음 |
| FE-17 | Studio 서버에 인증·CORS·Origin 검사·바인드 제한이 없고 `/api/sql`이 임의 SQL을 실행 | **High** | `portal/server/index.js:296-308, 344-368, 48-64` | ① `server.listen(PORT, cb)` — 호스트 인자가 없어 `0.0.0.0` 바인드. ② 인증 없음. ③ `readBody`가 **content-type을 검사하지 않아** `<form enctype="text/plain">` 한 장으로 preflight 없이 `POST /api/sql` **CSRF**가 성립(응답은 못 읽지만 쓰기는 실행됨). ④ `/api/cypher`·`/api/sql`이 `PGUSER`(기본 `dev`) 권한 전부를 노출 | ① `server.listen(PORT, '127.0.0.1')`. ② `Origin`/`Sec-Fetch-Site` 검사 또는 기동 시 발급하는 토큰. ③ `content-type: application/json` 강제. ④ `/api/sql`을 `OG_ALLOW_RAW_SQL=1`일 때만 등록 | 로컬 개발 도구로서 최소한의 안전선 | 토큰 도입은 `start.sh`와 문서 갱신 필요 |
| FE-18 | `highlightSql`의 문자열 리터럴 규칙이 동작하지 않음 (죽은 코드) | Low | `portal/web/app.js:274-277, 806` · `portal/web/style.css:186` | `escapeHtml`이 `'`를 `&#39;`로 바꾼 **뒤에** `/('(?:[^']\|'')*')/g`를 적용 → 절대 매칭되지 않음. SQL 탭에서 리터럴이 영원히 하이라이트되지 않고 `pre.code .lit` CSS가 죽어 있음 | 정규식을 `&#39;` 기준으로 바꾸거나, 토크나이즈 후 이스케이프하는 순서로 변경 | SQL 탭 가독성 | 이스케이프 순서를 바꾸면 XSS 위험 — 토크나이저 방식 권장 |
| FE-19 | `og-vs-neo4j` 비율의 방향이 산문 어순에 암묵 결합 | Low | `portal/web/benchmark.js:365` · `portal/web/benchmark.html:40-42` | 값은 `ontological ÷ neo4j`(= 우리가 몇 배 **느린가**)인데 문장은 "… — `<값>` faster". Ontological이 더 빨라지면 `0.5× faster` 같은 무의미한 문장이 렌더됨 | 비율이 1보다 큰지 작은지에 따라 주어와 단어(`faster`/`slower`)를 함께 생성 | 수치가 뒤집혀도 문장이 참 | 문장 생성 로직이 HTML에서 JS로 이동 |
| FE-20 | 랜딩 사이트 구현 현황에 스펙 `010`(TypeQL)이 없음 | Low | `web/index.html:1531-1540` (`001…009, 011`) | 저장소에는 스펙 11개가 있고 010은 partial 상태인데 목록에서 빠짐. 의도적 생략인지 누락인지 미확인 | 의도라면 목록 상단에 "일부만 표시"를 명시, 누락이면 `partial` 행 추가 | 현황표의 신뢰도 | 없음 |
| FE-21 | `/api/cypher`가 매 질의마다 컴파일 왕복을 한 번 더 하고 그 시간이 `elapsed_ms`에 포함됨 | Low | `portal/server/index.js:187-207` | `og_cypher` 실행 후 같은 커넥션에서 `og_cypher_sql`을 또 호출. 화면의 `server N ms`(`app.js:717`)는 **질의 시간이 아니다.** SQL 탭을 열지 않아도 항상 발생 | `og_cypher_explain`처럼 필요할 때만 요청하는 별도 라우트로 분리하거나, `elapsed_ms`를 첫 질의 구간만 측정 | 표시 시간의 정직성 + 왕복 1회 절감 | SQL 탭이 지연 로딩되면 UX가 살짝 달라짐 |

---

## 우선순위 제안

### 1군 — 지금 사용자가 잘못된 것을 보고 있다

`FE-02` `FE-03` — 벤치마크 리포트가 기본 화면에서 빈 차트 + 거짓 경고를 보여준다.
이 프로젝트의 핵심 주장이 걸린 페이지다.

### 2군 — 문서화된 기능이 동작하지 않는다

`FE-01` — 더블클릭 이웃 확장. `FE-13` — refresh 버튼.

### 3군 — 보안

`FE-15` (저장형 XSS) · `FE-17` (인증 부재 + CSRF).
`FE-17`은 "로컬 전용 도구"라는 전제를 문서화하는 것만으로도 절반은 해결된다.

### 4군 — 규모와 접근성

`FE-04` `FE-05` `FE-06` (대량 결과·취소·시뮬레이션) ·
`FE-07` `FE-08` `FE-09` (a11y) · `FE-11` (랜딩 boilerplate — 비용 대비 효과 최고).

---

## 세부 근거 — 검증 방법

### FE-01의 검증

`portal/server/index.js:259-269`의 SQL 형태를 최소 재현으로 PostgreSQL 17에서 실행:

```sql
-- minimal reproduction of the shape at portal/server/index.js:259-269
SELECT 1 FROM generate_series(1,10) e LIMIT 3
UNION ALL
SELECT 2 FROM generate_series(1,10) e LIMIT 3;
```

```
ERROR:  syntax error at or near "UNION"
LINE 1: SELECT 1 FROM generate_series(1,10) e LIMIT 3 UNION ALL SELE...
                                                      ^
```

PostgreSQL 문법에서 `LIMIT`/`ORDER BY`는 집합 연산의 피연산자에 직접 붙을 수 없고
괄호가 필요하다. 이 라우트는 어떤 입력으로도 성공할 수 없다.

### FE-02의 검증

`bench/results/`의 전 파일에서 시스템·질의 키를 추출해 `benchmark.js:12-29`와 대조했다.
스케일별 병합 결과와 화면 귀결은 [05_benchmark_report.md §5.2](05_benchmark_report.md)의 표에 있다.

### FE-03의 검증

`bench-250000-20260817T052859Z.json`의 `correctness.reach20hop`:
- 파일: `agree: true`, `answers.age = "error: … statement timeout"`, 나머지 `"230"`
- `harness.py:1089`: `distinct = {v for v in answers.values() if not v.startswith("error")}` → `{"230"}` → 일치
- `index.js:127`: `[...new Set(Object.values(kept))]` → `{"230", "error: …"}` → 불일치

### FE-09의 측정

WCAG 2.x 상대 명도 공식으로 계산한 값이다. 배경은 해당 요소가 실제로 놓이는 표면
(`--bg-2 #191d24` / `--bg-3 #20252e`)을 사용했고, `opacity`는 배경과의 알파 합성으로 환산했다.

### FE-15의 검증

`portal/web/app.js:85`:
```js
data-run="MATCH (n:${t.name}) RETURN n LIMIT 50"
```
같은 템플릿의 라벨 출력(`app.js:86`)에는 `escapeHtml(t.name)`이 있으나 속성 컨텍스트에는 없다.
`escapeHtml`(`app.js:804-808`)은 `"`를 `&quot;`로 바꾸므로, **적용만 하면** 속성 탈출이 막힌다.
타입 이름의 제약 부재는 `engine/sql/bootstrap.sql:38`(`name text NOT NULL`)과
`engine/src/catalog/types.rs:363-400`(검증 없이 그대로 INSERT)에서 확인했다.

`portal/web/benchmark.js`는 대조적으로 안전하다 — `innerHTML` 사용 4곳이 전부 `''`이고
데이터는 `textContent`로만 들어간다 (`benchmark.js:32-37`).

---

## 미확인 사항

- `FE-04`/`FE-06`의 실제 임계치(몇 행/몇 노드에서 브라우저가 멈추는가)는 측정하지 않았다.
  본문의 숫자는 연산 횟수의 산술이다.
- `FE-17`의 CSRF는 코드 구조상 성립하지만 실제 공격 페이지로 재현하지는 않았다.
- `FE-20`의 `010` 누락이 의도인지 확인하지 못했다.
- 라이브 데이터베이스에 대한 Studio 실행 테스트는 하지 않았다.
  `FE-01`을 제외한 모든 항목은 정적 코드 분석과 실측 파일 대조에 기반한다.

<!-- affects: frontend, api, security, operations -->
<!-- requires-update: docs/04_frontend/03_api_contract_rules.md, docs/04_frontend/05_benchmark_report.md -->
