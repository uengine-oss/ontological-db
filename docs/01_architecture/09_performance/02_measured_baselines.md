# 측정된 기준선 — 수치와 그 조건

> **이 문서가 답하는 질문**
> - 이 프로젝트에서 **실제로 측정된** 수치는 무엇인가?
> - 각 수치는 어떤 머신·데이터 규모·평균 차수·PostgreSQL 설정에서 나왔는가?
> - 어떤 수치가 아직 **측정되지 않았는가**?

---

## 0. 규칙 — 이 문서의 사용법

**필수**

- ✅ 어떤 수치를 인용할 때는 표 헤더의 **출처 JSON 파일명**과 **§1의 측정 조건**을 함께 인용한다.
- ✅ 지연 수치는 같은 표의 **프로토콜 바닥값** 행과 함께 읽는다.
- ✅ 각 표는 **하나의 런(run)** 에서 온 값만 담는다. 표 사이의 셀을 옮기지 않는다.

**금지**

- ❌ 이 문서에 없는 수치를 만들어 쓰는 것.
- ❌ 평균 차수 20의 랜덤 그래프에서 얻은 수치를 다른 모양의 그래프에 적용하는 것.
- ❌ 논리 페이지 접근 수를 지연과 같은 방식으로 측정된 값으로 다루는 것 — §6의 주의사항을 볼 것.

## 1. 사실 — 측정 조건

모든 수치는 **한 대의 머신, 한 프로세스, 동시성 없음, 웜 캐시** 조건이다.

| 항목 | 값 | 근거 |
|---|---|---|
| 호스트 | Apple silicon (arm64), macOS, Docker Desktop | [`docs/benchmark.md`](../../benchmark.md) "Environment" |
| PostgreSQL | `PostgreSQL 16.14 (Debian 16.14-1.pgdg12+1) on aarch64-unknown-linux-gnu` | 모든 결과 JSON의 `environment.postgres` |
| PostgreSQL 설정 | **튜닝 없음.** `start.sh` 는 `cargo pgrx start pg16` 만 호출하고 `postgresql.conf` 를 건드리지 않는다 | [`start.sh:36`](../../../start.sh), [`docker/Dockerfile.dev`](../../../docker/Dockerfile.dev) |
| 접속 | `localhost:28816` | 결과 JSON의 `environment.host` |
| 비교 대상 | Neo4j 5.26.28 Community (heap 2 GB / pagecache 1 GB), Apache AGE 1.5.0, TypeDB 3.12.1 (기본값), pgGraph 1.1.0 (소스 빌드) | [`docs/benchmark.md`](../../benchmark.md), [`docs/deep-traversal.md`](../../deep-traversal.md) |
| 클라이언트 | psql 18.4 / `neo4j` Python 6.2.0 / `typedb-driver` 3.12.1 | [`docs/benchmark.md`](../../benchmark.md) |
| 타이밍 방식 | 이미 열린 psql 세션 안에서 `\timing`, 시작 노드 5개, 중앙값 | [`bench/harness.py:135-181`](../../../bench/harness.py) |
| 정확성 게이트 | 모든 시스템의 **답이 같은지 먼저 확인**하고, 다르면 그 질의의 타이밍을 VOID 처리 | [`bench/harness.py:1058-1096`](../../../bench/harness.py) |

**미확인 / 측정되지 않음**
동시성, 콜드 캐시, 스큐(허브·파워로우) 그래프, LDBC SNB, 쓰기 워크로드, 장기 스트레스.
[`docs/benchmark.md`](../../benchmark.md) "What this benchmark does not show" 와
[`bench/README.md`](../../../bench/README.md) "Not yet implemented" 가 같은 목록을 든다.

## 2. 사실 — 데이터셋

| 별칭 | 노드 | 엣지 | 평균 차수 | 모양 | 쓰인 곳 |
|---|---|---|---|---|---|
| 5k | 5,000 | 37,588 | 8 | 균등 랜덤 | `bench-5000-*` |
| 50k | 50,000 | 974,936 | 20 | 균등 랜덤 | `bench-50000-*` (공개 표의 기준) |
| chain-250k | 250,000 | 249,999 | 1 | 사슬, 지름 250,000 | `bench-250000-20260817T051823Z` |
| grid-250k | 250,000 | 499,000 | 2 | 500×500 격자, 지름 998 | `bench-250000-20260817T052859Z` |
| dense | 50,000 | 999,784 | 20 | 균등 랜덤 | `bench/csr/results/deep-dense-*` |
| sparse | 200,000 | 799,988 | 4 | 균등 랜덤 | `bench/csr/results/deep-sparse-*` |
| chain-1M | 1,000,000 | 999,999 | 1 | 사슬 | `bench/csr/results/deep-chain-*` |
| grid-1M | 1,000,000 | 1,998,000 | 2 | 1000×1000 격자 | `bench/csr/results/deep-grid-*` |

