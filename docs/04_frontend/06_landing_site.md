# 랜딩 사이트 — `web/index.html` 한 파일의 구조와 유지보수 규칙

> **이 문서가 답하는 질문**
> - 1,916줄짜리 단일 파일이 어떻게 나뉘어 있나? 어디를 고쳐야 하나?
> - 이 페이지의 수치는 어디서 오나? Studio 벤치마크 페이지와 어떻게 다른가?
> - 무엇이 빠져 있나? (스포일러: `<!doctype>`, `<meta charset>`, `<meta viewport>`)
> - 수치를 갱신할 때 함께 고쳐야 하는 곳은 어디인가?

---

## 1. 사실 — 파일 한 개, 외부 의존성 0

| 항목 | 값 | 근거 |
|---|---|---|
| 총 라인 | 1,916 | `wc -l web/index.html` |
| `<style>` 블록 | 1개 (3-415행) | `grep -c "<style"` = 1 |
| `<script>` 블록 | 1개 (1660-1916행) | `grep -c "<script"` = 1 |
| 외부 리소스 | **0개** — `src="http`, `href="http`, `@import` 전부 0건 | grep |
| `@media` 쿼리 | 13개 | grep |
| `table.data` 표 | 9개 | grep |
| `.bar-fill` 막대 | 44개 (전부 `data-ms` 하드코딩) | grep |
| 언어 | 한국어 본문 | — |

폰트도 시스템 스택뿐이다 (`web/index.html:27-30`) — Pretendard / Apple SD Gothic Neo /
Noto Sans KR가 로컬에 있으면 쓰고, 없으면 시스템 산세리프로 떨어진다.
**웹폰트를 내려받지 않는다.**

---

## 2. 라인 맵 — 어디를 고칠 것인가

| 라인 | 영역 | 내용 |
|---|---|---|
| 1 | `<title>` | **파일의 첫 줄이 `<title>`이다** — §4 참조 |
| 3-415 | `<style>` | 전체 CSS |
| ├ 9-33 | `:root` | 라이트 토큰 + 폰트 스택 + `--w: 1140px` |
| ├ 35-54 | `@media (prefers-color-scheme: dark)` | 다크 토큰 |
| ├ 56-69 | `:root[data-theme=...]` | 명시적 테마 오버라이드 (**토글 UI는 없음** — §5.3) |
| ├ 88-109 | 셸 | `.wrap` `.band` `.shead` |
| ├ 111-128 | 헤더 | `.topbar` `.topnav` |
| ├ 130-180 | 히어로 | `.hero` `.metrics` `.footnote` |
| ├ 182-255 | 컴파일러 데모 | `.studio` `.pane-head` `.rtable` `.gpane` |
| ├ 257-298 | 카드/표 | `.grid` `.cell` `.tablewrap` `table.data` |
| ├ 300-347 | 벤치마크 차트 | `.chart` `.bar-row` `.s-*` 색 매핑 `.chart-axis` |
| ├ 349-364 | 정직성 패널 | `.honest` |
| ├ 366-381 | 구현 현황 | `.status` `.srow` `.st-*` |
| ├ 383-402 | 에이전트 루프 / 코드 블록 / 최종 CTA | `.loop` `pre.block` `.final` |
| └ 410-414 | 접근성 | `prefers-reduced-motion` + `:focus-visible` |
| 417-438 | `<header class="topbar">` | 워드마크 + `.topnav` 링크 9개 |
| 440-1650 | `<main id="top">` | 본문 (§3) |
| 1652-1658 | `<footer>` | 라이선스 + 스택 + **날짜가 박힌 문장** |
| 1660-1916 | `<script>` | IIFE 3부 (§5) |

---

## 3. 섹션 구성

`.shead .rail`의 번호가 이 사이트의 목차다 (`web/index.html`에서 grep 가능):

