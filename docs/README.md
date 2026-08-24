# Ontological 문서

> **이 문서가 답하는 질문**
> - 내가 지금 필요한 문서는 어디 있는가?
> - 무엇을 바꾸기 전에 무엇을 읽어야 하는가?
> - 이 저장소에서 지금 가장 시급한 문제는 무엇인가?

이 폴더는 **역할 × 관심사 × 시점**으로 나뉜 10개 카테고리를 따릅니다.
문서는 타입을 설명하지 않고 **질문에 답합니다.** 각 문서 첫머리에
"이 문서가 답하는 질문"이 있으니 그것으로 찾으세요.

---

## 카테고리 지도

| 카테고리 | 답하는 질문 | 문서 |
|---|---|---|
| [00_overview/](00_overview/00_index.md) | 이 프로젝트는 무엇인가? | 6 |
| [01_architecture/](01_architecture/00_index.md) | 왜 이렇게 설계했는가? | 9 |
| [01_architecture/09_performance/](01_architecture/09_performance/00_index.md) | 성능은 어디서 나오고 어디서 새는가? | 8 |
| [02_api/](02_api/00_index.md) | 계약은 무엇인가? | 13 |
| [03_backend/](03_backend/00_index.md) | 서버 내부는 어떻게 생겼는가? | 12 |
| [04_frontend/](04_frontend/00_index.md) | UI는 왜 이렇게 동작하는가? | 8 |
| [05_llm/](05_llm/00_index.md) | 에이전트/RAG는 이 DB를 어떻게 쓰는가? | 11 |
| [06_data/](06_data/00_index.md) | 데이터는 무엇을 의미하는가? | 11 |
| [07_security/](07_security/00_index.md) | 어디가 깨지는가? | 11 |
| [08_operations/](08_operations/00_index.md) | 어떻게 띄우고 고치는가? | 11 |
| [99_decisions/](99_decisions/00_index.md) | 왜 그렇게 했는가? (ADR 25건) | 26 |

기존 영문 문서 — [architecture.md](architecture.md), [api.md](api.md),
[cypher.md](cypher.md), [typeql.md](typeql.md), [benchmark.md](benchmark.md),
[deep-traversal.md](deep-traversal.md), [comparison.md](comparison.md),
[agents.md](agents.md) — 는 그대로 두었고, 위 한글 문서가 이들을 근거로 인용합니다.
**단, 아래 §정오표에 적힌 부분은 코드와 어긋나 있습니다.**

---

## 역할별 진입점

| 당신이 | 여기부터 |
|---|---|
| 이 프로젝트를 처음 본다 | [00_overview/01_what_is_ontological.md](00_overview/01_what_is_ontological.md) → [00_overview/03_glossary.md](00_overview/03_glossary.md) |
| 스토리지를 건드린다 | [06_data/01_physical_schema.md](06_data/01_physical_schema.md) → [ADR-004](99_decisions/ADR-004-csr-adjacency-segments.md), [ADR-005](99_decisions/ADR-005-typed-property-columns.md) |
| Cypher 컴파일러를 건드린다 | [03_backend/04_cypher_compiler.md](03_backend/04_cypher_compiler.md) → [ADR-012](99_decisions/ADR-012-visited-set-bfs-rewrite.md), [ADR-013](99_decisions/ADR-013-conservative-bfs-rewrite.md) |
| 성능 회귀를 쫓는다 | [01_architecture/09_performance/03_hot_paths.md](01_architecture/09_performance/03_hot_paths.md) |
| 배포/운영을 맡는다 | [08_operations/01_install.md](08_operations/01_install.md) → [07_security/08_secure_deployment.md](07_security/08_secure_deployment.md) |
| 코드 리뷰를 한다 | [03_backend/10_coding_rules.md](03_backend/10_coding_rules.md) |
| 에이전트를 붙인다 | [05_llm/01_agent_native_interface.md](05_llm/01_agent_native_interface.md) |

---

## 개선 포인트 230건

카테고리별 개선 문서에 **근거 파일:라인 / 현상 / 제안 / 예상 효과 / 리스크**를 갖춘
표로 정리되어 있습니다.