## 3. 사실 — 공개 기준선: 1/2/3홉 + 프로퍼티 스캔

**50,000 노드 / 974,936 엣지 / 평균 차수 20**
출처: [`bench/results/bench-50000-20260806T042833Z.json`](../../../bench/results/bench-50000-20260806T042833Z.json)
(AGE explicit·TypeDB 열은 같은 데이터에 대한 별도 런 — [`docs/benchmark.md`](../../benchmark.md) "Reproducing" 참고)

| 질의 | Ontological (Cypher) | Ontological (스토리지) | Neo4j 5 | Apache AGE | AGE explicit | TypeDB 3 | 재귀 CTE |
|---|---|---|---|---|---|---|---|
| 1홉 | 2.104 ms | 0.309 ms | 0.74 ms | 2.15 ms | 2.15 ms | **0.46 ms** | 0.244 ms |
| 2홉 | 3.955 ms | 0.499 ms | **0.92 ms** | 799.50 ms | 7.99 ms | 1.48 ms | 0.359 ms |
| 3홉 | 33.856 ms | 4.325 ms | **2.99 ms** | 22,412 ms | 34.63 ms | 19.73 ms | 3.491 ms |
| 프로퍼티 스캔 | 0.619 ms | 0.237 ms | 0.68 ms | **0.27 ms** | 0.24 ms | 0.52 ms | 0.191 ms |
| *프로토콜 바닥값* | *0.187 ms* | *0.190 ms* | *0.79 ms* | *0.17 ms* | *0.18 ms* | *0.37 ms* | *0.177 ms* |

> 이 JSON 파일 자체의 `age` 3홉 값은 **15,383.778 ms** 이고, `docs/benchmark.md` 가 싣는 22,412 ms 는
> 같은 데이터에 대한 다른 런(`bench-50000-20260806T052220Z`, 저장소에 없음)의 값이다.
> AGE의 `*1..n` 은 시작 노드에 따라 변동이 극심하며(중앙값 22.4 s, p95 914 s — `docs/benchmark.md`),
> 이 열의 어떤 값도 안정된 수치로 인용해서는 안 된다.

**논리 페이지 접근** (같은 파일):

| 질의 | Apache AGE | Ontological (Cypher) | Ontological (스토리지) | 재귀 CTE |
|---|---|---|---|---|
| 1홉 | 1,707 | 1,742 | 389 | 8 |
| 2홉 | 48,523 | 3,340 | 1,335 | 420 |
| 3홉 | 48,523 | 32,004 | 6,510 | 8,898 |
| 프로퍼티 스캔 | 35 | 1,174 | 8 | 6 |

## 4. 사실 — 깊은 순회 기준선 (정규화된 질문)

질문: **"시작 노드를 제외하고 k홉 안에 있는 서로 다른 노드의 수"**
([`bench/harness.py:305-317`](../../../bench/harness.py) 의 `reach_hop`).
문당 60초 상한. 출처:
[`bench/results/bench-50000-20260817T033001Z.json`](../../../bench/results/bench-50000-20260817T033001Z.json)
(50,000 노드 / 974,936 엣지 / 평균 차수 20).

| 깊이 | Ontological (Cypher) | Ontological (`og_reach`) | 재귀 CTE | Apache AGE | pgGraph 1.1.0 | Neo4j 5 |
|---|---|---|---|---|---|---|
| 1 | 1.519 | 0.107 | **0.083** | 94.002 | 1.163 | 0.960 |
| 2 | 2.636 | 0.183 | **0.192** | 761.429 | 9.996 | 2.944 |
| 3 | 29.064 | **1.058** | 2.174 | 13,696.722 | 237.870 | 4.659 |
| 4 | 193.355 | **18.956** | 27.532 | *>60 s* | 2,123.560 | 63.268 |
| 5 | 251.533 | **54.448** | 170.374 | — | 2,533.336 | 131.334 |
| 6 | 267.751 | **67.101** | 374.389 | — | 2,457.498 | 168.818 |
| 8 | 270.310 | **70.507** | 788.017 | — | 2,540.889 | 151.672 |
| 프로퍼티 스캔 | 0.371 | 0.069 | **0.041** | — | 0.065 | 0.664 |

