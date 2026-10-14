# Feature Specification: 쓰기 시점 보안 컨텍스트 (Write-Time Security Context)

**Feature Branch**: `014-write-security-context`

**Created**: 2026-09-30

**Status**: Implemented (partial — 아래 Out of Scope 참고)

**Input**: `그래프DB 스키마 변경 운영 가이드.pdf` — "Neo4j 는 create, set property, delete,
merge 같은 쓰기 연산을 privilege 로 쪼개 노드·관계·속성 단위로 제한한다. LLM 이 Cypher 를
아무리 생성해도 DB 가 최종적으로 막아준다. Palantir 는 Action 이라는 단위로 '쓸 수 있는 행위'
자체를 정의하고 submission criteria 를 붙인다. 입력과 수정 시점부터 시큐리티 컨텍스트가
적용되는 구조 — Write Policy, Context Injection, System Managed Property 통제."

## 개요

[ADR-025](../../docs/99_decisions/ADR-025-privilege-model-default-deny.md) 는 권한을
`read ⊂ write ⊂ admin` 세 단계로 만들었다. `write` 는 **모든 타입의 모든 속성**을 쓸 수 있다.
사람 애플리케이션에는 그걸로 충분했지만, 에이전트에게 쓰기를 주는 순간 질문이 바뀐다.

- 이 에이전트는 `Person` 을 만들 수는 있지만 `salary` 는 못 쓰게 할 수 있나?
- 누가 썼는지를 **행이 스스로** 말하게 할 수 있나 — 에이전트가 `__created_by` 를 위조하지
  못하게?
- 에이전트가 "직원 채용" 한 가지만, 정해진 파라미터로만, 조건이 맞을 때만 쓰게 할 수 있나?
- 그리고 이 모든 것이 **원시 SQL 에도** 성립하나?

마지막 질문이 설계를 정했다. 엔진이 Cypher 를 파싱해 검사하는 방식은 파서가 틀리는 순간,
또는 누군가 엔진을 우회해 SQL 을 쓰는 순간 무너진다. 이 스펙은 세 질문에 세 개의
PostgreSQL 메커니즘으로 답한다: **테이블·컬럼 GRANT**(무엇을 쓸 수 있나), **트리거가 세션에서
찍는 시스템 컬럼**(누가·어디서), **SECURITY DEFINER 함수**(어떤 행위만). 엔진 쪽 검사는 읽기
좋은 오류 메시지를 위해서만 존재한다.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 연산·타입·속성 단위 쓰기 권한 (Priority: P1)

DBA 가 `clerk` 역할에 "Person 생성과 name 수정만" 을 준다.

**Why this priority**: 가이드가 인용한 Neo4j 의 `CREATE ON … NODES L`, `SET PROPERTY {p}` 의
대응물이다.

**Independent Test**: `engine/tests/sql/07_write_security.sql` 의 fine-grained grants 구간.

**Acceptance Scenarios**:

1. **Given** `og_grant_write('clerk', g, 'Person', ['create','set'], ['name'])`, **When**
   clerk 가 `CREATE (:Person {name:…})` 와 `SET p.name = …` 을 하면, **Then** 성공한다.
2. **Given** 같은 권한, **When** 하위 타입 `Employee` 를 만들면, **Then** 성공한다(부여가
   하위 타입에 닿는다).
3. **Given** 같은 권한, **When** `salary` 를 쓰거나, 선언되지 않은 속성을 쓰거나, 삭제하거나,
   다른 타입(`Doc`)을 만들면, **Then** `OG_WRITE_DENIED: role 'clerk' may not … (property
   'salary')` 류의 오류(SQLSTATE 42501)로 거부된다.
4. **Given** 같은 권한, **When** clerk 가 저장 테이블에 원시 SQL `UPDATE … SET p_salary` 나
   `DELETE` 를 하면, **Then** PostgreSQL 이 거부한다.
5. **Given** clerk 가 쓸 수 없는 타입의 행을 레지스트리(`og_node`)에 원시 SQL 로 넣으면,
   **Then** 행 수준 보안이 거부한다.
6. **Given** `og_revoke_write(…, ['create'])`, **When** 생성하면 거부되고, `set` 은 남는다.

