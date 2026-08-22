# 04_frontend — 프런트엔드 문서 색인

> **이 문서가 답하는 질문**
> - 이 프로젝트의 프런트엔드는 몇 개이고, 각각 무엇을 책임지는가?
> - 백엔드를 고치기 전에 반드시 읽어야 할 문서는 어느 것인가?
> - 어떤 필드를 바꾸면 화면이 깨지는가 — 그 답은 어느 문서에 있는가?

---

## 1. 사실 — 프런트엔드는 두 개다

| # | 이름 | 실체 | 규모 | 서빙 주체 |
|---|---|---|---|---|
| 1 | **Studio 콘솔** | `portal/` (Node.js 백엔드 + 빌드 없는 순수 JS 프런트) | 서버 376줄 / 프런트 880줄 + 378줄 | `portal/server/index.js` 자신이 정적 파일까지 서빙 |
| 2 | **랜딩/문서 사이트** | `web/index.html` — HTML·CSS·JS가 한 파일 | 1,916줄 | 미확인 (저장소에 배포 설정이 없음) |

두 프런트엔드는 **코드를 한 줄도 공유하지 않는다.** 색 팔레트도, 폰트 스택도, 벤치마크 수치의 출처도
서로 다르다. 이 사실이 이 문서 묶음 전체의 전제다.

- Studio 팔레트: `portal/web/style.css:2-14` (`--accent: #18b6a0`)
- 랜딩 팔레트: `web/index.html:9-33` (`--accent: #059669` / 다크 `#10b981`)

## 2. 이 문서 묶음의 목적

프런트엔드 문서가 없으면 **백엔드가 API를 임의로 바꾼다.** 특히 이 프로젝트는
`og_cypher()`가 돌려주는 jsonb의 키 이름(`_id` / `_type` / `_src` / `_dst`)이
그래프 시각화의 유일한 입력이라, 엔진의 `og_node_json` 한 줄을 고치면 Studio의
그래프 뷰가 조용히 빈 화면이 된다.

그래서 이 묶음의 초점은 **화면 구조가 아니라 상태 흐름과 API 연결 규칙**이다.

## 3. 읽는 순서

| 문서 | 언제 읽나 |
|---|---|
| [01_studio_architecture.md](01_studio_architecture.md) | Studio를 처음 띄울 때 / 왜 빌드 스텝이 없는지 궁금할 때 |
| [02_state_flow.md](02_state_flow.md) ★ | 질의 입력부터 결과 렌더까지의 흐름, 상태가 어디 있는지 |
| [03_api_contract_rules.md](03_api_contract_rules.md) ★ | **백엔드/엔진을 고치기 전에 반드시** |
| [04_graph_rendering.md](04_graph_rendering.md) | 그래프가 안 그려질 때 / 대량 결과가 느릴 때 |
| [05_benchmark_report.md](05_benchmark_report.md) | `bench/harness.py`의 출력 스키마를 바꿀 때 |
| [06_landing_site.md](06_landing_site.md) | `web/index.html`을 고칠 때 |
| [07_improvements_frontend.md](07_improvements_frontend.md) ★ | 개선 작업을 고를 때 |

★ = 이 묶음에서 가장 중요한 세 문서.

## 4. 원문 근거 (영문 문서)

이 묶음은 아래 기존 영문 문서를 **대체하지 않는다.** 중복되는 곳은 원문으로 링크한다.

- [`docs/api.md`](../api.md) — 모든 SQL 함수의 시그니처
- [`docs/benchmark.md`](../benchmark.md) — 벤치마크 방법론
- [`docs/architecture.md`](../architecture.md) — 엔진 전체 구조
- [`docs/images/studio.png`](../images/studio.png) — Studio 스크린샷 (참조용)

## 5. 전 문서 공통 규칙

### 필수 (Required)

- 모든 주장에는 `파일:라인` 근거를 붙인다.
- 코드/식별자/SQL/경로는 원문 그대로 둔다. 번역하지 않는다.
- 코드 블록 안의 주석은 영어로 쓴다 (프로젝트 규칙).
- 확인하지 못한 것은 **"미확인"**이라고 명시한다.

### 금지 (Forbidden)

- 실행해 보지 않은 동작을 "동작한다"고 쓰지 않는다.
- 벤치마크 수치를 이 문서에 다시 타이핑하지 않는다. `docs/benchmark.md`와
  `bench/results/`가 유일한 출처다.
- 이 묶음의 문서는 **코드를 수정하지 않는다.** 개선 제안은
  [07_improvements_frontend.md](07_improvements_frontend.md)에 표로만 남긴다.

## 6. 확인된 결함 요약 (상세는 07번 문서)

이 묶음을 쓰면서 실제 코드/실행으로 확인한 것들:

| 심각도 | 무엇 |
|---|---|
| High | `POST /api/expand`의 SQL이 **PostgreSQL 문법 오류** — 그래프 더블클릭 확장이 100% 실패 (`portal/server/index.js:259-269`) |
| High | 벤치마크 리포트 렌더러의 질의 키가 현재 하네스 출력과 어긋나 **기본 화면이 빈 차트** (`portal/web/benchmark.js:22-29`) |
| High | 정답 게이트를 서버가 하네스와 **다르게** 재계산해 정상 결과를 빨간 경고로 표시 (`portal/server/index.js:127` vs `bench/harness.py:1089`) |
| High | 타입 이름이 이스케이프 없이 HTML 속성에 들어감 — 저장형 XSS 면 (`portal/web/app.js:85,95,131`) |
| High | `web/index.html`에 `<!doctype>` · `<meta charset>` · `<meta viewport>`가 **전부 없음** |

<!-- affects: frontend, api, docs -->
<!-- requires-update: docs/04_frontend/03_api_contract_rules.md -->
