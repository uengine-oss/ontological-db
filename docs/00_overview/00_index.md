# 00_overview — 개요

> **이 문서가 답하는 질문**
> - 이 카테고리에는 무엇이 들어 있고, 무엇이 들어 있지 않은가?
> - 처음 이 저장소를 여는 사람은 어떤 순서로 읽어야 하는가?
> - LLM 에이전트가 이 프로젝트의 컨텍스트를 얻으려면 어느 문서부터 읽어야 하는가?

---

## 이 카테고리의 역할

`00_overview/`는 **역할과 무관하게 모두가 먼저 읽는** 층이다.
"무엇을 만들었는가", "왜 만들었는가", "지금 어디까지 왔는가"에 답하며,
구현 세부(어떤 함수가 어떤 SQL을 뱉는가)는 다루지 않는다.

LLM 에이전트가 이 저장소에 대해 답변을 생성할 때 **우선순위 1 컨텍스트**로 읽어야 하는
문서들이다. 특히 [`05_spec_status.md`](05_spec_status.md)는 "무엇이 아직 안 되는가"의
단일 진실 원천이므로, 기능 지원 여부를 답하기 전에 반드시 참조해야 한다.

---

## 문서 목록

| 문서 | 답하는 질문 | 주 독자 |
|---|---|---|
| [`01_what_is_ontological.md`](01_what_is_ontological.md) | 이 시스템은 무엇이고 어떤 문제를 푸는가? | 전원 (비개발자 포함) |
| [`02_positioning.md`](02_positioning.md) | Neo4j / Apache AGE / TypeDB / pgGraph 와 무엇이 다른가? | 도입 검토자, 아키텍트 |
| [`03_glossary.md`](03_glossary.md) | 이 저장소의 용어는 정확히 무엇을 가리키는가? | 전원, 특히 LLM |
| [`04_repository_map.md`](04_repository_map.md) | 어떤 코드가 어디에 있고 누가 무엇을 책임지는가? | 개발자 |
| [`05_spec_status.md`](05_spec_status.md) | 11개 스펙 중 무엇이 되고 무엇이 안 되는가? | 전원, 특히 LLM |

---

## 읽는 순서

**처음 온 사람**
1. [`01_what_is_ontological.md`](01_what_is_ontological.md)
2. [`02_positioning.md`](02_positioning.md)
3. [`../01_architecture/01_system_overview.md`](../01_architecture/01_system_overview.md)

**코드를 고치러 온 사람**
1. [`04_repository_map.md`](04_repository_map.md)
2. [`05_spec_status.md`](05_spec_status.md)
3. [`../01_architecture/02_layer_boundaries.md`](../01_architecture/02_layer_boundaries.md)
4. [`../01_architecture/08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md)

**LLM 에이전트**
1. [`03_glossary.md`](03_glossary.md) — 용어 고정
2. [`05_spec_status.md`](05_spec_status.md) — 지원 범위 고정
3. 질문 도메인에 해당하는 `01_architecture/` 문서

---

## Facts

- 이 카테고리의 모든 주장은 저장소 안의 파일에 근거한다. 근거는 `파일:라인` 형식으로 붙는다.
- 프로젝트 거버넌스 문서는 [`.specify/memory/constitution.md`](../../.specify/memory/constitution.md)
  (버전 1.0.0, 2026-08-05 비준)이며, 이 문서들보다 우선한다.
- 영문 원문 문서는 [`docs/architecture.md`](../architecture.md),
  [`docs/comparison.md`](../comparison.md), [`docs/benchmark.md`](../benchmark.md),
  [`docs/deep-traversal.md`](../deep-traversal.md), [`docs/cypher.md`](../cypher.md),
  [`docs/typeql.md`](../typeql.md), [`docs/api.md`](../api.md),
  [`docs/agents.md`](../agents.md) 에 있다. 한국어 문서는 이들을 **대체하지 않고 참조**한다.

## Decisions

- **결정 1**: 한국어 문서 세트는 `docs/00_overview/` ~ `docs/99_decisions/` 로 분리 신설한다.
  기존 영문 `docs/*.md`는 README가 직접 링크하므로 유지·수정하지 않는다.
- **결정 2**: 숫자(벤치마크, 라인 수)는 원본 파일에서 실측한 값만 적는다.
  실측하지 못한 값은 "미확인"이라고 쓴다.

---

## Forbidden / Required

**Forbidden**
- 이 카테고리 문서에서 구현 함수 시그니처를 정의하지 말 것 — 그것은 `02_api/`의 책임이다.
- 스펙 상태를 여기서 임의로 갱신하지 말 것. [`05_spec_status.md`](05_spec_status.md) 와
  루트 [`README.md`](../../README.md) 의 표가 동시에 갱신되어야 한다.

**Required**
- 새 기능이 병합되면 [`05_spec_status.md`](05_spec_status.md) 를 같은 PR에서 갱신할 것.
- 새 용어가 코드에 등장하면 [`03_glossary.md`](03_glossary.md) 에 추가할 것.

<!-- affects: overview, architecture -->
<!-- requires-update: 00_overview/05_spec_status.md, 01_architecture/00_index.md -->
