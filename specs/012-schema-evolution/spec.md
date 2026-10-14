# Feature Specification: 스키마 진화 (Schema Evolution — Expand / Migrate / Validate / Contract)

**Feature Branch**: `012-schema-evolution`

**Created**: 2026-09-30

**Status**: Implemented (partial — 아래 Out of Scope 참고)

**Input**: `그래프DB 스키마 변경 운영 가이드.pdf`(저장소 루트) — "기존 데이터가 많은 상태에서
필드가 추가되거나 타입이 변경되는 케이스들을 뽑아 보고, Neo4j·TypeDB 가 어떻게 운영하는지
조사하자. 한 번에 바꾸기보다 새 스키마 추가 → 마이그레이션 → 검증 → 구 스키마 제거 순서로
쪼개고, 메타모델을 다루는 우리는 스키마 변경 자체를 1급 객체로 버전 관리하자."

## 개요

002 의 타입 시스템은 스키마를 **선언**할 수 있게 했다. 선언은 빈 그래프에서는 충분하지만,
데이터가 수백만 건 쌓인 뒤의 스키마는 선언이 아니라 **변경**의 대상이다. 그리고 변경은
한 문장이 아니다.

- 속성 타입을 `string` 에서 `int` 로 바꾸면 `"abc"` 를 가진 행은 어떻게 되는가?
- 필수 속성으로 바꾸는 순간 비어 있는 행은?
- 유니크 제약을 거는데 이미 중복이 있으면?
- 그 작업이 도는 동안 애플리케이션은 쓰기를 계속할 수 있는가?

Neo4j 는 제약 생성 시 전수 검사하고 위반이 있으면 생성 자체가 실패한다. TypeDB 는
`redefine` 을 트랜잭션으로 수행하되 일관성 조건을 엄격히 요구한다. 두 경우 모두 운영
현장의 답은 같다 — **Expand → Migrate → Validate → Contract** 로 쪼갠다.

이 스펙은 그 네 단계를 DB 기능으로 제공하고, 각 변경을 **카탈로그에 기록되는 1급 객체**
(`og_catalog.schema_change`)로 만든다. 그리고 이 저장소의 저장 구조가 주는 이점을
명시적으로 이용한다: 속성 이름·타입 이름·계층 위치는 저장소를 **가리키는 카탈로그 항목**
이므로(컬럼은 `p_<name>` 으로 태어났을 뿐이고 테이블은 `n_<type_id>` 다), 대부분의
변경은 행을 건드리지 않는다. 행을 다시 쓰는 것은 **표현(representation)이 바뀌는 변경**
— 컬럼 타입 변경, 기본값 채우기 — 뿐이다.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 잘못된 속성 타입을 운영 중에 고친다 (Priority: P1)

모델러가 `Person.age` 를 `string` 으로 선언했다. 데이터가 쌓인 뒤 이를 `int` 로 바꾸려 한다.
애플리케이션은 멈추지 않는다.

**Why this priority**: 가이드가 꼽은 "난이도 높은 변경"의 대표이며, 네 단계가 모두 의미를
갖는 유일한 종류다.

**Independent Test**: `engine/tests/sql/08_schema_evolution.sql` 의 alter_property_type 구간,
temporal 타입의 이력 테이블은 `engine/tests/sql/10_evolution_temporal_coverage.sql`.

**Acceptance Scenarios**:

1. **Given** 값 `"31"`, `"42"`, `"abc"`, NULL 을 가진 행들이 있을 때, **When** 변경을
   계획(plan)하면, **Then** 아무것도 바뀌지 않고 캐스트 불가능한 값의 수와 해당 행 id 가
   보고된다.
2. **Given** 확장(expand)된 상태에서, **When** 새 타입으로 캐스트되지 않는 값을 쓰면,
   **Then** 그 쓰기는 즉시 거부된다 — 엔진을 거치지 않는 원시 SQL 쓰기도 같다. 캐스트되는
   값은 쓰이고, 읽기는 여전히 기존 컬럼을 본다.
3. **Given** 확장된 상태에서, **When** 배치 크기를 지정해 마이그레이션을 반복 호출하면,
   **Then** 각 호출은 최대 배치 크기만큼만 옮기고, 중단 후 재호출하면 이어서 진행하며,
   끝나면 `done: true` 를 돌려준다.
4. **Given** 캐스트 불가 행이 남아 있을 때, **When** 검증(validate)하면, **Then** 위반 수와
   행 id 가 반환되고 단계는 `validated` 로 넘어가지 않으며 contract 는 거부된다.