같은 런의 **논리 페이지 접근** (PostgreSQL 상주 시스템만):

| 깊이 | Ontological (Cypher) | Ontological (`og_reach`) | 재귀 CTE | pgGraph |
|---|---|---|---|---|
| 1 | 2,070 | 448 | 30 | 4,198 |
| 3 | 32,173 | 3,300 | 8,874 | 202,534 |
| 4 | 195,202 | 16,170 | 165,750 | 1,631,086 |
| 6 | 222,561 | 41,910 | 2,524,332 | 1,783,210 |
| 8 | 222,561 | 41,910 | 4,984,092 | 1,783,210 |

`age_explicit` 행은 이 런에서 **5홉을 물었을 때 백엔드가 커널에 의해 종료(signal 9)** 되어 전부 결측이다
(JSON의 `not_attempted: "… crashed the server at reach5hop"`).

## 5. 사실 — 큰 지름 그래프

**chain-250k** (250,000 노드 / 249,999 엣지 / 차수 1) — [`bench-250000-20260817T051823Z.json`](../../../bench/results/bench-250000-20260817T051823Z.json)

| 깊이 | Ontological (Cypher) | Ontological (스토리지) | 재귀 CTE | Apache AGE | pgGraph | Neo4j 5 |
|---|---|---|---|---|---|---|
| 10 | 5.433 | 0.197 | **0.096** | 216.149 | 1.651 | 7.552 |
| 100 | 5.988 | 1.085 | **0.172** | 1,083.859 | 44.556 | 7.298 |
| 1,000 | 12.826 | 9.587 | **1.004** | *>60 s* | 4,163.659 | 13.006 |
| 10,000 | 122.295 | 91.010 | **7.855** | — | *2 GB 한도 초과* | 10.860 |

**grid-250k** (250,000 노드 / 499,000 엣지 / 차수 2) — [`bench-250000-20260817T052859Z.json`](../../../bench/results/bench-250000-20260817T052859Z.json)

| 깊이 | Ontological (Cypher) | Ontological (스토리지) | 재귀 CTE | Apache AGE | pgGraph | Neo4j 5 |
|---|---|---|---|---|---|---|
| 10 | 6.364 | 0.371 | **0.140** | 20,068.343 | 5.533 | 1.329 |
| 20 | 10.316 | 0.672 | **0.307** | *>60 s* | 28.986 | 8.621 |
| 50 | 11.625 | 1.758 | **1.307** | — | 382.005 | 8.447 |
| 100 | 27.556 | 5.582 | 4.978 | — | 2,975.771 | **4.331** |
| 500 | 649.168 | 146.338 | 145.150 | — | *2 GB 한도 초과* | **61.352** |

**결론(사실):** 프론티어가 그래프 전체를 덮는 모양에서는 우리 힙 BFS가 앞서고,
프론티어가 얇고 깊이가 큰 모양에서는 Neo4j와 **plain 재귀 CTE** 가 우리보다 앞선다.
`docs/deep-traversal.md` 의 "A correction we owe the earlier section" 이 같은 결론을 서술한다.

## 6. 사실 — `og_vlp` / `og_reach_sql` / `og_reach` / `og_csr_reach` 4종 비교

출처: [`bench/csr/results/`](../../../bench/csr/results/). 전체 표는
[`docs/deep-traversal.md`](../../deep-traversal.md) 에 있으므로 여기서는 **문서에 없는 값**만 싣는다.

**CSR 컴파일 비용** (백엔드마다 1회, 질의는 이 비용을 내지 않음):

| 픽스처 | 노드 | 엣지 | 크기 | 컴파일 시간 | 출처 |
|---|---|---|---|---|---|
| dense | 50,000 | 999,784 | 8,798,280 B (8.39 MiB) | 119.2 ms | `deep-dense-20260817T021522Z.json` |
| dense (재측정) | 50,000 | 999,784 | 8,798,280 B | 123.4 ms | `deep-dense-deep-20260817T021624Z.json` |
| sparse | 200,000 | 799,988 | 9,599,912 B (9.16 MiB) | 229.4 ms | `deep-sparse-20260817T021627Z.json` |
| **chain-1M** | 1,000,000 | 999,999 | 24,000,000 B (22.9 MiB) | **935.2 ms** | `deep-chain-20260817T053710Z.json` |
| **grid-1M** | 1,000,000 | 1,998,000 | 31,984,008 B (30.5 MiB) | **968.3 ms** | `deep-grid-20260817T054540Z.json` |

