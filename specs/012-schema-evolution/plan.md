# Implementation Plan: 스키마 진화

**Branch**: `012-schema-evolution` | **Date**: 2026-09-30 | **Spec**: [spec.md](spec.md)

## Summary

스키마 변경을 **네 번의 호출**과 **카탈로그의 한 행**으로 만든다. 느린 단계(마이그레이션)만
배치로 돌고, 나머지는 짧은 카탈로그 잠금으로 끝난다.

**핵심 설계 결정**

1. **이름과 계층은 카탈로그 항목이다.** 속성 이름은 `og_catalog.property(name → column_name)`
   의 한 행이고, 타입 이름은 `og_catalog.type(name → storage_table)` 의 한 행이며, 상속은
   구간 라벨이다([ADR-007](../../docs/99_decisions/ADR-007-nested-set-interval-labels.md)).
   저장소는 이름이 아니라 id 로 불린다(`n_<type_id>`, `p_<원래 이름>`). 그래서
   `rename_property`, `rename_type`, `add_parent`, `remove_parent`, `drop_property` 는
   **카탈로그 편집이고, 비용이 데이터 크기에 비례하지 않는다.** 행을 다시 쓰는 것은 표현이
   바뀌는 변경 — `alter_property_type` 의 캐스트와 `set_required` 의 기본값 채우기 — 뿐이며,
   그것만 배치로 돈다. 결정의 근거와 대가는
   [ADR-028](../../docs/99_decisions/ADR-028-schema-change-at-catalog-level.md).
2. **변경은 1급 객체다.** `og_catalog.schema_change` 한 행이 단계, 시작·종료 버전, 계획 시
   영향 보고, 마이그레이션 커서, 검증 결과를 갖는다. "누가 언제 무엇을 어떤 순서로
   바꿨는가"가 스크립트 이력이 아니라 질의 결과다.
3. **새 쓰기가 먼저 새 규칙을 따른다.** 확장 직후부터 새 쓰기는 새 규칙을 만족해야 한다
   (`NOT VALID` 제약, 그림자 컬럼으로의 캐스트 이중 쓰기). 그래야 마이그레이션이 끝났을 때
   "그 사이에 들어온 위반"이 남지 않는다. 표현 변경의 이중 쓰기는 트리거라 원시 SQL 쓰기에도
   성립한다. 단, 이 성질은 아래 Complexity Tracking 에 적은 두 종류(`add_unique`,
   `alter_role`)에는 성립하지 않는다.
4. **스냅샷은 버전을 올릴 때마다.** `bump_schema_version` 이 `og__schema_snapshot(gid)`
   를 함께 저장한다. 스냅샷은 버전 올림의 부수 효과이므로 따로 잊을 수 없다.

## Architecture

| 함수 | 스펙 |
|------|------|
| `og_schema_plan(graph, change jsonb) → int` | FR-001, FR-003 — 기록 + 영향 분석, 무변경 |
| `og_schema_expand(id) → jsonb` | FR-004 |
| `og_schema_migrate(id, batch = 10000) → jsonb` | FR-005 — `{moved, done, migrated}`, 테이블별 id 커서 |
| `og_schema_validate(id) → jsonb` | FR-006 — `{phase, violations, samples}` |
| `og_schema_contract(id) → jsonb` | FR-007 |
| `og_schema_abort(id) → jsonb` | FR-002 |
| `og_schema_apply(graph, change) → jsonb` | FR-008 — 전 단계 한 트랜잭션 |
| `og_schema_changes(graph)` | FR-001 — 최신순 목록 |
| `og_schema_snapshot(graph, version = NULL, at = NULL) → jsonb` | FR-009 |

코드: `engine/src/catalog/evolution.rs`(단계와 종류), `engine/sql/bootstrap.sql`
(`og_catalog.schema_change`, `schema_version.snapshot`), `engine/sql/access.sql`
(`og__castable`, `og__shadow_write` — 이중 쓰기 트리거 함수, `og__schema_snapshot`,
`og_schema_snapshot`), `engine/src/cypher/views.rs`
(두 이름이 한 컬럼을 가리킬 때 뷰가 컬럼을 한 번만 고르는 `view_columns`).