| 라인 | 앵커 | rail | 제목 |
|---|---|---|---|
| 443 | (히어로) | — | `CREATE EXTENSION ontological CASCADE;` + 지표 4칸 |
| 491 | `#seam` | `01 · 제품의 핵심` | Cypher가 들어가고, 플래너가 최적화하는 SQL이 나옵니다 |
| 551 | `#why` | `02 · 특장점` | 여섯 가지가 구조적으로 다릅니다 (h3 6개: 567/579/593/608/622/638) |
| 656 | — | `02.1 · 구조 대조` | Apache AGE와 무엇이 다른가 |
| 714 | `#bench` | `03 · 벤치마크` | 하네스를 우리에게 적대적으로 짰습니다 |
| 791 | — | `03.1 · 논리 페이지` | 페이지 접근 횟수는 저장 구조의 직접 함수입니다 |
| 826 | `#deep` | `03.2 · 깊은 홉` | 4홉을 넘기면 그건 경로 열거였습니다 |
| 874 | — | `03.3 · 무엇을 바꿨나` | 컴파일러가 질문을 다시 읽을 뿐입니다 |
| 932 | — | `03.4 · 깊은 홉 벤치마크` | Neo4j·AGE·pgGraph를 직접 빌드해 같은 서버에 |
| 1008 | `#diameter` | `03.5 · 지름이 큰 그래프` | 10만 홉을 걸어봤습니다 |
| 1067 | — | `03.6 · … 엔진 비교` | **여기서는 Neo4j가 우리를 이깁니다** |
| 1131 | — | (`.honest`) | 이 표에서 우리가 지는 칸들 (li 8개) |
| 1193 | — | `pre.block` | 재현 명령 전체 |
| 1220 | `#agent` | `04 · 에이전트 네이티브` | `.loop` 스텝 6개 |
| 1296 | `#usecase` | `05 · 활용 용도` | `.cell` 6개 + `.honest`(쓰지 말아야 할 경우 4개) |
| 1405 | `#neo4j` | `06 · Neo4j 샘플 이식` | movies 데이터셋 이식 결과 |
| 1517 | `#status` | `07 · 구현 현황` | `.srow` 10개 + 타협 2건 |
| 1568 | `#start` | `08 · 시작하기` | Docker → Studio → 첫 30초 → 문서 표 |
| 1635 | — | `.final` | 수치를 믿지 마십시오. 직접 돌려 보십시오 |

`.topnav`의 링크 9개(`index.html:427-435`)는 이 앵커들을 가리킨다:
`#seam #why #bench #deep #diameter #usecase #neo4j #status #start`.

> **주의**: `.topnav`는 `@media (max-width: 860px)`에서 `display: none`이 된다
> (`index.html:128`). **대체 메뉴가 없다** — 좁은 화면에서는 섹션 이동 수단이 사라진다.

### 3.1 구현 현황 목록의 공백

`index.html:1531-1540`의 `.srow`는 10개다: `001 002 003 004 005 006 007 008 009 011`.

**`010`(TypeQL 질의 표면, partial)이 빠져 있다.** 저장소의 스펙은 011개이고
README의 상태표에는 010이 있다. 의도적 생략인지 누락인지 미확인이다. (`FE-20`)

---

## 4. 빠져 있는 것 — HTML 보일러플레이트 전체 ⚠

파일의 1행이 `<title>`이다. 실측 (`grep -c`):

| 요소 | 개수 |
|---|---|
| `<!doctype ...>` | **0** |
| `<html ...>` | **0** |
| `<head>` | **0** |
| `<meta ...>` (전부) | **0** |
| `<body>` | **0** |
| `lang=` 속성 | **0** |

브라우저는 이것들을 암묵적으로 만들어 주므로 페이지는 "뜬다". 하지만 세 가지가 실제 문제다:

### 4.1 `<meta charset="utf-8">` 없음

본문 전체가 한국어 UTF-8이다. HTTP 응답에
`Content-Type: text/html; charset=utf-8`이 붙지 않으면 브라우저가 인코딩을 추정해야 하고,
추정은 로케일 의존적이다. 실패하면 **페이지 전체가 깨진 글자로 나온다.**

Studio 서버는 이 파일을 서빙하지 않으므로(`WEB_DIR`는 `portal/web`,
`portal/server/index.js:18`), 어떤 서버가 어떤 헤더로 내보내는지가 미확인이다.
`<meta charset>` 한 줄이면 서버와 무관하게 안전해진다.

### 4.2 `<meta name="viewport">` 없음 — 반응형 CSS 13개가 전부 죽는다

모바일 브라우저는 viewport 메타가 없으면 가상 뷰포트를 **약 980px**로 잡고 페이지 전체를 축소한다.
그 결과:

- `@media (max-width: 860px)` / `(max-width: 760px)` / `(max-width: 640px)` /
  `(max-width: 460px)` — **어느 것도 발동하지 않는다.**
