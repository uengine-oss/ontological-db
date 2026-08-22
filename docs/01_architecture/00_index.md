# 01_architecture — 아키텍처

> **이 문서가 답하는 질문**
> - 이 카테고리는 무엇에 답하는가?
> - 어떤 질문이 들어오면 어느 문서를 봐야 하는가?
> - "왜 이 경계가 여기 있는가"에 대한 답은 어디에 있는가?

---

## 이 카테고리의 원칙

`01_architecture/` 는 **"무엇이 어디 있는가"가 아니라 "왜 이 경계가 존재하는가"** 에 답한다.
"무엇이 어디 있는가"는 [`../00_overview/04_repository_map.md`](../00_overview/04_repository_map.md) 의 몫이다.

이 프로젝트에서 경계는 대부분 **헌법 원칙 사이의 충돌을 해결한 흔적**이다.
예: Cypher가 함수 호출로 진입하는 이유는 원칙 I(포크 금지)이 원칙 II(Cypher 1급)를
이겼기 때문이고, 그 판단은 `specs/003-*/plan.md` 의 Complexity Tracking에 기록되어 있다.
따라서 이 카테고리의 각 문서는 **어떤 원칙 충돌을 해결한 것인지**를 함께 밝힌다.

---

## 문서 목록

| 문서 | 답하는 질문 |
|---|---|
| [`01_system_overview.md`](01_system_overview.md) | 논리 아키텍처와 물리 아키텍처는 각각 무엇인가? 프로세스는 몇 개인가? |
| [`02_layer_boundaries.md`](02_layer_boundaries.md) | 왜 읽기는 SQL, 쓰기는 Rust인가? 왜 이 경계를 넘으면 안 되는가? |
| [`03_query_pipeline.md`](03_query_pipeline.md) | Cypher/TypeQL 한 줄이 디스크 읽기가 되기까지 무슨 일이 일어나는가? |
| [`04_storage_architecture.md`](04_storage_architecture.md) | 인접 정보, 프로퍼티, 뷰는 물리적으로 어떻게 배치되는가? |
| [`05_type_system_architecture.md`](05_type_system_architecture.md) | 상속 DAG와 구간 라벨은 어떻게 상수 시간 판정을 만드는가? |
| [`06_protocol_surfaces.md`](06_protocol_surfaces.md) | 진입면이 넷인데, 각각 무엇을 보장하고 무엇을 보장하지 않는가? |
| [`07_failure_isolation.md`](07_failure_isolation.md) | 무엇이 무엇을 무너뜨릴 수 있는가? 동기/비동기 경계는 어디인가? |
| [`08_improvements_architecture.md`](08_improvements_architecture.md) | ★ 지금 아키텍처의 구조적 부담은 무엇이고 어떻게 고칠 것인가? |

---

## 질문 → 문서 라우팅

| 질문 | 문서 |
|---|---|
| "Cypher가 어떤 SQL이 되나?" | [`03_query_pipeline.md`](03_query_pipeline.md) |
| "왜 홉이 싼가?" | [`04_storage_architecture.md`](04_storage_architecture.md) |
| "`MATCH (v:Vehicle)` 이 왜 Car를 찾나?" | [`05_type_system_architecture.md`](05_type_system_architecture.md) |
| "Bolt로 붙으면 트랜잭션이 어떻게 되나?" | [`06_protocol_surfaces.md`](06_protocol_surfaces.md) |
| "임베딩 서버가 죽으면?" | [`07_failure_isolation.md`](07_failure_isolation.md) |
| "왜 여기에 코드를 넣으면 안 되나?" | [`02_layer_boundaries.md`](02_layer_boundaries.md) |
| "이거 왜 이렇게 되어 있나? 고칠 수 있나?" | [`08_improvements_architecture.md`](08_improvements_architecture.md) |

---

## 원문 근거

영문 아키텍처 문서 [`docs/architecture.md`](../architecture.md) 가 다이어그램과 함께
같은 내용을 다룬다. **한 곳에서 서로 다르게 말하면 코드가 옳다.**
현재 알려진 불일치는 `docs/architecture.md` 의 "`WITH` and `UNION` are not implemented" —
`WITH` 은 구현되어 있다
([`../00_overview/05_spec_status.md`](../00_overview/05_spec_status.md) 참조).

---

## Forbidden / Required

**Forbidden**
- 함수 시그니처를 여기서 정의하지 말 것 — `02_api/` 의 책임이다.
- 경계를 서술할 때 "왜"를 빼지 말 것. "무엇"만 있으면 이 카테고리가 아니다.

**Required**
- 새 경계를 추가하면 그것이 해결한 **헌법 원칙 충돌**을 명시할 것.
- 경계를 바꾸면 ADR을 `99_decisions/` 에 남길 것.

<!-- affects: architecture -->
<!-- requires-update: 99_decisions/ -->
