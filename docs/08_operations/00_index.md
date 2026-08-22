# 08_operations — 운영

> **이 문서가 답하는 질문**
> - 이 카테고리는 누구를 위한 문서인가?
> - "띄우고, 보고, 고친다"는 각 단계에서 어느 문서를 읽어야 하는가?
> - 이 저장소에서 운영자가 신뢰해도 되는 명령은 어디까지인가?

---

## 이 카테고리의 독자

`08_operations/`는 **이 시스템을 실제로 실행하는 사람**을 위한 층이다.
설계 의도(`01_architecture/`)나 질의 언어 사용법(`02_api/`)이 아니라,
다음 세 가지 동사에만 답한다.

| 동사 | 문서 |
|---|---|
| **띄운다** | [01_install](01_install.md) · [02_startup_and_teardown](02_startup_and_teardown.md) · [03_configuration](03_configuration.md) |
| **검증한다** | [04_testing_and_ci](04_testing_and_ci.md) · [05_benchmarking](05_benchmarking.md) |
| **관측한다** | [06_monitoring](06_monitoring.md) |
| **고친다 / 지킨다** | [07_maintenance](07_maintenance.md) · [08_backup_and_restore](08_backup_and_restore.md) · [09_troubleshooting](09_troubleshooting.md) |
| **개선한다** | [10_improvements_ops](10_improvements_ops.md) |

---

## 문서 목록

| 문서 | 답하는 것 |
|---|---|
| [01_install.md](01_install.md) | Docker 경로와 로컬 빌드 경로, 지원 PostgreSQL 버전, pgvector 의존성, `cargo pgrx` 절차 |
| [02_startup_and_teardown.md](02_startup_and_teardown.md) | `start.sh`가 실제로 하는 일의 단계별 해부, 수동 등가 명령, 환경변수 전체 표, 정지 절차 |
| [03_configuration.md](03_configuration.md) | `og_catalog.setting` / `og_set_setting`, PostgreSQL 파라미터, Studio·Bolt 환경변수 |
| [04_testing_and_ci.md](04_testing_and_ci.md) | 회귀 스위트 3종의 실행법과 각각이 실제로 검증하는 범위, CI 부재 사실 |
| [05_benchmarking.md](05_benchmarking.md) | `bench/harness.py` 실행법, 정확성 게이트, 결과 JSON 스키마, 리포트 페이지 연결 |
| [06_monitoring.md](06_monitoring.md) | `og_graph_stats` / `og_degree_distribution` / `og_csr_stats` / `og_embedding_stats` / `og_data.og_audit` / `pg_stat_*` — 복사 가능한 SQL |
| [07_maintenance.md](07_maintenance.md) | `og_reorganize` / `og_check_integrity` / `og_relabel`, VACUUM·ANALYZE 전략, CSR 재빌드 시점 |
| [08_backup_and_restore.md](08_backup_and_restore.md) | `pg_dump`가 그래프를 온전히 담는 근거(`pg_extension_config_dump`), 복원 시 버전 주의점 |
| [09_troubleshooting.md](09_troubleshooting.md) | 증상 → 원인 → 조치. 실제 코드에 존재하는 오류 문자열만 사용 |
| [10_improvements_ops.md](10_improvements_ops.md) | 운영 관점 개선 포인트 `OPS-01`..`OPS-14` |

---

## 이 카테고리 전체에 걸친 규칙

### 필수 (Required)

- **명령은 저장소 파일에서 확인된 것만 쓴다.** 각 명령에는 근거 파일:라인이 붙어 있다.
- 포트·데이터베이스 이름·환경변수는 **`start.sh`의 기본값**을 기준으로 쓴다
  (`start.sh:6-10`). 다른 값을 쓰는 경우 그 사실을 명시한다.
- 확장 함수를 호출할 때는 그래프 이름을 항상 명시한다. 데모 그래프의 이름은 `default`이다
  (`examples/demo.sql` 및 `portal/server/index.js:169`의 기본값).

### 금지 (Forbidden)

- **문서에 없는 CLI 옵션을 추측해서 쓰지 않는다.** 예를 들어 `bench/harness.py`에는
  `--systems`, `--scale`, `--degree`, `--runs`, `--hops`, `--shape`, `--workload`,
  `--query-timeout`, `--compare-baseline` **만** 존재한다 (`bench/harness.py:1247-1266`).
- **확장 업그레이드 명령(`ALTER EXTENSION ontological UPDATE`)을 안내하지 않는다.**
  업그레이드 스크립트가 저장소에 존재하지 않는다 — [08_backup_and_restore.md](08_backup_and_restore.md)
  및 [10_improvements_ops.md](10_improvements_ops.md) `OPS-02` 참조.
- **프로덕션 배포 절차를 지어내지 않는다.** 저장소에는 `docker/Dockerfile.dev` 하나만 있고
  릴리스 이미지 정의가 없다 (`OPS-05`).

---

## 참조하는 원문 문서 (영문, 삭제·수정 금지)

- [`docs/api.md`](../api.md) — SQL 함수 레퍼런스. 운영 함수는 "Operations" 절.
- [`docs/benchmark.md`](../benchmark.md) — 벤치마크 결과 원문 및 환경.
- [`docs/deep-traversal.md`](../deep-traversal.md) — `og_reach` / `og_csr_*` 설계와 비용.
- [`bench/README.md`](../../bench/README.md), [`bench/csr/README.md`](../../bench/csr/README.md)
- [`bolt/README.md`](../../bolt/README.md) — Bolt 게이트웨이 환경변수와 지원 범위.
- [`tests/neo4j-movies/README.md`](../../tests/neo4j-movies/README.md)

<!-- affects: ops -->