> `docs/deep-traversal.md` 는 dense/sparse의 컴파일 비용만 싣는다.
> **100만 노드 규모에서는 백엔드당 약 1초 / 23~31 MB** 라는 것이 위 두 줄이 새로 말해 주는 사실이다.
> 연결당 이 비용을 내는 배치라면 `og_csr_build` 는 사실상 쓸 수 없다.

**chain-1M / grid-1M 4종 비교 (ms, 중앙값)** — 두 파일 모두 모든 변형의 답이 일치(`agrees: true`):

| chain-1M 깊이 | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
|---|---|---|---|---|
| 10 | 0.245 | 0.133 | 0.177 | **0.061** |
| 100 | 0.440 | 0.258 | 0.977 | **0.079** |
| 1,000 | 8.963 | 1.404 | 8.817 | **0.131** |
| 10,000 | 707.821 | 12.895 | 95.424 | **0.728** |
| 100,000 | 65,820.290 | 154.490 | 1,015.933 | **9.852** |

| grid-1M 깊이 | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
|---|---|---|---|---|
| 10 | 4.285 | 0.283 | 0.278 | **0.075** |
| 20 | 2,784.755 | 0.488 | 0.565 | **0.083** |
| 50 | *>120 s* | 8.097 | 1.802 | **0.168** |
| 100 | *>120 s* | 22.307 | 5.999 | **0.434** |
| 500 | *>120 s* | 186.768 | 108.307 | **9.396** |
| 1,000 | *>120 s* | 235.072 | 125.020 | **10.606** |

## 7. 사실 — 로드 처리량 (주의: 이것은 쓰기 API의 수치가 아니다)

결과 JSON의 `load_edges_per_sec` 는 하네스가 `og_data.og_node` / `og_edge` / `og_adj` 에
**직접 `INSERT … SELECT` 하는 SQL 경로**를 잰 값이다
([`bench/harness.py:322-355`](../../../bench/harness.py)).
`og_create_node()` / `og_create_edge()` / Cypher `CREATE` 는 이 수치에 **포함되지 않는다.**

| 런 | 데이터 | Ontological | AGE | 재귀 CTE | pgGraph | Neo4j |
|---|---|---|---|---|---|---|
| `bench-50000-20260806T042833Z` | 50k | 124,580 e/s | 406,977 | 517,159 | — | 58,947 |
| `bench-50000-20260817T033001Z` | 50k | 161,852 e/s | 434,119 | 505,306 | 174,552 | 71,313 |
| `bench-250000-20260817T051823Z` | chain-250k | 81,367 e/s | 214,133 | 344,691 | 100,578 | 26,761 |
| `bench-5000-20260817T030411Z` | 5k | 56,661 e/s | 72,893 | 91,131 | 62,295 | 32,824 |

같은 데이터에 대해 두 런이 124,580 대 161,852로 **30% 차이**가 난다.
이 항목은 안정된 지표가 아니며, 회귀 게이트가 감시하지도 않는다([`06_regression_guard.md`](06_regression_guard.md)).

## 8. 사실 — 페이지 수 측정의 주의사항 (중요)

`buffers` 값은 지연과 **같은 방식으로 측정되지 않는다.**

- 지연은 이미 열린 psql 세션 안에서 워밍업 후 중앙값을 잰다
  ([`harness.py:135-181`](../../../bench/harness.py)).
- 페이지 수는 `buffers_read()` 가 **새 psql 프로세스를 띄워** `EXPLAIN (ANALYZE, BUFFERS)` 를 한 번 돌린 값이다
  ([`harness.py:183-206`](../../../bench/harness.py) → [`harness.py:54-66`](../../../bench/harness.py) 의 `psql()`).
  즉 **콜드 백엔드의 첫 호출**이다.

이 차이는 `ontological` 행에만 비대칭적으로 불리하게 작용한다.
Cypher 표면은 첫 호출에서 Rust 쪽 컴파일 경로를 지나고, 그 경로가 카탈로그를 SPI로 여러 번 읽기 때문이다
([`03_hot_paths.md` §2](03_hot_paths.md)).

