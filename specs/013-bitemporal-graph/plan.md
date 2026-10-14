# Implementation Plan: 이중 시간 그래프

**Branch**: `013-bitemporal-graph` | **Date**: 2026-09-30 | **Spec**: [spec.md](spec.md)

## Summary

SQL:2011 시스템 버전 테이블의 모양을 타입별 저장 테이블에 씌운다. **현재 테이블**은 지금
믿는 것을, 옆의 **이력 테이블** `og_data.h_<type_id>` 는 대체된 믿음과 여전히 믿는 과거 유효
구간을 갖는다. 캡처는 트리거, 읽기는 생성 뷰가 세션 설정을 읽어서 한다. 질의 텍스트도,
컴파일된 SQL 도 시점을 모른다.

**핵심 설계 결정** — 근거와 대가는 [ADR-027](../../docs/99_decisions/ADR-027-bitemporal-current-plus-history.md)

1. **현재 + 이력, 두 테이블.** 모든 버전을 한 테이블에 두면 현재만 묻는 질의(대부분)가 과거
   버전을 걸러야 하고, 인덱스·통계가 역사에 희석된다. 현재 테이블은 이전과 같은 크기로 남는다.
2. **시점은 설정이다.** `og.valid_at` / `og.system_at` 을 `og_valid_at()` / `og_system_at()`
   (STABLE SQL 함수)이 읽고, 생성 뷰의 필터가 그 함수를 부른다. 컴파일러는 시점을 SQL 에
   넣지 않으므로 계획 캐시가 시점마다 갈라지지 않는다. 이력 행 중 **닫힌 행**(`__sys_to`
   유한 — 대체된 믿음)의 분기는 시점이 없을 때 `og_asof_active()` 가 거짓인 한 번의 필터로
   잘린다. **열린 행**(`__sys_to = ∞` — 여전히 믿는 유효 구간 조각)은 현재의 일부라 늘 읽는다.
3. **캡처는 트리거다.** `og__temporal()`(BEFORE 트리거, SECURITY DEFINER). 엔진이 아니라
   테이블이 버전을 만들므로 원시 SQL 도 똑같이 기록된다. 쓰기 역할은 이력 테이블에 DML 이
   없다.
4. **못 하면 거부한다.** 레지스트리와 인접 세그먼트는 현재만 안다. 그걸 읽어야만 하는 질의
   모양에는 컴파일러가 `og_require_current(what)` 술어를 넣고, 시점이 설정되어 있으면 실행
   시점에 `OG_TEMPORAL_UNSUPPORTED` 가 난다. 이 술어도 시점과 무관한 SQL 이다.

## Architecture

| 구성 | 위치 | 스펙 |
|------|------|------|
| `og_enable_temporal(graph, type)` | `engine/src/temporal/mod.rs` | FR-001 |
| `__valid tstzrange`, `__sys_from timestamptz` 컬럼 + `h_<tid>`(+ `__sys_to`, `__op`) | `temporal::enable_table` | FR-001 |
| `og__temporal()` 트리거 `og_b_temporal` | `engine/sql/access.sql` | FR-002..FR-005 |
| `og__visible(doc)` — 보이는 변화 판정 (시스템 컬럼·그림자 컬럼 제외) | `access.sql` | FR-005 |
| `og_valid_at()`, `og_system_at()`, `og_asof_active()` | `access.sql` | FR-006 |
| 생성 뷰: 현재 분기 `__valid @> og_valid_at() AND __sys_from <= og_system_at()` ∪ 열린 이력 분기 `__sys_to = 'infinity' AND __valid @> og_valid_at() AND …` ∪ 닫힌 이력 분기 `og_asof_active() AND __sys_to <> 'infinity' AND … AND __sys_to > og_system_at()` | `engine/src/cypher/views.rs`, `temporal::visibility_predicates` | FR-007 |
| 열린 이력 행의 부분 GiST 인덱스 `ON h_<tid> USING gist (__valid) WHERE __sys_to = 'infinity'` | `temporal::enable_table` | FR-007 |
| 단일 홉: temporal 관계 타입은 `temporal::edge_relation`(간선 테이블 현재 + 열린 이력 + 닫힌 이력, 같은 세 분기) | `engine/src/cypher/compile.rs` `join_rel` | FR-008 |
| Bolt `tx_metadata` → `set_config(…, true)` | `bolt/src/session.rs` | FR-009 |
| `og_require_current(what)` | `access.sql`, `compile.rs`, `typeql/mod.rs` | FR-010 |
| `temporal::sync_history(sub)` | `og_add_property`, 컬럼 승격, 012 파이프라인에서 호출 | FR-011 |
| `og_temporal_types(graph)`, `og_meta(id)` | `temporal/mod.rs`, `access.sql` | FR-012 |

### 쓰기의 의미 (X = `og.valid_from` 또는 `now()`)