| 접두사 | 건수 | 문서 |
|---|---|---|
| `ARCH-` | 18 | [01_architecture/08_improvements_architecture.md](01_architecture/08_improvements_architecture.md) |
| `PERF-` | 30 | [01_architecture/09_performance/07_improvements_performance.md](01_architecture/09_performance/07_improvements_performance.md) |
| `API-` | 32 | [02_api/12_improvements_api.md](02_api/12_improvements_api.md) |
| `CODE-` | 37 | [03_backend/11_improvements_code.md](03_backend/11_improvements_code.md) |
| `FE-` | 21 | [04_frontend/07_improvements_frontend.md](04_frontend/07_improvements_frontend.md) |
| `LLM-` | 18 | [05_llm/10_improvements_llm.md](05_llm/10_improvements_llm.md) |
| `DATA-` | 22 | [06_data/10_improvements_data.md](06_data/10_improvements_data.md) |
| `SEC-` | 34 | [07_security/09_improvements_security.md](07_security/09_improvements_security.md) |
| `OPS-` | 18 | [08_operations/10_improvements_ops.md](08_operations/10_improvements_ops.md) |

---

## 먼저 볼 것: 검증된 상위 결함

아래 항목은 문서 작성 중 발견되어 **코드에서 직접 재확인한 것들**입니다.
정답성을 해치는 것부터 나열합니다.

> **정답성 5건은 전부 수정되었고, 보안 항목도 일부 수정되었습니다.**
> 정답성은 [03_backend/12_fixed_correctness.md](03_backend/12_fixed_correctness.md),
> 보안은 [07_security/10_fixed.md](07_security/10_fixed.md) 가 현재 상태입니다.
>
> **보안 항목 일부는 이미 수정되었습니다.** Critical 5건은 전부, High 13건은
> 4건 수정 · 4건 부분 수정이고, 아래 보안 표에 상태를 표시했습니다. 무엇이 어떻게 바뀌었고 무엇이
> 남았는지는 [07_security/10_fixed.md](07_security/10_fixed.md) 에 있습니다.
> 설계 근거는 [ADR-025](99_decisions/ADR-025-privilege-model-default-deny.md).
> **정답성·운영 항목은 손대지 않았습니다.**

### 정답성 — 다섯 건 모두 수정됨

상태는 [03_backend/12_fixed_correctness.md](03_backend/12_fixed_correctness.md)
기준입니다. "증상" 열은 **감사 시점(`7d60c82`)의 기록**이며, 회귀 테스트는
[`engine/tests/sql/06_correctness_regressions.sql`](../engine/tests/sql/06_correctness_regressions.sql)
에 있으며, **수정 전 코드에서 다섯 단언이 전부 실패하는 것을 확인**했습니다.