---

### User Story 2 - 누가·어디서 썼는지는 행이 말한다 (Priority: P1)

감사자가 노드를 보고 "어느 DB 역할이, 어느 에이전트를 대신해, 어느 출처에서" 썼는지 묻는다.

**Independent Test**: 07 의 system-managed stamps 구간, `tests/bolt-context/run.py`.

**Acceptance Scenarios**:

1. **Given** 타입에 스탬프가 켜져 있고 세션에 `og.actor`, `og.source` 가 설정되어 있으면,
   **When** 쓰면, **Then** `og_meta(id)` 가 `created_by`(DB 역할), `actor`, `source`,
   `created_at` 을 돌려준다.
2. **Given** 테이블 전체 UPDATE 권한을 가진 역할이 원시 SQL 로 `__created_by`, `__actor` 를
   덮어쓰면, **Then** 트리거가 세션 값으로 되돌린다(위조 불가).
3. **Given** 스탬프가 켜진 타입의 하위 타입을 나중에 만들면, **Then** 그 하위 타입도 찍힌다.
4. **Given** Bolt 드라이버가 `tx_metadata = {actor, source}` 를 보내면, **Then** 같은 값이
   행과 감사 로그에 남고, 다음 트랜잭션으로 새지 않는다.

---

### User Story 3 - 에이전트는 정해진 행위만 한다 (Priority: P1)

운영자가 읽기 권한만 가진 에이전트에게 `hire(name: string)` 행위 하나를 준다. 조건: 같은
이름이 없을 것.

**Why this priority**: 가이드가 인용한 Palantir Action 의 대응물. "LLM 이 어떤 Cypher 를
만들어도" 쓸 수 있는 것은 이 행위뿐이어야 한다.

**Independent Test**: 07 의 actions 구간.

**Acceptance Scenarios**:

1. **Given** 에이전트가 자기 Cypher 로 `CREATE` 하면, **Then** 거부된다(42501).
2. **Given** `og_run_action(g, 'hire', {"name":"ada"})`, **Then** 행위의 소유 역할 권한으로
   생성된다.
3. **Given** 같은 파라미터로 다시 실행하면, **Then** 전제 조건이 거짓이므로
   `OG_ACTION_REJECTED`(23514).
4. **Given** 선언되지 않은 파라미터나 타입이 틀린 파라미터를 주면, **Then** 22023.
5. **Given** 에이전트가 실행기 `og__action_exec` 를 직접 부르면, **Then** 거부된다(42501).
6. **Given** 행위 실행 권한이 없는 역할이 실행하면, **Then** 거부된다(42501).
7. **Given** 행위로 쓴 쓰기는, **When** 감사 로그를 보면, **Then** 행위 이름이 남아 있다.
8. **Given** 본문이 오류로 끝난 행위 뒤의 질의는, **When** 감사 로그를 보면, **Then** 그 행에는
   행위 이름이 없다.

---

### User Story 4 - 감사 기록은 쓰는 사람이 고칠 수 없다 (Priority: P1)

**Acceptance Scenarios**:

1. **Given** 쓰기 역할이, **When** `og_audit` 을 `DELETE`/`UPDATE` 하거나 이력 테이블에
   쓰려 하면, **Then** 거부된다.
2. **Given** 읽기 역할이 질의하면, **Then** 감사 행이 `db_user` 와 함께 남는다.

---

### User Story 5 - 예약된 이름은 쓸 수 없다 (Priority: P2)

**Acceptance Scenarios**:

1. **Given** 속성 `_type`, `_id`, `_src`, `_dst` 나 `__` 로 시작하는 속성을 쓰면, **Then**
   `OG_RESERVED_PROPERTY`(42501).

---

### Edge Cases

- **쓰기 역할과 선언되지 않은 속성**: 스키마 변경 권한이 없는 쓰기 역할이 새 속성을 쓰면 컬럼
  승격([ADR-006](../../docs/99_decisions/ADR-006-write-time-property-promotion.md)) 대신
  `__ext` 에 남는다. 이전에는 `og_add_property` EXECUTE 에서 트랜잭션이 실패했다.