- `.topnav`가 숨겨지지 않고, `.metrics`가 4열로 남고, `.grid-2`가 2열로 남고,
  `.loop`가 3열로 남는다.
- `clamp(36px, 5.6vw, 62px)`의 `vw`가 980px 기준으로 계산된다.

즉 **작성된 반응형 규칙 13개가 실제 폰에서 한 번도 쓰이지 않는다.** (`FE-11`)

`portal/web/index.html:5`와 `portal/web/benchmark.html:5`에는 viewport 메타가 있다.
랜딩 사이트에만 없다.

### 4.3 `lang` 속성 없음

스크린 리더가 한국어 본문을 기본 로케일(대개 영어) 음성으로 읽는다.
`<html lang="ko">` 한 줄로 해결된다.

---

## 5. `<script>` 블록 — 세 부분

전체가 IIFE 하나다 (`index.html:1661-1915`). 프레임워크도 모듈도 없고,
`var` + `function` 기반 ES5 스타일이다.

### 5.1 컴파일러 심(seam) 데모 — `index.html:1664-1856`

```js
var state = { q: 0, view: "sql" };   // index.html:1803
```

- 데이터는 `QUERIES` 배열 (`index.html:1673-1756`) — 예제 3개.
  각 원소: `{ note, cypher, sql, explain, cols, rows, foot, graph }`.
- 하이라이팅은 문자열 결합으로 미리 만든다: `K()`/`L()`/`I()`/`V()` 헬퍼가
  `<span class="k1">` 등을 붙인다 (`index.html:1668-1671`).
  **즉 `cypher`와 `sql` 필드는 이미 HTML이고, `innerHTML`로 직접 들어간다**
  (`index.html:1828, 1839`).
- 그래프 탭은 고정 SVG 문자열 하나다 (`GRAPH_SVG`, `index.html:1758-1801`).
  `role="img"` + `aria-label`이 붙어 있다 (`index.html:1759`).
- `renderTable()`(`index.html:1815-1824`)만 `esc()`를 쓴다. 그런데 `esc()`는
  `&`와 `<`만 처리한다 (`index.html:1813`) — 셀 값(`r`)은 **이스케이프 없이** 들어간다
  (`index.html:1819`). 데이터가 전부 이 파일 안의 상수라 실害는 없지만,
  값을 외부에서 가져오도록 바꾸는 순간 XSS가 된다.

주석이 데이터 출처를 명시한다 (`index.html:1664-1666`):
> Every query, result and compiled statement below is taken from the
> repository (README.md, docs/, examples/demo.sql, Studio screenshot).

### 5.2 로그 스케일 막대 — `index.html:1858-1903`

```js
// web/index.html:1862-1868
var LO = Math.log10(0.1), HI = Math.log10(30000);
function widthFor(ms) {
  var w = (Math.log10(ms) - LO) / (HI - LO) * 100;
  return Math.max(1.2, Math.min(100, w));
}
```

- 축 눈금 `[0.1, 1, 10, 100, 1000, 10000]`을 **막대와 같은 함수로 배치**한다
  (`index.html:1875-1881`). 눈으로 맞춘 눈금이 아니라는 뜻 — 주석이 그렇게 밝힌다
  (`index.html:1859-1861`, `index.html:333-334`).
- 두 차트(`#chart`, `#chart-deep`)가 **같은 축**을 공유한다. 이유도 주석에 있다
  (`index.html:1870-1871`): 얕은 워크로드와 깊은 워크로드를 비교하는 독자 아래에서
  축이 바뀌면 안 된다.
- `IntersectionObserver`로 스크롤 진입 시 한 번만 애니메이션한다
  (`index.html:1896-1903`). 미지원 브라우저는 즉시 `paint()`.

**하드코딩 범위**: `LO`/`HI`가 상수다. 30,000 ms를 넘는 측정값이 생기면
막대가 100%에서 잘리고 축은 여전히 10,000 ms까지만 표시한다.

### 5.3 앵커 스무스 스크롤 — `index.html:1905-1914`

`prefers-reduced-motion`을 존중한다 (`index.html:1912`).

### 5.4 없는 것

