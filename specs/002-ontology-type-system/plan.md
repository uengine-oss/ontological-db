# Implementation Plan: 온톨로지 타입 시스템과 상속 인덱싱

**Branch**: `002-ontology-type-system` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

TypeDB의 개념 모델(entity/relation/attribute + role + 상속)을 PostgreSQL 카탈로그 테이블로
구현하고, **상속 판정을 재귀 없이 단일 범위 비교로** 수행하는 구간 라벨링 인덱스를 만든다.

**핵심 설계 결정**

1. **Nested-set 구간 라벨링**: 각 타입에 `(lft, rgt)` 를 부여. `X ⊂ Y` 판정 =
   `Y.lft <= X.lft AND X.rgt <= Y.rgt` — **단일 범위 비교**. 상위 타입 질의는
   `WHERE lft BETWEEN y.lft AND y.rgt` 하나로 모든 후손 타입 ID를 얻는다.
2. **여유 구간(gap) 예약**: 라벨을 1씩이 아니라 **1024 간격**으로 부여. 계층 중간 삽입 시
   재할당 없이 빈 구간을 쓴다. 구간이 고갈된 서브트리만 국소 재할당(FR-013).
3. **다중 상속 보조 인코딩**: 다중 부모 타입은 각 부모 경로마다 **추가 구간 라벨 행**을 갖는다
   (`og_type_label` 이 타입당 N행). 판정은 여전히 범위 비교이며 상한은 부모 경로 수.

## Technical Context

**Language/Version**: Rust 1.83+ / pgrx

**Primary Dependencies**: PostgreSQL 16 (카탈로그 테이블 + btree 인덱스)

**Storage**: `og_catalog` 스키마의 카탈로그 테이블. 트랜잭션·롤백 그대로 상속

**Testing**: `cargo pgrx test`, SQL 회귀, 계층 1000타입 부하 테스트

**Performance Goals**: 상속 판정 O(1), 깊이 1 질의 대비 5% 이내 (SC-001)

**Constraints**: 스키마 변경 중 읽기 무차단 (MVCC로 자동 충족)

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I. 확장 | ✅ | 순수 SQL 카탈로그 + Rust 함수 |
| III. 네이티브 저장 | ✅ | 카탈로그가 001의 컬럼 레이아웃을 결정 |
| **IV. 온톨로지 우선** | ✅ | 본 스펙이 원칙 IV의 구현체. 재귀 CTE 0개를 테스트로 강제 |
| VIII. 에이전트 | ✅ | 카탈로그 전체가 introspection 대상 (008) |
| IX. ACID | ✅ | 카탈로그도 일반 테이블 → 스키마 변경이 트랜잭션·롤백 가능 |

## Architecture

### 카탈로그 스키마

```sql
og_catalog.graph        (graph_id, name, created_at)
og_catalog.type         (type_id, graph_id, name, kind, abstract, storage_table, ...)
                        -- kind: 'entity' | 'relation' | 'attribute'
og_catalog.type_parent  (type_id, parent_id)              -- 다중 상속 DAG
og_catalog.type_label   (type_id, path_id, lft, rgt, depth) -- 구간 라벨 (다중 상속 시 N행)
og_catalog.property     (prop_id, type_id, name, data_type, required, is_key, ...)
og_catalog.role         (role_id, rel_type_id, name, player_type_id, min_card, max_card)
og_catalog.constraint   (con_id, target, kind, params jsonb)
og_catalog.rule         (rule_id, rel_type_id, characteristic)  -- transitive/symmetric/...
og_catalog.schema_version (version, changed_at, description)
```

### 상속 판정의 실체

```sql
-- "Vehicle의 모든 후손 타입" — 재귀 없음, 인덱스 범위 스캔 1회
SELECT DISTINCT l2.type_id
FROM og_catalog.type_label l1
JOIN og_catalog.type_label l2
  ON l2.lft >= l1.lft AND l2.rgt <= l1.rgt
WHERE l1.type_id = $vehicle;
```
`(lft, rgt)` btree 인덱스로 범위 스캔. **이것이 SC-003("계획에 재귀 노드 0개")의 근거이며,
자동 테스트가 `EXPLAIN` 출력에서 `Recursive` 문자열 부재를 검증한다.**

### 라벨 할당 알고리즘

```
GAP = 1024
assign(node, cursor):
    node.lft = cursor;  cursor += GAP
    for child in children: cursor = assign(child, cursor)
    node.rgt = cursor;  cursor += GAP
```
중간 삽입 시 부모-자식 사이 여유 구간을 사용. 고갈 시 해당 서브트리만 재할당하고
`schema_version` 을 올린다.

### 제약 강제 지점

| 제약 | 강제 위치 |
|------|-----------|
| required / cardinality | 노드/엣지 쓰기 경로 (Rust, 001 호출 전) |
| key (유일성) | 타입 테이블의 UNIQUE 인덱스 (PG가 강제) |
| role player type | 엣지 생성 시 `type_label` 범위 비교 1회 |
| value domain | 타입 테이블의 CHECK 제약 (PG가 강제) |

**설계 원칙**: 가능한 제약은 PostgreSQL 네이티브 제약으로 내려 강제 비용을 0에 수렴시킨다
(SC-009: 쓰기 처리량 저하 15% 이내의 근거).

## Project Structure

```text
engine/src/catalog/
├── mod.rs
├── types.rs          # 타입 CRUD, DDL 생성, 상속 등록
├── labeling.rs       # nested-set 라벨 할당/재할당/판정
├── roles.rs          # role 선언·검증
├── constraints.rs    # 제약 선언·강제
├── evolution.rs      # 스키마 진화, 버전
├── inference.rs      # 관계 특성 추론 (transitive/symmetric/inverse)
└── introspect.rs     # 카탈로그 조회 뷰 (008이 소비)
```

## Phasing

| Phase | 내용 | FR |
|-------|------|-----|
| **P0** | 카탈로그 스키마, 타입 CRUD, 단일 상속 + 라벨링 | FR-001,002,009,010,011 |
| **P1** | 다중 상속(보조 라벨), 순환 탐지, abstract | FR-003,007,012 |
| **P2** | role, n-ary, role specialization | FR-004,005,006 |
| **P3** | 제약 전체, 위반 오류 포맷 | FR-015~020 |
| **P4** | 스키마 진화, 중간 삽입, 버전 | FR-021~026 |
| **P5** | 관계 특성 추론 | FR-027~030 |
| **P6** | DDL 내보내기/재현 | FR-031,032 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| 다중 상속 시 타입당 복수 라벨 행 | 단일 nested-set은 트리만 표현 가능. DAG는 부모 경로마다 라벨이 필요 | 비트셋 인코딩은 타입 수 증가 시 폭이 선형 증가하고 스키마 변경 시 전체 재작성. 전이 폐포 materialize는 계층 변경 비용이 O(후손²) |
| 라벨 GAP=1024 로 인한 공간 낭비 | 중간 삽입 시 재할당 회피 (SC-005: 10초 이내) | GAP=1 은 삽입마다 전체 재할당 → 대형 온톨로지에서 수십 초 |