5. **Given** 위반 행을 평범한 쓰기로 고친 뒤, **When** 검증과 축소(contract)를 수행하면,
   **Then** 컬럼은 같은 이름의 새 타입이 되고, 비교는 문자열이 아니라 수치로 동작하며,
   그 컬럼의 인덱스가 재생성되고, 하위 타입의 카탈로그도 새 타입을 가리킨다.

---

### User Story 2 - 이름과 계층은 데이터 크기와 무관하게 바뀐다 (Priority: P1)

모델러가 `name` 을 `fullName` 으로, `Employee` 를 `Staff` 로 바꾸고, `Person` 위에 새
추상 상위 타입 `Agent` 를 끼워 넣는다.

**Why this priority**: 이것이 "스키마를 카탈로그로 푼다"는 설계의 핵심 이득이다. 행 수에
비례하는 비용이 없어야 한다.

**Independent Test**: 08_schema_evolution.sql 의 rename_property, rename_type,
add_parent/remove_parent 구간. add_parent 는 저장 테이블 모든 행의 `xmin` 이 변하지 않음을
단언한다.

**Acceptance Scenarios**:

1. **Given** 속성 이름 변경이 확장된 상태에서, **When** 옛 이름과 새 이름으로 각각 읽고
   쓰면, **Then** 둘 다 같은 컬럼을 가리키며, 새 이름은 하위 타입에도 적용된다.
2. **Given** 이름 변경이 축소되면, **When** 옛 이름을 조회하면, **Then** 카탈로그에 옛
   이름이 남아 있지 않고 기존 데이터는 새 이름으로 읽힌다.
3. **Given** 타입 이름을 바꾸면, **When** 새 이름으로 `MATCH` 하면, **Then** 같은 노드들이
   반환되고 노드 id 는 변하지 않는다.
4. **Given** 기존 타입에 상위 타입을 추가하면, **When** 상위 타입으로 질의하면, **Then**
   기존 인스턴스가 모두 포함되고, 저장된 행은 하나도 다시 쓰이지 않는다.
5. **Given** 순환을 만드는 상위 타입 추가를 계획하면, **When** 실행하면, **Then** 거부된다.

---

### User Story 3 - 제약은 새 쓰기에 먼저, 기존 데이터에 나중에 걸린다 (Priority: P1)

데이터 스튜어드가 `age` 를 필수로, `fullName` 을 유니크로 만든다. 비어 있는 행과 중복이
이미 있다.

**Why this priority**: Neo4j 에서 "제약 생성이 실패한다"로 끝나는 지점이다. 위반을 보고하고,
고칠 시간을 주고, 그동안 새 위반이 생기지 않게 해야 한다.

**Independent Test**: 08_schema_evolution.sql 의 set_required, add_unique 구간.

**Acceptance Scenarios**:

1. **Given** 필수화 변경을 계획하면, **When** 보고를 보면, **Then** 값이 없는 행 수와
   id 가 있다.
2. **Given** 필수화가 확장되면, **When** 그 속성 없이 새 행을 쓰면, **Then** 즉시
   거부된다(기존 행은 아직 검사하지 않는다).
3. **Given** 기본값이 지정되어 있으면, **When** 마이그레이션하면, **Then** 빈 행이 배치
   단위로 채워진다.
4. **Given** 검증과 축소가 끝나면, **When** 컬럼을 보면, **Then** `NOT NULL` 이다.
5. **Given** 유니크 추가를 검증하면, **When** 부모·자식 타입에 걸친 중복 값이 있으면,
   **Then** 중복 값이 보고되고 단계는 넘어가지 않는다.

---

### User Story 4 - 스키마 변경은 기록이다 (Priority: P2)

감사자가 "이 노드가 쓰일 당시 스키마는 어땠나?", "이 컬럼은 언제 누가 int 로 바꿨나?"를
묻는다.

**Why this priority**: 가이드의 "스키마 변경 자체를 1급 객체로 버전 관리" 요구이자,
013(시점 질의)의 세 번째 축(스키마 버전)이다.

**Independent Test**: 08_schema_evolution.sql 의 "the schema record" 구간, 시각으로 조회하는
스냅샷은 10_evolution_temporal_coverage.sql.

**Acceptance Scenarios**:

1. **Given** 변경이 축소되면, **When** 변경 기록을 조회하면, **Then** 종류, 단계, 계획 시
   보고, 생성자, 시작·종료 스키마 버전이 있다.
2. **Given** 변경 전 버전을 지정해 스냅샷을 조회하면, **When** 결과를 보면, **Then** 그
   시점의 타입·속성·데이터 타입이 반환된다.
3. **Given** 그래프 하나에 여러 변경이 있으면, **When** 목록을 조회하면, **Then** 최신순으로
   반환된다.