- 다크/라이트 **토글 UI가 없다.** CSS에는 `:root[data-theme="light"]` /
  `[data-theme="dark"]` 블록이 있지만(`index.html:56-69`) `data-theme`를 세팅하는
  코드가 파일 어디에도 없다. 실제 동작은 `prefers-color-scheme`뿐이다.
- 폼도, 분석 스크립트도, 서비스 워커도 없다.

---

## 6. ★ 수치는 어디서 오나 — Studio와 정반대다

| | 랜딩 사이트 (`web/index.html`) | Studio 리포트 (`portal/web/benchmark.*`) |
|---|---|---|
| 데이터 출처 | **파일 안에 손으로 타이핑** | `GET /api/benchmark` → `bench/results/` |
| 갱신 방법 | 사람이 44개 `data-ms` + 44개 텍스트 + 9개 표를 고침 | 하네스를 다시 돌리면 끝 |
| 드리프트 가능성 | **있음** | (설계상) 없음 — 실제로는 [05번 문서](05_benchmark_report.md) §5 참조 |
| 축 | 로그 (`index.html:1862`) | 선형 + 25배 클리핑 (`benchmark.js:152-154`) |

### 6.1 같은 수치가 파일 안에서 두 번 나온다

```html
<!-- web/index.html:748 -->
<div class="bar-row s-og"><span class="nm">ontological</span>
  <div class="bar-track"><div class="bar-fill" data-ms="3.96"></div></div>
  <span class="vv num">3.96 ms</span></div>
```

`data-ms="3.96"`(막대 길이)와 `3.96 ms`(표시 텍스트)가 **독립적으로** 존재한다.
44개 막대 전부 그렇다. 한쪽만 고치면 막대와 숫자가 어긋나고, 아무도 알아채지 못한다.
(`FE-12`)

### 6.2 갱신 시 함께 고쳐야 하는 곳

수치를 바꾸면 다음이 **전부** 손봐야 하는 대상이다:

| 위치 | 무엇 |
|---|---|
| `index.html:461, 465` | 히어로 지표 `3.96 ms` / `270 ms` |
| `index.html:462, 466, 470, 474` | 지표 설명문의 규모·엔진 수 |
| `index.html:478-486` | 히어로 각주의 버전·조건 |
| `index.html:739-773` | 얕은 홉 차트 막대 24개 (`data-ms` + 텍스트) |
| `index.html:779-786` | 그 각주 (프로토콜 바닥값 3개 포함) |
| `index.html:805-808` | 논리 페이지 표 |
| `index.html:816-819` | 그 각주 안의 페이지 수치 |
| `index.html:854, 860` | 6,700만 행 / 50,000 행 |
| `index.html:923` | `1.9 ms → 4.9 ms` |
| `index.html:949-972` | 깊은 홉 차트 막대 20개 |
| `index.html:978-1002` | 그 각주 (pgGraph 분해, AGE 과대평가 90 ms) |
| `index.html:1029-1032`, `1045-1048` | 체인/격자 지름 표 |
| `index.html:1053-1062` | 그 각주 |
| `index.html:1084-1087`, `1101-1104` | 25만 엔진 비교 표 2개 |
| `index.html:1109-1126` | 그 각주 (pgGraph 제곱 증가 5개 값 포함) |
| `index.html:1136-1188` | `.honest` 패널의 8개 항목 안 수치 전부 |
| `index.html:1656` | **푸터의 날짜** |

푸터는 현재 이렇게 적혀 있다:

```html
<!-- web/index.html:1656 -->
<span>모든 수치: 2026-08-06 · 50,000 노드 / 974,936 엣지 · 정답 일치 검증 완료</span>
```

그런데 03.4~03.6절의 깊은 홉·지름 수치는 `bench/results/`의 `2026-08-17` 실행에서 온 것으로
보인다 (`bench-50000-20260817T033001Z.json`, `bench-250000-20260817T052859Z.json`).
**푸터의 "모든 수치: 2026-08-06"은 이미 정확하지 않다.** (`FE-12`)

> 수치 인용 시에는 반드시 [`docs/benchmark.md`](../benchmark.md)와
> [`docs/deep-traversal.md`](../deep-traversal.md)를 직접 읽어 확인한다.
> 이 문서는 수치를 재게시하지 않는다.

---

## 7. 접근성 — 잘 되어 있는 것과 아닌 것

### 7.1 되어 있는 것