### 종류별 물리 효과

| kind | expand | migrate | validate | contract | 행 재작성 |
|---|---|---|---|---|---|
| `add_property` | `og_add_property`(선택) — `ADD COLUMN`, `__ext` 값 이동 | — | — | — | 없음(`__ext` 에 값이 있던 행만 갱신) |
| `drop_property` | — | — | — | `DROP COLUMN`(PG 에서 메타데이터 연산), 카탈로그 삭제 | 없음 |
| `rename_property` | 같은 컬럼에 두 번째 이름 | — | — | 옛 이름 삭제, `write_grant`·`compat_index` 갱신 | 없음 |
| `alter_property_type` | 그림자 컬럼 `p_x__s<id>` 추가, 이중 쓰기 트리거 `og_c_shadow_<id>` 부착 | 텍스트 경유 캐스트, 배치, 재개 가능 | 그림자가 비어 있는 행 = 위반 | 옛 컬럼 삭제, 그림자 개명, 인덱스 재생성, 컬럼 권한 재적용 | **있음** |
| `set_required` | `CHECK (col IS NOT NULL) NOT VALID` | 기본값 채우기(배치) | `VALIDATE CONSTRAINT`(SHARE UPDATE EXCLUSIVE) | `SET NOT NULL`(검증된 CHECK 덕분에 스캔 없음), CHECK 삭제 | 기본값이 있을 때 빈 행만 |
| `add_unique` | — | — | 계층 전체 중복 | 구체 테이블마다 유니크 인덱스 | 없음(인덱스 빌드) |
| `rename_type` | — | — | — | 카탈로그 이름, 별칭 뷰 이동, `compat_index` 갱신 | 없음 |
| `alter_role` | — | — | 기존 간선/role player 검사 | 역할 갱신 | 없음 |
| `add_parent` | — | — | — | `type_parent` 추가, 구간 재라벨, 상속 컬럼 추가, temporal/stamping/권한 상속 | 없음 (`xmin` 불변을 테스트가 단언) |
| `remove_parent` | — | — | — | `type_parent` 삭제, 재라벨 | 없음 |

대부분의 종류는 카탈로그(매핑) 수준이며 그 비용은 데이터 크기와 무관하다. 행을 다시 쓰는
것은 `alter_property_type` 와 기본값이 있는 `set_required` 뿐이다.
측정값은 [`docs/schema-evolution-benchmark.md`](../../docs/schema-evolution-benchmark.md) 에 있다.
요지: 이름·계층 변경(속성 이름, 타입 이름, 관계 타입 이름, 상위 타입 도입)은 1만~100만 노드에서
모두 2.5–5.3 ms 로 데이터 크기와 무관했고, 같은 변경을 Neo4j 5 는 100만 노드에서 2.2–39.5 s, 같은
저장소의 물리 마이그레이션은 2.6–124 s 에 했다. 타입 변경(×2.9)과 유니크 제약(×2.9)은 Neo4j 가 빠르다
— 단계 분리가 같은 데이터를 여러 번 읽기 때문이다.

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I. 포크가 아닌 확장 | ✅ | `ALTER TABLE`, `NOT VALID`/`VALIDATE CONSTRAINT`, 전환 중에만 붙는 이중 쓰기 트리거 — 전부 표준 PG |
| II. Cypher 1급 | ✅ | 변경은 SQL 함수. 질의 언어 무관 |
| III. 저장구조 | ✅ | 타입별 실컬럼 유지. 이름이 저장소와 분리되어 있다는 기존 성질을 활용 |
| IV. 온톨로지 우선 타이핑 | ✅ | 재부모화는 구간 재라벨 한 번. 재귀 없음 |
| V. 벡터 | ✅ | 해당 없음 |
| VI. 코어는 하나 | ⚠️ | TypeQL `redefine`/`undefine` 이 이 파이프라인을 거치지 않는다 — 두 개의 스키마 변경 경로 |
| VII. 확장성 | ✅ | 해당 없음(분산 없음) |
| **VIII. 에이전트 네이티브** | ✅ | 영향 보고와 위반 표본이 구조화된 jsonb. 스키마 버전·스냅샷 노출 |
| **IX. ACID** | ✅ | 각 단계가 PG 트랜잭션. `og_schema_apply` 는 전 단계가 한 트랜잭션이라 실패 시 아무것도 남지 않는다 |
| X. 벤치마크로 증명 | ✅ | `bench/schema_evolution.py`, 정확성 게이트 포함 — 결과와 지는 지점까지 `docs/schema-evolution-benchmark.md` |