---

### User Story 5 - 작은 그래프에서는 한 번에 한다 (Priority: P3)

개발 환경에서 모델러는 네 단계를 따로 부르고 싶지 않다.

**Acceptance Scenarios**:

1. **Given** 위반이 없는 변경은, **When** 한 번에 적용하면, **Then** 네 단계가 한
   트랜잭션에서 수행된다.
2. **Given** 위반이 있는 변경은, **When** 한 번에 적용하면, **Then** 실패하고 **아무것도
   바뀌지 않는다**(역할 제약이 그대로 남아 있음을 테스트가 확인한다).

---

### Edge Cases

- **상속된 속성의 변경**: 하위 타입에 복사된 속성을 하위 타입에서 삭제·개명·재타입하면 상위와
  어긋난다. 선언된 타입에서만 허용하고, 그 외에는 `'x' is inherited from 'Y'; change it there`
  로 거부한다.
- **캐스트가 반올림하는 경우**: `3.5::int8` 은 반올림되지만 `'3.5'::int8` 은 실패한다.
  마이그레이션은 **텍스트를 거쳐** 캐스트한다 — 조용히 반올림하는 타입 변경을 원한 사람은 없다.
- **이력 테이블**: 013 의 `h_<type_id>` 가 있는 타입은 이력도 같은 변경을 받아야 한다.
  삭제된 컬럼은 이력에서 지우지 않는다 — 이력이기 때문이다.
- **단계 위반**: `planned` 인 변경을 contract 하려 하면 `OG_SCHEMA_PHASE`.
- **중단된 변경**: 확장 이후 포기하면 추가된 구조(그림자 컬럼, NOT VALID 제약, 두 번째 이름)를
  걷어낸다.
- **권한**: 확장 도중 타입 변경 중인 속성에 쓰기 권한이 있는 역할은 그 속성을 계속 쓸 수 있어야 한다. 그림자 컬럼은 트리거가 채우고, PostgreSQL 은 문장에 적힌 컬럼에만 컬럼 권한을 검사하므로 그림자 컬럼에 별도 권한이 필요 없다.

## Requirements *(mandatory)*

### Functional Requirements

**변경 기록**

- **FR-001**: 시스템은 스키마 변경을 `(종류, 명세 jsonb, 단계, 시작 버전, 종료 버전, 생성자,
  시각, 보고)` 를 갖는 카탈로그 레코드로 저장해야 한다.
- **FR-002**: 변경은 `planned → expanded → migrating → validated → contracted` 순서로만
  진행되고, 축소 전 어느 단계에서든 `aborted` 로 끝낼 수 있어야 한다. 순서 위반은
  구조화된 오류(`OG_SCHEMA_PHASE`, SQLSTATE 55000)로 거부한다.
- **FR-003**: 계획 단계는 아무것도 바꾸지 않고 영향(행 수, 위반 표본 id, 잠금 성격)을
  보고해야 한다.

**단계**

- **FR-004**: 확장은 새 구조를 옛 구조 옆에 추가해야 하며, 확장 이후의 새 쓰기는 새 규칙을
  즉시 만족해야 한다.
- **FR-005**: 마이그레이션은 호출당 최대 `batch` 행만 처리하고, 커서를 변경 레코드에 저장해
  재개 가능해야 한다.
- **FR-006**: 검증은 위반 수와 표본을 보고하고, 위반이 0 일 때만 `validated` 로 넘어가야
  한다.
- **FR-007**: 축소는 `validated` 에서만 가능하며, 옛 구조를 제거하고 새 구조에 원래 이름을
  준다.
- **FR-008**: 네 단계를 한 트랜잭션에서 수행하는 일괄 적용을 제공해야 하며, 검증 실패 시
  아무것도 바꾸지 않고 `OG_SCHEMA_VIOLATION`(SQLSTATE 23514)으로 실패해야 한다.

**스냅샷**

- **FR-009**: 스키마 버전이 올라갈 때마다 그 그래프의 카탈로그 전체(타입, 상위 타입, 속성과
  데이터 타입·컬럼·필수·키, 역할, temporal/stamped 여부)를 스냅샷으로 저장하고, 버전 또는
  시각으로 조회할 수 있어야 한다 — "이것은 어떤 구조에서 쓰였는가?"

**변경 종류**

- **FR-010**: `add_property` — 선택 속성 추가. 행 재작성 없음. `__ext` 에 있던 같은 이름의
  값을 컬럼으로 옮긴다. 필수로 추가하는 것은 거부하고 `set_required` 를 안내한다.