| 항목 | 상태 | 근거 | 감사 시점의 증상 |
|---|---|---|---|
| `UNION`이 무시된다 | **수정** | [parser.rs:161](../engine/src/cypher/parser.rs#L161)이 `Query.union`을 채우지만 `grep -rn "\.union" engine/src` **소비자 0건** | 오류 없이 **첫 분기 행만** 반환. [bolt/README.md:75](../bolt/README.md#L75)는 "실패한다"고 적혀 있으나 실패하지 않는다 |
| 쓰기 질의의 `WITH` 무시 | **수정** | [cypher/mod.rs:171-176](../engine/src/cypher/mod.rs#L171-L176)의 `take_while(Match\|Unwind)` | `MATCH (n) WITH n LIMIT 1 DELETE n` 이 **전부 삭제** |
| `count(DISTINCT)` 오답 | **수정** | [cypher/mod.rs:355](../engine/src/cypher/mod.rs#L355)의 `values.dedup()` — 정렬 없이 호출 | 연속 중복만 제거되어 **틀린 수** |
| `*min..max`의 `min > 1` 발산 | **수정** | [compile.rs:865](../engine/src/cypher/compile.rs#L865)의 `prefer_reachability(max)`가 `min`을 보지 않음. [traverse.rs:144](../engine/src/storage/traverse.rs#L144)의 방문집합은 **최단 거리**만 방출 | `og_vlp`(트레일 길이)와 `og_reach`(최단 거리)가 다른 답. 회귀 스위트가 `*1..k`만 써서 **본 적이 없다** |
| 플랜 캐시가 스키마 변경에 무효화 안 됨 | **수정** | 캐시 키가 `(graph, query)`뿐 ([cypher/mod.rs:26-31](../engine/src/cypher/mod.rs#L26-L31)). `bump_schema_version()`은 `og_data.v_*`를 DROP | 캐시 히트 시 **폐기된 뷰를 참조하는 SQL 실행**. 프로퍼티 자동 승격조차 이 경로를 탄다 |

### 보안 — 대부분 수정됨

상태는 [07_security/10_fixed.md](07_security/10_fixed.md) 기준입니다. "증상" 열은
**감사 시점(`7d60c82`)에 무엇이 잘못돼 있었는지**의 기록입니다.

| 항목 | 상태 | 감사 시점의 증상 |
|---|---|---|
| `GRANT`/`REVOKE` 0줄 | **수정** | 모든 함수가 PUBLIC EXECUTE 인데 테이블에는 GRANT 가 하나도 없어, 닫혀야 할 곳이 열리고 열려야 할 곳이 닫혀 있었다. 이제 기본 거부 + `og_grant(role, level)` |
| `og_set_setting` 경유 SSRF | **수정** | `genai.endpoint` 를 바꿔 DB 백엔드가 임의 주소로 요청. 이제 admin 전용 |
| 생성 뷰에 `security_invoker` 없음 | **수정** (pg15+) | 라벨 `MATCH` 가 지나는 뷰에서 RLS 가 뷰 소유자 기준으로 평가. **pg13/pg14 에는 옵션 자체가 없어 그대로 남는다** |
| `FORCE ROW LEVEL SECURITY` 없음 | **수정** | `ENABLE` 단독은 테이블 소유자를 면제하는데, 저장 테이블 소유자가 곧 확장 설치자였다 |
| Studio 무인증 + 전 인터페이스 바인드 | **수정** | 임의 SQL 이 네트워크에 노출되면서 로그는 `http://localhost` 라고 표시. 이제 루프백 기본, raw SQL 은 옵트인, `/api/*` 에 동일 출처·content-type 검사 |
| Bolt 인증 전 입력 파싱 | **수정** | 길이 필드 기반 선할당, 메시지 길이 무제한, 재귀 깊이 무제한. 회귀 테스트 4건 추가 |
| Bolt `0.0.0.0` 기본 바인드 | **수정** | 평문 인증이 전 인터페이스에 노출. 이제 `127.0.0.1` 기본 |
| SQL 문자열 보간 3곳 | **부분** | [:338](../engine/src/vector/mod.rs#L338) `s.prop = '{prop}'` 은 바인딩으로 교체. `AND ({f})` 는 설계상 SQL 조각이고, `og_map_table` 의 표현식도 마찬가지 — 둘 다 권한으로 축소했을 뿐이다 |
| Bolt TLS 없음 | **미수정** | 자격증명이 평문으로 오간다. 루프백 기본이 완화일 뿐 해결이 아니다 |
| 레지스트리 테이블 RLS 없음 | **미수정** | 순회 계획이 바뀌므로 측정 없이 넣을 변경이 아니다 |

### 운영

| 항목 | 근거 | 증상 |
|---|---|---|
| 확장 업그레이드 경로 없음 | `ontological--*--*.sql` **0건** | `ALTER EXTENSION UPDATE` 불가 — 첫 스키마 변경 시 재설치 외 방법 없음 |
| 확장이 `ANALYZE`를 하지 않음 | `engine/` 전체에서 `ANALYZE`는 [cypher/mod.rs:682](../engine/src/cypher/mod.rs#L682)의 EXPLAIN 옵션 문자열 1건뿐 | `reltuples`가 0이면 [compile.rs:52](../engine/src/cypher/compile.rs#L52)가 깊은 순회 판단을 **"깊이 ≥ 4" 고정 규칙으로 폴백** |
| 회귀 게이트가 단언을 보지 않음 | [tests/run.sh:24](../tests/run.sh#L24)가 `^ERROR` 줄 **개수만** 비교 | SQL 단언이 `f`를 내도 통과. [:71-75](../tests/run.sh#L71-L75) 무결성 검사는 파이프 서브셸이라 종료 코드에 미반영 |
| CI 없음 | `.github` 디렉터리 부재 | 위 항목들이 잡히지 않은 이유 |
| pg_regress 스캐폴딩 파손 | [setup.sql:3](../engine/tests/pg_regress/sql/setup.sql#L3) `CREATE EXTENSION engine;` (pgrx 템플릿 잔재), `#[pg_test]` 0건 | pg13~pg19 feature 선언은 pg16 외 검증 근거 없음 |
| `og_drop_graph`가 고아 행을 남김 | [types.rs:322-340](../engine/src/catalog/types.rs#L322-L340). `og_data`에 FK 0건 | `og_check_integrity()`가 `orphan_node`로 보고하는 상태를 **엔진 자신이 만든다** |
| PGDATA 볼륨 없음 | [start.sh:21-28](../start.sh#L21-L28)이 만드는 볼륨은 빌드 캐시 2개뿐 | `docker rm` 한 번에 그래프 소실 |

### 선언되었으나 구현되지 않은 것

| 항목 | 근거 |
|---|---|
| `og.max_rows` 행 상한 | [agent/mod.rs:438](../engine/src/agent/mod.rs#L438)이 `SET`만 하고 **읽는 코드 0건** |
| `og_catalog.rule` 추론 | INSERT만 있고 **SELECT 0건** — transitive/symmetric 선언이 저장만 되고 추론 없음 |
| `og_explain_error`의 라벨 오타 교정 | [agent/mod.rs:295](../engine/src/agent/mod.rs#L295)가 `"unknown label"`을 찾지만 **그 문자열을 만드는 코드가 없음** ([types.rs:168](../engine/src/catalog/types.rs#L168)은 `notice!`) |
| `insert_between` 증분 라벨링 | [labeling.rs:115](../engine/src/catalog/labeling.rs#L115) 주석만 존재, **함수 없음** |
| `chunk_size`/`supernode_threshold`/`inference_max_depth` 설정 | 소비자 0건 |
| `og_reach_sql` | [access.sql:169](../engine/sql/access.sql#L169)에 정의되어 있으나 컴파일러가 선택하지 않음 |

---

## 정오표 — 기존 영문 문서와 코드의 불일치

| 문서 | 서술 | 코드 |
|---|---|---|
| [cypher.md:295](cypher.md#L295) | `ERROR: unknown label 'Persn' …` | 그런 `ERROR`가 없다. [types.rs:168](../engine/src/catalog/types.rs#L168)의 `NOTICE`이고 문구도 다르다 |
| [cypher.md:249](cypher.md#L249) | `UNSUPPORTED_SYNTAX` | `classify()`가 `"unknown function"`을 먼저 검사 → 실제 `UNKNOWN_FUNCTION` |
| [api.md:139](api.md#L139) | "Scores are normalised so higher is always better" | `l2`는 원 거리 그대로. `og_hybrid_search`는 l2에 `1/(1+d)`라는 세 번째 변환 |
| [api.md:178](api.md#L178) | `og_apply_role`의 row caps | `max_rows`를 읽는 코드가 없다 |
| [api.md:132](api.md#L132) | `og_similar(graph, …)` | [vector/mod.rs:173](../engine/src/vector/mod.rs#L173) `let _ = graph;` |
| [api.md:113](api.md#L113) | `og_typeql(…, params)` | [typeql/mod.rs:52](../engine/src/typeql/mod.rs#L52) `_params`, 미사용 |
| [architecture.md:264](architecture.md#L264) | "RLS가 순회 중간에 적용된다" | 감사 시점에는 생성 뷰에 `security_invoker` 가 없어 성립하지 않았다. **수정 후 pg15 이상에서는 성립하고, pg13/pg14 에서는 여전히 성립하지 않는다** |
| [benchmark.md:397](benchmark.md#L397) | 출처 `bench-50000-20260806T052220Z` | `bench/results/`에 없다 |
| [benchmark.md:325](benchmark.md#L325) | 124,580 edges/s | [harness.py:322](../bench/harness.py#L322)의 `og_data.*` 직접 INSERT 경로. **쓰기 API 수치가 아니다** |
| [deep-traversal.md](deep-traversal.md) | 전환 규칙 `Σ degreeⁱ > \|V\|`, "Depth ≥ 12" 분기 | 실제는 고정 `WALKS = 512`, 해당 분기 없음 |
| [specs/003](../specs/003-cypher-query-engine/plan.md), [architecture.md](architecture.md) | "`WITH` 미지원" | 읽기 경로는 지원. 낡은 서술 |
| [specs/001](../specs/001-graph-storage-engine/plan.md) | `CHUNK_SIZE = 1024` | [adjacency.rs:15](../engine/src/storage/adjacency.rs#L15) `256` |

---

## 이 문서들을 쓸 때의 규칙

- **문서는 완벽할 필요는 없지만 거짓이어서는 안 된다.** 확인하지 못한 것은 "미확인"으로 적혀 있다.
- 모든 주장에 `파일:라인` 근거가 붙어 있다. 근거가 없으면 그 문장은 신뢰하지 말 것.
- 결정(Decisions)과 사실(Facts)은 절 단위로 분리되어 있다.
- 각 문서 끝의 `<!-- affects: -->` / `<!-- requires-update: -->` 태그는 변경 영향 범위다.

<!-- affects: all -->
