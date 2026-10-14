# Feature Specification: 이중 시간 그래프 (Bitemporal Graph)

**Feature Branch**: `013-bitemporal-graph`

**Created**: 2026-09-30

**Status**: Implemented (partial — 아래 Out of Scope 참고)

**Supersedes / extends**: 스펙 **008** User Story 4, FR-018..FR-023, tasks T019.
008 의 `og_enable_history` / `og_history` / `og_as_of` 는 그대로 남는다(엔티티 단위 변경
로그와 기록 시각 기준 단건 조회). 이 스펙은 그 위에 **유효 시각과 기록 시각을 분리한,
질의 전체에 적용되는 시점**을 더한다.

**Input**: `그래프DB 스키마 변경 운영 가이드.pdf` — "유효 시점 from/to 를 포함해 어떤 시점에
유효한 정보인가를 관리하는 버저닝… 사이퍼로 질의할 때 이 시점이다 라는 옵션만 주면 백그라운드
메커니즘이 그 시점에 유효한 정보만 가져오고, 사용자는 시점의 디테일을 관리하지 않아도 되는
프레임워크."

## 개요

지식에는 세 개의 시간 축이 있다.

| 축 | 질문 | 이 스펙에서 |
|---|---|---|
| 유효 시각 (valid time) | 현실에서 언제부터 언제까지 참이었나? | 모든 행의 `__valid tstzrange` |
| 기록 시각 (system time) | 우리는 언제부터 그렇게 알고 있었나? | 현재 행의 `__sys_from`, 이력 행의 `[__sys_from, __sys_to)` |
| 스키마 버전 | 그 당시 구조는 어땠나? | 스펙 012 의 `og_catalog.schema_version.snapshot` |

두 시간 축을 분리해야 "과거의 현실"과 "과거에 우리가 알던 것"을 구분해 재현할 수 있다.
감사·재현·설명 책임에서 결정적인 차이다. 008 의 이력은 기록 시각 하나만 가졌고, 한 번에
엔티티 하나만 되돌려 볼 수 있었다.