- **FR-011**: `drop_property` — 축소 시 컬럼 제거. 이력 테이블은 컬럼을 유지한다.
- **FR-012**: `rename_property` — 카탈로그만. 확장 시 같은 컬럼에 두 번째 이름을 주고,
  축소 시 옛 이름을 지운다. 이름을 참조하는 쓰기 권한 기록과 호환 인덱스 기록도 따라간다.
- **FR-013**: `alter_property_type` — 그림자 컬럼 + 이중 쓰기(전환 중에만 붙는 트리거라 원시
  SQL 쓰기도 포함) + 배치 캐스트 + 컬럼 교체.
  인덱스 재생성, 컬럼 권한 재적용, 이력 테이블 동반 변경.
- **FR-014**: `set_required` — 확장 시 `CHECK … NOT VALID`, 마이그레이션 시 기본값 채우기,
  검증 시 `VALIDATE CONSTRAINT`, 축소 시 `SET NOT NULL`.
- **FR-015**: `add_unique` — 계층 전체의 중복을 보고하고, 축소 시 구체 테이블마다 유니크
  인덱스를 만든다.
- **FR-016**: `rename_type` — 카탈로그만. 식별자 불변.
- **FR-017**: `alter_role` — 역할의 player 타입·카디널리티 변경. 기존 간선·role player 를
  검사한다.
- **FR-018**: `add_parent` / `remove_parent` — 카탈로그 + 구간 재라벨링. 상속 컬럼은 재작성
  없이 추가되고, temporal·stamping·쓰기 권한도 상속된다. 제거 시 상속받았던 속성은 자기
  속성으로 남는다.
- **FR-019**: 상속된 속성은 선언된 타입에서만 삭제·개명·재타입할 수 있다.

**부수 효과**

- **FR-020**: 스키마를 바꾸는 모든 경로(`og_create_index`, `og_map_table`,
  `og_materialize_mapping` 포함)는 스키마 버전을 올려 Cypher 계획 캐시를 무효화해야 한다.

### Key Entities

- **Schema Change**: `og_catalog.schema_change` 한 행. 종류, 명세, 단계, 보고(커서 포함).
- **Schema Snapshot**: `og_catalog.schema_version.snapshot` — 버전별 카탈로그 문서.
- **Shadow Column**: 타입 변경 중 새 타입을 담는 `p_<name>__s<change_id>` 컬럼.

## Success Criteria *(mandatory)*

- **SC-001**: 08_schema_evolution.sql 과 10_evolution_temporal_coverage.sql 의 012 구간의 모든
  단언이 통과한다(현재 통과).
- **SC-002**: `add_parent` 가 기존 행을 하나도 다시 쓰지 않는다 — `xmin` 불변으로 증명
  (현재 통과).
- **SC-003**: 위반이 있는 일괄 적용이 카탈로그를 바꾸지 않는다(현재 통과).
- **SC-004**: 카탈로그 수준 변경(rename, re-parent, drop)의 소요 시간이 데이터 크기와
  무관하다. 측정은 [`docs/schema-evolution-benchmark.md`](../../docs/schema-evolution-benchmark.md)
  에 둔다.

## Assumptions

- 타입·속성·역할 카탈로그와 구간 라벨링은 스펙 **002** 가 제공한다.
- 스키마 버전과 계획 캐시 키는 스펙 **003**/**008** 의 `schema_version` 을 쓴다.
- 이력 테이블 동반 변경은 스펙 **013** 에 의존한다.
- 쓰기 권한 기록의 재적용은 스펙 **014** 에 의존한다.
- 스키마 변경 함수는 [ADR-025](../../docs/99_decisions/ADR-025-privilege-model-default-deny.md)
  에 따라 admin 전용이다(READ/WRITE 목록에 없음). 조회(`og_schema_changes`,
  `og_schema_snapshot`)만 read 이다.

## Out of Scope (구현되지 않음)

- **타입 분리/병합**: 노드 id 가 타입 id 를 인코딩하므로([ADR-003](../../docs/99_decisions/ADR-003-int8-identifier-bitfields.md))
  노드를 다른 타입으로 옮기면 재식별과 그 id 를 가리키는 모든 인접 항목의 재작성이 필요하다.
  이 파이프라인에 없다.
- **TypeQL `redefine` / `undefine`** 를 이 파이프라인으로 매핑하는 것.
- **계층 전체 유니크**: `add_unique` 의 검증은 계층 전체를 보지만, 축소 후 강제는 구체
  테이블마다의 유니크 인덱스다. 부모와 자식 테이블에 같은 값을 새로 쓰는 것은 막지 못한다.
- 관계 방향 변경(가이드의 "관계 방향 재정의")은 별도 종류로 존재하지 않는다.