측정으로 뒷받침되는 증거: `ontological` 의 `prop_scan` 페이지 수는 **데이터 크기와 무관하게 거의 일정하다.**

| 데이터 규모 | `ontological` prop_scan 페이지 | `ontological_raw` prop_scan 페이지 | 출처 |
|---|---|---|---|
| 5,000 노드 | 1,170 | 6 | `bench-5000-20260817T030411Z.json` |
| 50,000 노드 | 1,173 | 6 | `bench-50000-20260817T033001Z.json` |
| 50,000 노드 (다른 런) | 1,174 | 8 | `bench-50000-20260806T042833Z.json` |
| 250,000 노드 (chain) | 1,177 | 8 | `bench-250000-20260817T051823Z.json` |
| 250,000 노드 (grid) | 1,177 | 8 | `bench-250000-20260817T052859Z.json` |

노드 수가 50배로 늘어도 1,170 → 1,177 로만 움직인다.
**따라서 이 1,170여 페이지는 데이터 스캔이 아니다.** 무엇이 소비하는지는 **미확인**이며,
후보는 컴파일 시점 카탈로그 SPI 조회(`types::graph_id`, `resolve_label_set`, `views::view_exists` /
`view_properties` / `concrete_tables`, `labeling::og_subtypes`), `og_cypher` 의 감사 로그 INSERT
([`cypher/mod.rs:122-135`](../../../engine/src/cypher/mod.rs)), pgrx 확장 첫 로드다.
확인 방법은 [`PERF-14`](07_improvements_performance.md) 에 적었다.

## 9. 사실 — 재현 불가·해석 주의가 필요한 관측

**`bench-5000-20260817T030411Z.json` 에서 `ontological` 과 `ontological_raw` 의 페이지 수가 사실상 동일하다.**

| 질의 | `ontological` | `ontological_raw` |
|---|---|---|
| reach1hop | 1,754 페이지 / 0.482 ms | 1,754 페이지 / 0.607 ms |
| reach2hop | 2,046 / 0.756 ms | 2,047 / 0.742 ms |
| reach3hop | 4,376 / 2.672 ms | 4,376 / 2.719 ms |
| reach4hop | 22,260 / 18.188 ms | 22,260 / 18.575 ms |

같은 두 행이 50,000 노드 런에서는 2,070 대 448 (4.6배), 1.519 ms 대 0.107 ms (14배)로 갈린다.
`ontological_raw` 의 존재 이유는 "Cypher 표면을 뺀 스토리지 경로를 재는 것"인데
([`bench/README.md`](../../../bench/README.md) "Two Ontological rows"),
이 런에서는 두 행이 서로 다른 것을 재고 있다는 전제가 성립하지 않는 것으로 보인다.
**원인 미확인.** 회귀 게이트의 공백으로 [`06_regression_guard.md` §5](06_regression_guard.md) 에 기록한다.

## 10. 미확인 — 아직 측정되지 않은 것

| 항목 | 왜 필요한가 |
|---|---|
| `og_create_node` / `og_create_edge` 의 실제 처리량 | 쓰기 경로 개선의 기준선이 아예 없다 |
| Cypher `CREATE` / `MERGE` / `SET` 의 행당 비용 | 배치 쓰기 애플리케이션의 실제 경험 |
| 동시 쓰기 확장성 (`og_id_alloc` 경합) | [`PERF-08`](07_improvements_performance.md) 의 효과를 판단할 수 없다 |
| 인접 세그먼트의 쓰기 증폭 (WAL 바이트/엣지) | [`PERF-09`](07_improvements_performance.md) |
| 벡터 검색 지연 / HNSW recall | `og_vector_search_exact` 라는 기준 구현은 있으나 벤치가 없다 |
| Bolt 게이트웨이의 행당 처리량 | [`PERF-16`](07_improvements_performance.md), [`PERF-17`](07_improvements_performance.md) |
| Studio(`portal/`) 의 대용량 결과 처리 | [`PERF-18`](07_improvements_performance.md) |
| 콜드 캐시 / 동시성 / 스큐 그래프 | `docs/benchmark.md` 가 명시적으로 제외한 것 |

<!-- affects: backend, ops -->
<!-- requires-update: docs/01_architecture/09_performance/06_regression_guard.md -->