사용자 요구의 핵심은 **질의가 시점만 말하면 된다**는 것이다. 이 스펙에서 질의는 Cypher 를
한 글자도 바꾸지 않는다. 세션(또는 Bolt 트랜잭션 메타데이터)에 `og.valid_at` / `og.system_at`
을 설정하면 같은 질의가 그 시점의 그래프에 답한다. 그리고 그렇게 답할 수 없는 질의 모양은
**오늘의 데이터로 답하는 대신 오류를 낸다** — 008 FR-021 의 원칙("현재 값을 대신 반환해서는
안 된다")을 질의 전체로 넓힌 것이다.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 같은 질의를 과거 시점에 묻는다 (Priority: P1)

에이전트가 `MATCH (p:Person {name:"ann"}) RETURN p.salary` 를 실행한다. 설정만 바꿔 "2021년
기준", "2023년 기준, 단 정정 이전에 알던 대로"를 묻는다.

**Why this priority**: 이 스펙의 존재 이유.

**Independent Test**: `engine/tests/sql/09_bitemporal.sql` — 2020 입사(급여 100) → 2023 부터
인상(200) → 정정(250, 처음부터 그랬다) → 2024-06 퇴사.

**Acceptance Scenarios**:

1. **Given** 위 이력에서 시점을 설정하지 않으면, **When** 질의하면, **Then** 250 과 "직장
   없음"이 반환된다.
2. **Given** `og.valid_at = 2021-06-01` 이면, **When** 같은 질의를 하면, **Then** 100 과
   `acme/eng` 가 반환된다.
3. **Given** `og.valid_at = 2023-06-01` 이면, **Then** 정정이 반영된 250 이 반환된다.
4. **Given** 같은 유효 시각에 `og.system_at` 을 정정 이전으로 두면, **Then** 200 이,
   인상 기록 이전으로 두면 100 이 반환된다.
5. **Given** 퇴사 기록 이전의 기록 시각이면, **When** 2025 년의 직장을 물으면, **Then**
   `acme/eng` 가 반환된다(그때는 몰랐다).
6. **Given** 시점이 달라져도, **When** 컴파일된 SQL 을 비교하면, **Then** 동일하다.

---

### User Story 2 - 쓰기는 언제부터 참인지를 말한다 (Priority: P1)

애플리케이션이 "이 인상은 2023-01-01 부터 유효", "이건 변경이 아니라 정정"을 쓰기 시점에
표시한다.

**Why this priority**: 유효 시각은 쓰는 쪽만 알 수 있다.

**Acceptance Scenarios**:

1. **Given** `og.valid_from` 을 설정하고 생성하면, **Then** 행의 유효 구간이 그 시각부터
   시작한다.
2. **Given** 기존 행을 `og.valid_from = X` 로 갱신하면, **Then** 옛 믿음은 이력에서 기록
   시각이 닫히고, 옛 값의 `[시작, X)` 구간은 "여전히 믿는 과거"로 다시 기록되며, 새 행은
   `[X, …)` 로 유효하다.
3. **Given** `og.valid_from = '-infinity'` 로 갱신하면, **Then** 과거 구간이 남지 않는 정정이
   된다.
4. **Given** X 가 현재 유효 구간의 끝 이후면, **Then** `OG_TEMPORAL_RANGE` 로 거부된다.
5. **Given** 원시 SQL `UPDATE` 로 바꿔도, **Then** 같은 방식으로 이력이 남는다.
6. **Given** 보이는 값이 바뀌지 않는 갱신(시스템 컬럼, 012 의 그림자 컬럼만 바뀜)이면,
   **Then** 이력이 생기지 않는다.

---

### User Story 3 - 답할 수 없으면 틀린 답 대신 오류 (Priority: P1)

에이전트가 과거 시점을 설정한 채 `MATCH (n) RETURN count(n)` 이나 가변 길이 경로를 실행한다.

**Why this priority**: 레지스트리(`og_node`)와 인접 세그먼트(`og_adj`)는 현재만 안다. 거기서
나온 답은 **그럴듯해 보이는 틀린 답**이다.

**Acceptance Scenarios**:

1. **Given** 시점이 설정되어 있고 그래프에 temporal 노드 타입이 있으면, **When** 레이블 없는
   노드 패턴을 쓰면, **Then** `OG_TEMPORAL_UNSUPPORTED`(SQLSTATE 0A000).
2. **Given** 같은 조건에서, **When** temporal 관계 타입 위의 가변 길이 경로를 쓰면, **Then**
   같은 오류.
3. **Given** 같은 조건에서, **When** TypeQL 을 실행하면, **Then** 같은 오류.
4. **Given** 시점을 해제하면, **When** 같은 질의를 실행하면, **Then** 정상 실행된다.

---

### User Story 4 - 삭제된 것도 과거에는 있었다 (Priority: P2)

**Acceptance Scenarios**:

1. **Given** 노드를 삭제하면, **When** 현재를 조회하면, **Then** 없다.
2. **Given** 삭제 이전의 기록 시각이면, **When** 조회하면, **Then** 있다.

---

### User Story 5 - Bolt 로 시점을 말한다 (Priority: P2)

Neo4j 드라이버를 쓰는 애플리케이션이 API 를 바꾸지 않고 시점을 지정한다.

**Independent Test**: `tests/bolt-context/run.py` (공식 neo4j Python 드라이버).

**Acceptance Scenarios**:

1. **Given** `tx_metadata` 에 `validFrom` 을 담아 쓰면, **Then** 행의 유효 구간이 그 날짜부터다.
2. **Given** 자동 커밋 `RUN` 이나 명시적 트랜잭션의 `tx_metadata` 에 `validAt`/`valid_at` 을
   담으면, **Then** 그 시점의 값이 반환된다.
3. **Given** 다음 트랜잭션이 메타데이터 없이 실행되면, **Then** 설정이 새지 않는다.

---

### Edge Cases

- **활성화 이전의 행**: 활성화 시점에 이미 있던 행은 유효 `[-∞, ∞)`, 기록 시각 = 활성화 시각.
  그 전에 무엇이 참이었는지 기록된 바가 없으므로, 그 이전 기록 시각을 물으면 **없음**이 답이다.
- **하위 타입**: 활성화는 하위 타입, 이후에 만들어지는 하위 타입까지 적용된다.
- **스키마 변경**: 저장 테이블에 추가된 컬럼과 타입 변경은 이력 테이블에 따라간다. 삭제된
  컬럼은 이력에서 지우지 않는다.
- **작성자 권한**: 쓰기 역할은 이력 테이블에 DML 을 갖지 않는다. 캡처 트리거가 SECURITY
  DEFINER 로 쓴다.
- **미래 유효 시각의 갱신·삭제**: 현재 행의 유효 구간이 미래에서 시작하고, 그때까지 유효한
  옛 값은 이력의 열린 행(`__sys_to = ∞`, op `v`)에만 남는다. 열린 이력 행은 여전히 믿는 것이므로
  현재의 일부다 — 시점을 설정하지 않은 질의도 그때까지는 옛 값을 본다(09 가 단언,
  plan.md Complexity Tracking).

## Requirements *(mandatory)*

### Functional Requirements

**활성화와 캡처**

- **FR-001**: 시스템은 타입 단위로(하위 타입 포함, 이후 생성되는 하위 타입 포함) 이중 시간
  버전 관리를 켤 수 있어야 한다. 엔티티와 관계 타입만 대상이다.
- **FR-002**: 생성 시 유효 구간은 `[og.valid_from 또는 now(), og.valid_to 또는 ∞)` 이어야 한다.
- **FR-003**: 갱신·삭제 시 옛 행은 이력으로 옮겨져 기록 구간이 닫혀야 하며, 변경 시점 이전의
  유효 구간은 현재 믿음으로 다시 기록되어야 한다.
- **FR-004**: `og.valid_from = '-infinity'` 는 정정으로 해석되어야 한다. 현재 유효 구간 끝
  이후의 변경은 `OG_TEMPORAL_RANGE`(22023)로 거부해야 한다.
- **FR-005**: 캡처는 트리거로 수행되어 원시 SQL 쓰기에도 동일하게 적용되어야 한다. 보이는
  값이 바뀌지 않는 갱신은 이력을 만들지 않아야 한다. 쓰기 역할은 이력 테이블을 수정할 수
  없어야 한다.

**시점 질의**

- **FR-006**: 질의는 `og.valid_at` / `og.system_at` 설정만으로 시점을 지정할 수 있어야 하며,
  설정이 없으면 현재를 뜻한다.
- **FR-007**: 생성 타입 뷰는 temporal 타입의 현재 행, 열린 이력 행(`__sys_to = ∞`), 닫힌 이력
  행을 그 시점으로 거른 합집합이어야 하며, 컴파일된 SQL 은 시점과 무관하게 동일해야 한다(계획
  캐시 유효). 열린 이력 행은 시점 설정과 무관하게 유효 구간이 그 시각을 포함하면 보여야 하고,
  시점이 없으면 닫힌 이력 분기는 스캔되지 않아야 한다.
- **FR-008**: temporal 관계 타입의 단일 홉은 인접 세그먼트 대신 간선 테이블(현재 + 열린 이력 +
  닫힌 이력)을 시점으로 걸러 읽어야 한다.
- **FR-009**: Bolt 게이트웨이는 `tx_metadata` 의 `valid_at`/`validAt`, `system_at`/`systemAt`,
  `valid_from`/`validFrom`, `valid_to`/`validTo` 를 트랜잭션 로컬 설정으로 적용해야 한다.
- **FR-010**: 현재만 볼 수 있는 질의 모양은 시점이 설정되어 있을 때 `OG_TEMPORAL_UNSUPPORTED`
  (0A000)로 거부해야 한다: temporal 노드 타입이 있는 그래프의 레이블 없는 노드 패턴, temporal
  관계 타입 위 가변 길이 경로, 단일 타입이 아닌 관계 변수, TypeQL.

**부수**

- **FR-011**: 저장 테이블의 스키마 변경(컬럼 추가, 타입 변경, 012 의 파이프라인)은 이력
  테이블에 반영되어야 한다.
- **FR-012**: `og_temporal_types(graph)` 로 temporal 타입을, `og_meta(id)` 로 한 요소의 현재
  유효 구간·기록 시각을 조회할 수 있어야 한다.
- **FR-013**: 이력 보존 정책(기간, 용량 상한) — 008 FR-023 과 동일. **미구현.**
- **FR-014**: 008 의 레거시 이력 경로를 바로잡는다: `og_as_of` 는 삭제 이후 시점에 NULL,
  같은 시각의 기록은 `hist_id` 로 순서, `og_enable_history` 이후에 만든 하위 타입도 캡처,
  `og_capture_history` 는 SECURITY DEFINER.

### Key Entities

- **Temporal Type**: `og_catalog.temporal` 한 행. 활성화 시각.
- **History Table**: `og_data.h_<type_id>` — 저장 테이블과 같은 컬럼 + `__sys_to`, `__op`
  (`u` 갱신 전, `d` 삭제 전, `v` 다시 기록된 과거 유효 구간).
- **Instant**: 세션 설정 `og.valid_at`, `og.system_at`. 쓰기 쪽은 `og.valid_from`, `og.valid_to`.

## Success Criteria *(mandatory)*

- **SC-001**: 09_bitemporal.sql 과 10_evolution_temporal_coverage.sql 의 013 구간의 모든 단언이
  통과한다(현재 통과) — 008 SC-006("과거 시점 질의 결과가 실제 상태와 100% 일치")의 검증
  가능한 판.
- **SC-002**: 시점을 바꿔도 컴파일된 SQL 이 동일하다(현재 통과).
- **SC-003**: 현재만 볼 수 있는 질의 모양이 시점 설정 시 틀린 답 대신 오류를 낸다(현재 통과.
  "단일 타입이 아닌 관계 변수"는 10 이 단언).
- **SC-004**: `tests/bolt-context/run.py` 가 공식 드라이버로 시점 지정·유효 시각 쓰기를
  검증한다(현재 통과).
- **SC-005**: 이력 활성화로 인한 쓰기 처리량 저하 — 008 SC-011(40% 이내). **측정하지 않았다.**

## Assumptions

- 타입 계층과 생성 타입 뷰는 스펙 **002**, **003** 이 제공한다.
- 스키마 버전 축은 스펙 **012** 의 스냅샷이다.
- 작성자 스탬프(`__created_by` 등)는 스펙 **014** 가 제공하며, `og_enable_temporal` 은 스탬프도
  함께 켠다.
- Bolt 메타데이터 전달은 스펙 **011** 의 게이트웨이를 확장한다.

## Out of Scope (구현되지 않음)

- TypeQL 의 시점 질의(시점 설정 시 거부된다).
- temporal 관계 타입 위 가변 길이 경로의 시점 질의(거부된다).
- 레이블 없는 노드 스캔의 시점 질의(거부된다).
- 보존 정책(FR-013).
- 과거 유효 구간의 소급 편집 — 갱신은 현재 행에만 적용된다. 이력의 `v` 구간을 고칠 방법이 없다.