- **부여 이후 생긴 저장소**: 새 하위 타입 테이블, 새 속성 컬럼, 012 의 그림자 컬럼에도 기록된
  부여가 재적용된다.
- **아직 생성되지 않은 뷰**: `og_data` 에 CREATE 권한이 없는 읽기 역할도 처음 질의하는 타입의
  생성 뷰를 만들 수 있어야 한다.
- **행위의 소유 역할**: `og_define_action` 호출자는 `run_as` 로 `SET ROLE` 할 수 있어야
  한다(함수 소유권 이전 조건).

## Requirements *(mandatory)*

### Functional Requirements

**쓰기 권한 (Write Policy)**

- **FR-001**: 시스템은 역할·타입·연산(`create`/`set`/`delete`)·속성 목록 단위의 쓰기 권한을
  부여·회수할 수 있어야 한다. 속성 목록 NULL 은 선언되지 않은 속성(`__ext`)을 포함한 전부다.
- **FR-002**: 쓰기 권한은 타입별 저장 테이블에 대한 PostgreSQL 테이블/컬럼 권한으로
  구현되어, 원시 SQL 에도 동일하게 적용되어야 한다.
- **FR-003**: 타입에 대한 부여는 하위 타입, 이후 생기는 하위 타입, 이후 생기는 컬럼에 닿아야
  한다.
- **FR-004**: 공유 레지스트리(`og_node`, `og_edge`, `og_adj`, `og_role_player`)의 행은 그
  행의 타입 저장 테이블에 쓸 수 있는 역할만 쓸 수 있어야 한다(행 수준 보안).
- **FR-005**: 엔진 경로의 쓰기는 PostgreSQL 이 거부하기 전에 역할·연산·타입·속성을 명시한
  `OG_WRITE_DENIED`(42501)로 실패해야 한다.
- **FR-006**: `_id`, `_type`, `_src`, `_dst` 와 `__` 로 시작하는 이름은 속성으로 쓸 수 없고
  부여할 수도 없어야 한다(`OG_RESERVED_PROPERTY`, 42501).

**시스템 관리 속성 (System Managed Property)**

- **FR-007**: 타입 단위로 `__created_at`, `__created_by`, `__updated_at`, `__updated_by`,
  `__actor`, `__source` 스탬프를 켤 수 있어야 하며, 하위 타입(이후 생성 포함)에 상속되어야 한다.
- **FR-008**: 스탬프 값은 세션(`current_user`, `og.actor`, `og.source`)에서만 오며, 행에
  쓴 값은 무시되어야 한다 — 원시 SQL 포함.
- **FR-009**: `og_meta(id)` 로 한 요소의 시스템 컬럼을 읽을 수 있어야 한다.

**컨텍스트 주입과 감사 (Context Injection)**

- **FR-010**: 세션 설정 `og.actor`, `og.source`, `og.reason`, `og.request_id` 는 감사 로그에
  기록되어야 하고, Bolt 게이트웨이는 `tx_metadata` 의 `actor`, `source`, `reason`, `request_id`/`requestId`
  를 트랜잭션 로컬 설정으로 적용해야 한다. 감사 로그는 `db_user`(검증된 `current_user`),
  스키마 버전, 행위 이름을 가져야 하며, 쓰기 역할은 감사·이력·출처 테이블에 DML 을 갖지
  않아야 한다.

**행위 (Action gate)**

- **FR-011**: 관리자는 그래프에 이름 있는 행위(본문: Cypher 또는 TypeQL 한 문장)를 정의할 수
  있어야 한다.
- **FR-012**: 각 행위는 `run_as` 역할이 소유한 SECURITY DEFINER 함수로 실행되어, 행위가
  무엇을 쓸 수 있는지는 `run_as` 의 권한이 정해야 한다.
- **FR-013**: 행위 실행 권한은 역할 단위로 부여·회수할 수 있어야 하며(그 함수에 대한
  EXECUTE), 실행기는 호출자가 직접 부를 수 없어야 한다.
- **FR-014**: 행위는 파라미터 계약(이름 → `string|int|float|number|bool|list|map|any`)을
  가지며, 선언되지 않은 키·누락·타입 불일치는 22023 으로 거부되어야 한다.