| 항목 | 근거 |
|---|---|
| `:focus-visible` 전역 아웃라인 | `index.html:414` |
| `prefers-reduced-motion` 존중 (CSS + JS 양쪽) | `index.html:410-413`, `1912` |
| SVG에 `aria-label` | `index.html:420, 1759` |
| 탭 UI에 `role="tablist"` / `role="tab"` / `aria-selected` | `index.html:510-514, 525-529`, JS `1835-1836` |
| 색에만 의존하지 않는 인코딩 (모든 막대 행에 텍스트 라벨) | `index.html:312-314` 주석이 명시 |
| `text-wrap: balance`, `max-width: 62ch~74ch` 가독폭 | `index.html:103, 105` |

### 7.2 아닌 것

| 항목 | 근거 |
|---|---|
| `lang` 없음 | §4.3 |
| viewport 없음 → 모바일에서 반응형 CSS 전멸 | §4.2 |
| `role="tab"`에 `aria-controls`도 `tabpanel`도 없다 | `index.html:526-528` — 패널(`#pane-out`)에 `role="tabpanel"`이 없다 |
| 탭 로빙 포커스(←/→ 키) 없음 | JS에 키보드 핸들러가 없다 (`index.html:1850-1855`는 `click`만) |
| 860px 이하에서 내비게이션이 **사라진다** (대체 없음) | `index.html:128` |
| `table.data { min-width: 620px }` + `.tablewrap { overflow-x: auto }` | `index.html:285-286` — 표는 가로 스크롤로 처리. 스크롤 컨테이너에 `tabindex="0"`가 없어 키보드로 스크롤할 수 없다 |
| `#pane-out` 갱신에 `aria-live` 없음 | 탭을 바꿔도 스크린 리더가 새 내용을 알리지 않는다 |

---

## 8. 유지보수 규칙

### 필수 (Required)

- 이 파일은 **자기완결적**으로 유지한다. 외부 스크립트·폰트·이미지를 추가하지 않는다
  (현재 외부 참조 0건).
- 새 섹션을 추가하면 `.shead .rail`에 번호를 붙이고 `.topnav`(`index.html:426-436`)에
  링크를 추가한다.
- 막대를 추가/수정할 때는 **`data-ms` 속성과 `.vv` 텍스트를 같은 값으로** 함께 고친다 (§6.1).
- 새 시스템 색은 `.s-<key>` 클래스를 `index.html:315-328`에 추가하고,
  **텍스트 라벨을 반드시 함께** 넣는다 (색만으로 식별되게 하지 않는다).
- 수치를 갱신하면 **푸터의 날짜(`index.html:1656`)를 함께 갱신**한다.
- 수치의 출처는 `bench/results/`다. 이 파일의 값을 바꿀 때는 해당 JSON을 열어 확인한다.

### 금지 (Forbidden)

- `esc()`(`index.html:1813`)를 이스케이프 함수로 신뢰하지 않는다. `&`와 `<`만 처리한다.
  외부 데이터를 이 페이지에 넣으려면 새 함수를 쓴다.
- `QUERIES` 상수(`index.html:1673-1756`)의 `cypher`/`sql` 필드에 **이스케이프되지 않은
  사용자 데이터를 넣지 않는다.** 이 필드들은 HTML로 취급된다.
- 로그 축 상수 `LO`/`HI`(`index.html:1862`)를 두 차트 중 한쪽만 바꾸지 않는다.
  공통 축이 의도다 (`index.html:1870-1871`).
- `.honest` 패널에서 불리한 수치를 빼지 않는다. 이 사이트의 논지가 그 패널에 걸려 있다
  (`index.html:1131-1189`, `1390-1399`).

---

## 9. 미확인

- 이 파일이 어떻게 배포되는지 (CI, 호스팅, HTTP 헤더). 저장소에 설정이 없다.
  따라서 §4.1의 charset 문제가 실제로 발생하는지도 미확인이다.
- 03.5~03.6절의 지름 수치가 정확히 어느 결과 파일에서 왔는지는 파일명 대조로만 추정했다.
  `bench/csr/deep.py` 출력은 `bench/results/`에 JSON으로 남지 않는 것으로 보인다.
- `010` 스펙이 현황 목록에서 빠진 것이 의도인지 누락인지.

<!-- affects: frontend, docs -->
<!-- requires-update: docs/04_frontend/05_benchmark_report.md, docs/benchmark.md -->
