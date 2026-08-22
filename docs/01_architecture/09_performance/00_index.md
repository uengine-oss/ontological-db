# 성능 (09_performance) — 색인

> **이 문서가 답하는 질문**
> - 이 폴더의 문서들은 각각 무엇을 답하는가, 어떤 순서로 읽어야 하는가?
> - 어떤 수치가 **측정된 사실**이고, 어떤 것이 **추정**이며, 어떤 것이 **미확인**인가?
> - 성능에 대해 주장할 때 반드시 지켜야 할 규칙은 무엇인가?

---

## 1. 사실 — 이 폴더의 구성

| 문서 | 답하는 질문 |
|---|---|
| [`01_performance_model.md`](01_performance_model.md) | 한 홉의 비용은 무엇으로 구성되는가. Apache AGE와 무엇이 다르고 무엇이 같은가 |
| [`02_measured_baselines.md`](02_measured_baselines.md) | 실제로 측정된 수치는 무엇이고, 어떤 조건에서 나온 것인가 |
| [`03_hot_paths.md`](03_hot_paths.md) | 읽기·쓰기 핫패스가 코드 수준에서 정확히 무엇을 하는가. SPI 왕복은 몇 번인가 |
| [`04_deep_traversal_mechanics.md`](04_deep_traversal_mechanics.md) | 트레일 열거 / 방문집합 BFS / 컴파일 CSR 세 경로의 조건·비용·정확성 |
| [`05_planner_interaction.md`](05_planner_interaction.md) | PostgreSQL 플래너가 이 SQL을 어떻게 보는가. 오추정은 어디서 생기는가 |
| [`06_regression_guard.md`](06_regression_guard.md) | 성능 회귀를 막는 장치와 그 공백 |
| [`07_improvements_performance.md`](07_improvements_performance.md) | ★ 성능 개선 포인트 30건 (`PERF-01` ~ `PERF-30`) |

## 2. 사실 — 근거 자료의 위치

이 폴더의 모든 수치는 다음 중 하나에서 온다. 다른 출처는 없다.

| 종류 | 경로 |
|---|---|
| 원문 벤치마크 서술 | [`docs/benchmark.md`](../../benchmark.md), [`docs/deep-traversal.md`](../../deep-traversal.md), [`docs/comparison.md`](../../comparison.md) |
| 하네스 | [`bench/harness.py`](../../../bench/harness.py), [`bench/csr/deep.py`](../../../bench/csr/deep.py) |
| 원시 측정 결과 | [`bench/results/*.json`](../../../bench/results/), [`bench/csr/results/*.json`](../../../bench/csr/results/) |
| 핫패스 코드 | `engine/src/cypher/compile.rs`, `engine/src/storage/{mod,adjacency,traverse}.rs`, `engine/sql/{access,bootstrap}.sql` |

## 3. 결정 — 사실 / 추정 / 미확인의 구분 규칙

이 프로젝트의 존재 이유가 성능이므로, 성능 문서의 신뢰도 기준은 다른 문서보다 엄격하다.

- **사실(측정됨)** — `bench/results/` 또는 `bench/csr/results/` 의 JSON에 실제로 들어 있는 값.
  인용할 때 반드시 파일명과 측정 조건(머신·데이터 규모·평균 차수)을 함께 적는다.
- **사실(코드)** — 소스를 읽어 확인할 수 있는 구조적 사실(SPI 호출 횟수, 생성되는 SQL의 형태 등).
  `engine/src/...:123` 형식으로 라인을 붙인다.
- **추정** — 측정된 사실에서 산술적으로 유도했지만 그 자체로는 측정되지 않은 값.
  본문에 **"추정"**이라고 명시하고, 어떤 측정치에서 어떻게 유도했는지 적는다.
- **미확인** — 확인하지 못한 것. 그렇게 적는다. 추측을 사실처럼 쓰지 않는다.

## 4. 규칙 — 금지(Forbidden) / 필수(Required)

**금지**

- ❌ 측정되지 않은 개선 효과를 수치로 단정하는 것. (`"3배 빨라진다"` 대신 `"측정된 X에서 유도한 추정으로 3배"`)
- ❌ `docs/benchmark.md` 의 수치를 다른 데이터 규모·다른 워크로드에 그대로 옮겨 적는 것.
  1/2/3홉 표는 **평균 차수 20, 균등 랜덤 그래프, 웜 캐시, 동시성 없음** 조건에서만 성립한다.
- ❌ 서로 다른 런(run)의 셀을 하나의 표에 섞는 것. 각 표는 출처 JSON을 명시한다.
- ❌ "인덱스를 추가하라", "캐시를 도입하라" 같은 일반론. 반드시 이 코드의 구체적 지점을 지목한다.
- ❌ 이 문서를 근거로 코드를 고치면서 벤치를 다시 돌리지 않는 것.
  [`06_regression_guard.md`](06_regression_guard.md) 의 재현 명령을 함께 돌린다.

**필수**

- ✅ 모든 지연 수치 옆에 **프로토콜 바닥값(protocol floor)**을 같이 읽는다.
  Neo4j의 1홉 0.74 ms는 같은 클라이언트의 빈 질의 0.79 ms보다 *느리지 않다*.
- ✅ 지연(latency)과 논리 페이지 접근(logical page access)을 함께 본다.
  지연은 캐시 상태에 따라 움직이지만 페이지 수는 저장 구조의 직접 함수다.
- ✅ 깊은 순회를 이야기할 때는 **그래프 모양**을 먼저 말한다.
  평균 차수 20의 랜덤 그래프는 5홉이면 전체가 덮이므로, 거기서의 "20홉"은 깊이에 대한 질문이 아니다.
- ✅ 개선 제안에는 반드시 **검증 방법**(질의 / `EXPLAIN` / 벤치 명령)을 붙인다.

## 5. 사실 — 한 줄 요약 (2026-08-17 기준)

- 이 엔진의 저장 구조(CSR형 인접 세그먼트)는 **한 홉에서 이미 충분히 빠르다**.
  남은 비용은 대부분 **Cypher 표면**(jsonb 프로젝션, 도착 노드 재조인, SPI 왕복)에 있다.
  3홉 기준 Cypher 33.86 ms 대 스토리지 경로 4.33 ms — 7.8배
  ([`bench/results/bench-50000-20260806T042833Z.json`](../../../bench/results/bench-50000-20260806T042833Z.json)).
- 깊은 순회의 큰 승리(691배)는 **알고리즘 교체**(트레일 열거 → 방문집합 BFS)에서 왔고,
  힙을 떠나는 것(컴파일 CSR)은 그 위에 **약 15배**를 더한다
  ([`docs/deep-traversal.md`](../../deep-traversal.md)).
- 쓰기 경로는 아직 **측정되지 않았다.** `docs/benchmark.md` 의 "bulk load 124,580 edges/s" 는
  `og_create_node`/`og_create_edge` 가 아니라 하네스가 `og_data.*` 에 직접 넣는 SQL 경로의 수치다
  ([`bench/harness.py:322-355`](../../../bench/harness.py)). 자세한 것은
  [`02_measured_baselines.md` §5](02_measured_baselines.md) 와 [`03_hot_paths.md` §3](03_hot_paths.md).

<!-- affects: backend, data, ops -->
<!-- requires-update: docs/01_architecture/09_performance/02_measured_baselines.md, docs/01_architecture/09_performance/07_improvements_performance.md -->