- **FR-015**: 행위는 전제 조건(파라미터를 받는 Cypher, `ok` 불리언 반환)을 가질 수 있고,
  거짓이면 `OG_ACTION_REJECTED`(23514)로 거부되어야 한다.
- **FR-016**: 행위를 통한 쓰기의 감사 행에는 행위 이름이 남아야 한다. 행위가 오류로 끝나도
  그 이름이 이후 질의의 감사 행에 남아서는 안 된다.

**수정된 결함 (선행 스펙)**

- **FR-017**: 쓰기 역할이 선언되지 않은 속성을 쓰면 `__ext` 에 남아야 한다(ADR-025 가
  서술했으나 동작하지 않던 것).
- **FR-018**: `og_data` 에 CREATE 권한이 없는 역할도 아직 만들어지지 않은 생성 뷰를 통해
  읽을 수 있어야 한다.

### Key Entities

- **Write Grant**: `og_catalog.write_grant(role, type_id, op, props)` — 부여 의도. 실제 권한은
  PG GRANT 이며, 이 표는 새 저장소에 재적용하기 위한 기록이다.
- **Stamped Type**: `og_catalog.stamped`.
- **Action**: `og_catalog.action` 한 행 + `og_action.og_action_<id>(jsonb)` 함수.
- **Audit Entry**: `og_data.og_audit` 확장 컬럼 `db_user`, `actor`, `source`, `reason`,
  `request_id`, `schema_version`, `action`.

## Success Criteria *(mandatory)*

- **SC-001**: 07_write_security.sql 과 10_evolution_temporal_coverage.sql 의 014 구간의 모든
  단언이 통과한다(현재 통과).
- **SC-002**: 같은 권한 규칙이 Cypher 와 원시 SQL 에 동일하게 적용됨을 테스트가 보인다
  (현재 통과: UPDATE/DELETE/레지스트리 INSERT).
- **SC-003**: 읽기 권한 + 행위 EXECUTE 만 가진 에이전트가 행위 외의 쓰기를 할 수 없다(현재 통과).
- **SC-004**: 공식 Neo4j 드라이버의 `tx_metadata` 가 행 스탬프와 감사 로그에 닿는다
  (`tests/bolt-context/run.py`, 현재 통과).

## Assumptions

- 기본 거부와 `og_grant` 단계는 [ADR-025](../../docs/99_decisions/ADR-025-privilege-model-default-deny.md).
  `og_grant_write`, `og_enable_stamping`, `og_define_action`, `og_grant_action` 등 설정 함수는
  admin 전용이다. `og_run_action`, `og_meta` 는 read 이다.
- 역할은 DBA 가 만든다. 이 스펙의 어떤 함수도 역할을 만들지 않는다.
- Bolt 인증은 [ADR-018](../../docs/99_decisions/ADR-018-bolt-auth-uses-postgres-roles.md)
  대로 PostgreSQL 역할이다.
- 스펙 **013** 의 이력 테이블과 캡처 트리거가 같은 SECURITY DEFINER 분리를 쓴다.

## Out of Scope (구현되지 않음)

- **읽기 제한**: 레지스트리의 읽기 정책은 `USING (true)` 다. 이 스펙은 쓰기만 다룬다. 타입별
  읽기 제한은 여전히 005 의 `og_enable_rls`(저장 테이블) 뿐이며, 레지스트리·인접 세그먼트는
  모든 읽기 역할이 읽는다([SEC-08](../../docs/07_security/10_fixed.md)).
- **`MERGE` 권한**: Neo4j 처럼 별도 권한이 아니다. `MERGE` 가 실제로 수행하는 INSERT/UPDATE
  에 대한 테이블·컬럼 권한으로 판정된다. `MERGE` 경로에 대한 전용 테스트는 없다.
- **TypeQL 행위의 파라미터**: 계약 검사는 하지만 본문에 값이 전달되지 않는다. TypeQL 행위는
  사실상 파라미터 없는 고정 문장이다.
- 행위 실행 이력의 별도 표(감사 로그의 `action` 컬럼이 전부다), 행위의 버전 관리.