| 연산 | 이력에 쓰는 것 | 현재 행 |
|---|---|---|
| INSERT | — | `__valid = [X, og.valid_to 또는 ∞)`, `__sys_from = now()` |
| UPDATE (보이는 변화 없음) | — | `__valid`, `__sys_from` 유지 |
| UPDATE | ① 옛 행, `__sys_to = now()`, op `u` ② X > lower 이면 옛 값의 `[lower, X)`, `__sys = [now, ∞)`, op `v` | `__valid = [max(X, lower), og.valid_to 또는 옛 upper)` |
| DELETE | ①, ② (op `d`, `v`) | 없음 |
| X ≥ upper (X > lower 일 때) | 거부 `OG_TEMPORAL_RANGE` | — |

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I. 포크가 아닌 확장 | ✅ | `tstzrange`, 트리거, 커스텀 GUC(`og.*`) — 전부 표준 PG |
| II. Cypher 1급 | ✅ | Cypher 무변경. 시점은 질의 밖의 설정 |
| III. 저장구조 | ⚠️ 부분 | temporal 관계 타입의 단일 홉은 인접 세그먼트를 쓰지 않는다 — 아래 Complexity Tracking |
| IV. 온톨로지 우선 타이핑 | ✅ | 활성화가 계층을 따라 상속된다 |
| V. 벡터 | ✅ | 해당 없음 |
| VI. 코어는 하나 | ⚠️ | TypeQL 은 시점 질의를 지원하지 않는다(거부) |
| VII. 확장성 | ✅ | 해당 없음 |
| **VIII. 에이전트 네이티브** | ✅ | 본 스펙이 원칙 VIII 의 마지막 항목("유효 시각과 기록 시각") 구현. 답할 수 없으면 오류 |
| **IX. ACID** | ✅ | 캡처가 같은 트랜잭션의 트리거. 이력도 일반 힙 테이블(`pg_dump` 포함) |
| X. 벤치마크로 증명 | ⏳ | 쓰기 오버헤드(008 SC-011) 미측정 |

## Phasing

| Phase | 내용 | 상태 |
|-------|------|------|
| P0 | 캡처 트리거, 이력 테이블, 활성화 상속 | 완료 |
| P1 | 설정 기반 시점, 생성 뷰 합집합, 계획 캐시 불변 | 완료 |
| P2 | temporal 관계의 단일 홉, 거부 가드 | 완료 |
| P3 | Bolt 메타데이터 | 완료 |
| P4 | 가변 길이·TypeQL·무레이블 스캔의 시점 질의, 보존 정책, 소급 편집 | 미착수 |

## Complexity Tracking

**미래 유효 시각의 변경 — 열린 이력 행은 현재의 일부다 (수정됨, 09 가 단언).**
`og.valid_from` 을 **미래** 시각 X 로 두고 갱신하면, 현재 행의 유효 구간은 `[X, …)` 가 되고
지금 유효한 옛 값은 이력의 `v` 행(`__sys_to = ∞`)에만 남는다. 처음 설계는 이력 분기 전체를
`og_asof_active()` 로 잘랐기 때문에, 시점을 설정하지 않은 질의에서 그 엔티티가 X 까지
사라졌다. 지금은 이력을 **열린 행**(`__sys_to = ∞`, 여전히 믿는 유효 구간 조각)과 **닫힌
행**(대체된 믿음)으로 나누어, 열린 행은 시점 설정과 무관하게 `__valid` 가 그 시각을 포함하면
보이고 닫힌 행만 `og_asof_active()` 로 자른다. 미래 시각의 삭제도 같다. 대가: 시점을 설정하지
않은 기본 질의도 temporal 타입마다 열린 이력 분기를 한 번 탐색한다. 부분 GiST 인덱스
`ON h_<tid> USING gist (__valid) WHERE __sys_to = 'infinity'` 가 이를 인덱스 탐색으로 만들며,
열린 행은 보통 소수다(유효 구간을 쪼갠 변경의 앞 조각만 남는다).

**temporal 관계 타입의 홉은 인접 세그먼트를 쓰지 않는다.** 세그먼트는 현재만 알기 때문에,
관계 타입이 temporal 이면 단일 홉이 **시점 설정 여부와 무관하게** 간선 테이블(현재 + 열린
이력 + 닫힌 이력 합집합, `src`/`dst` 인덱스)을 읽는다. 원칙 III(홉은 순차 메모리 접근)에서의 이탈이며,
해당 관계 타입에만 해당한다. 시점이 없을 때 세그먼트로 되돌리는 분기는 컴파일된 SQL 이
시점에 무관해야 한다는 결정 2 와 충돌해서 넣지 않았다. 비용은 측정하지 않았다.

**현재 분기에도 필터가 붙는다.** 시점이 없어도 temporal 타입의 현재 테이블 스캔에는
`__valid @> now() AND __sys_from <= now()` 가 평가되고, 열린 이력 분기는 부분 인덱스로
탐색된다. 닫힌 이력 분기만 무비용으로 잘린다.

**가변 길이 경로, 레이블 없는 스캔, TypeQL 은 거부한다.** 구현하지 않은 것을 틀린 답으로
위장하지 않는다는 것이 008 FR-021 의 원칙이다. 가변 길이 경로의 시점 판은 `og_reach`/`og_vlp`
가 세그먼트 대신 시점 필터된 간선 관계를 받아야 하며, ADR-012/013 의 비용 모델을 다시 봐야
한다.

**`og_enable_temporal` 은 스탬프도 켠다**(`og_catalog.stamped` 에 함께 기록). 이력 행에 "누가"
가 없으면 기록 시각이 반쪽이기 때문이다. 스펙 014 의 `og_enable_stamping` 과 독립적으로 끌
수 없다.

**레거시 008 경로는 일부만 테스트가 있다.** `og_as_of` 의 삭제 후 NULL 과 같은 트랜잭션 안의
`hist_id` 순서는 `10_evolution_temporal_coverage.sql` 이 단언한다. 하위 타입 캡처와
`og_capture_history` 의 SECURITY DEFINER 는 코드 변경이며 전용 회귀 테스트가 없다.