## Phasing

| Phase | 내용 | 상태 |
|-------|------|------|
| P0 | 변경 레코드, 단계 상태 기계, 스냅샷 | 완료 |
| P1 | 카탈로그 수준 종류(rename ×2, drop, add, re-parent ×2) | 완료 |
| P2 | 표현 변경(`alter_property_type`) — 그림자 + 이중 쓰기 + 배치 | 완료 |
| P3 | 제약(`set_required`, `add_unique`, `alter_role`) | 완료(아래 한계) |
| P4 | 타입 분리/병합, TypeQL `redefine` 매핑 | 미착수 |

## Complexity Tracking

**이중 쓰기는 전환 중인 테이블에만 붙는 한시 트리거다.** `alter_property_type` 의 확장이
구체 테이블마다 `og_c_shadow_<change_id>` 를 `BEFORE INSERT OR UPDATE OF <col>` 로 붙이고
(`og__shadow_write`, `access.sql`), 축소와 중단이 뗀다. 그래서 Cypher·엔진 함수·원시 SQL 이
모두 그림자를 채우고, 새 타입이 담을 수 없는 값은 원시 SQL 이라도 그 자리에서 거부된다
(22P02). 비용은 전환 중인 테이블의 해당 컬럼 쓰기에만 붙는다. 마이그레이션의 배치 채우기는
그림자만 쓰므로 `UPDATE OF <col>` 에 걸리지 않아 트리거가 돌지 않는다. 한때 이 일을 하던
엔진 측 이중 쓰기(`storage::plan_props`)와 그 그림자 목록 캐시는 제거되었다.

**`add_unique` 와 `alter_role` 은 확장 단계에서 아무것도 강제하지 않는다.** 검증과 축소
사이에 새 중복이나 새 역할 위반을 쓸 수 있다. `add_unique` 는 그 경우 축소 시 인덱스 빌드가
실패하므로(트랜잭션째 롤백) 조용히 틀리지는 않는다. `alter_role` 은 축소가 재검사 없이 역할을
갱신하므로 **그 사이에 만들어진 위반 간선이 남을 수 있다.** 제거 계획: 축소 직전 재검증.

**계층 전체 유니크는 없다.** 검증은 부모·자식 테이블 전체의 중복을 보지만, 축소 후 강제는
구체 테이블마다의 유니크 인덱스다(002 의 `og_add_property(is_key)` 와 같은 한계).

**`og_schema_abort` 는 `add_property` 를 되돌리지 않는다.** `add_property` 는 확장에서
완결되며, 중단하면 속성이 남는다(코드 주석에 명시).

**`access.sql` 에 plpgsql 함수가 추가되었다.** [ADR-010](../../docs/99_decisions/ADR-010-access-sql-language-sql.md)
은 접근 경로를 `LANGUAGE sql` 로 두라고 한다. 이 스펙이 추가한 `og__castable` 은 예외를
잡아야 해서 plpgsql 이다. 접근 경로(집합 반환, 질의 계획 내부)가 아니라 마이그레이션이
행마다 부르는 스칼라 판정이므로 ADR-010 의 근거(옵티마이저 장벽)에 해당하지 않는다고
판단했다. 이중 쓰기 트리거 함수 `og__shadow_write` 와 013/014 의 트리거 함수도 같은
판단이다.
