# 03_backend — 서버 내부 구조

> **이 문서가 답하는 질문**
> - `docs/03_backend/`에는 무엇이 들어 있고, 무엇이 들어 있지 않은가?
> - 백엔드 코드를 처음 읽는 사람은 어떤 순서로 읽어야 하는가?
> - 코드 리뷰를 할 때 어떤 문서를 기준 문서로 삼아야 하는가?

---

## 이 카테고리의 역할

`03_backend/`는 **엔진 내부에서 실제로 무슨 일이 일어나는가**를 다룬다.
"어떤 SQL 함수를 호출하면 되는가"(= `02_api/`)나 "왜 이런 아키텍처인가"(= `01_architecture/`)가
아니라, **어느 파일 몇 번째 줄이 무엇을 하는가**를 다룬다.

대상 코드는 다음 세 덩어리다.

| 덩어리 | 경로 | 산출물 |
|---|---|---|
| PostgreSQL 확장 (Rust cdylib) | `engine/src/` | `ontological.so` + `ontological.control` |
| 부트스트랩 / 공개 접근 경로 (SQL) | `engine/sql/bootstrap.sql`, `engine/sql/access.sql` | 확장 설치 스크립트 |
| Bolt 게이트웨이 (별도 바이너리) | `bolt/src/` | `ontological-bolt` |

`portal/`(Studio), `web/`(랜딩), `bench/`(벤치마크 하네스)는 이 카테고리 밖이다.

---

## 문서 목록

| 문서 | 답하는 질문 | 주 독자 |
|---|---|---|
| [`01_module_map.md`](01_module_map.md) | 모듈이 몇 개이고 누가 누구를 부르는가? | 신규 개발자, LLM |
| [`02_write_path.md`](02_write_path.md) | 쓰기는 왜 Rust이고, 한 트랜잭션에서 무엇을 잠금-스텝으로 유지하는가? | 스토리지 담당 |
| [`03_cypher_frontend.md`](03_cypher_frontend.md) | Cypher 텍스트가 AST가 되기까지 무엇을 통과하는가? | 파서 담당 |
| [`04_cypher_compiler.md`](04_cypher_compiler.md) | ★ `compile.rs` 1,591줄은 정확히 무엇을 하는가? | 전원 |
| [`05_typeql_engine.md`](05_typeql_engine.md) | TypeQL의 7개 스테이지는 어떻게 실행되는가? | TypeQL 담당 |
| [`06_bolt_gateway.md`](06_bolt_gateway.md) | Neo4j 드라이버가 붙었을 때 무슨 일이 벌어지는가? | 프로토콜 담당 |
| [`07_transactions_and_concurrency.md`](07_transactions_and_concurrency.md) | 트랜잭션 경계는 어디이고 무엇이 동시성에 안전하지 않은가? | 전원 |
| [`08_error_handling.md`](08_error_handling.md) | 오류는 어디서 어떤 형태로 사용자에게 도달하는가? | 전원 |
| [`09_testing_strategy.md`](09_testing_strategy.md) | 무엇이 테스트되고 무엇이 테스트되지 않는가? | 전원 |
| [`10_coding_rules.md`](10_coding_rules.md) | ★ 이 코드베이스에서 무엇이 금지이고 무엇이 필수인가? | 리뷰어 |
| [`11_improvements_code.md`](11_improvements_code.md) | ★ 지금 코드에서 무엇이 문제인가? | 메인테이너 |

---

## 읽는 순서

**처음 읽는 경우**

1. [`01_module_map.md`](01_module_map.md) — 지도를 먼저 본다.
2. [`04_cypher_compiler.md`](04_cypher_compiler.md) — 이 프로젝트의 심장이다. 여기를 이해하면 나머지는 파생이다.
3. [`02_write_path.md`](02_write_path.md) — 읽기 경로와 쓰기 경로가 왜 다른 언어로 쓰였는지가 여기서 갈린다.
4. [`10_coding_rules.md`](10_coding_rules.md) — 코드를 고치기 전에 반드시.

**코드 리뷰를 하는 경우**

[`10_coding_rules.md`](10_coding_rules.md)의 금지/필수 표를 체크리스트로 쓰고,
지적 사항이 이미 알려진 것인지 [`11_improvements_code.md`](11_improvements_code.md)에서 확인한다.

**LLM 에이전트가 이 저장소에 대해 답하는 경우**

`01` → `04` → `10` 순으로 읽는다. 특히 `11_improvements_code.md`에 열거된 항목은
**"이 코드는 이미 이렇게 되어 있다"가 아니라 "이 코드는 이런 문제가 있다"**임에 주의한다.

---

## 이 카테고리가 다루지 않는 것

- **공개 SQL 함수의 시그니처와 사용법** → `02_api/`
- **왜 PostgreSQL 확장인가, 왜 포크하지 않았는가** → `01_architecture/`
- **벤치마크 수치** → [`docs/benchmark.md`](../benchmark.md), [`docs/deep-traversal.md`](../deep-traversal.md)
- **프론트엔드(Studio, 랜딩 사이트)** → `04_frontend/`

---

## 기존 영문 문서와의 관계

이 카테고리는 다음 영문 문서를 **대체하지 않고 참조**한다. 수치나 설계 의도가 충돌하면
영문 문서를 원문 근거로 본다.

- [`docs/architecture.md`](../architecture.md) — 전체 아키텍처
- [`docs/cypher.md`](../cypher.md) — Cypher 지원 범위와 Neo4j와의 알려진 차이
- [`docs/deep-traversal.md`](../deep-traversal.md) — 깊은 순회 재작성의 근거와 측정
- [`docs/typeql.md`](../typeql.md) — TypeQL 지원 범위
- [`docs/api.md`](../api.md) — 공개 함수 목록

<!-- affects: backend -->
