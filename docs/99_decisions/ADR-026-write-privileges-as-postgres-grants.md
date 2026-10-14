# ADR-026: 세분화된 쓰기 권한은 PostgreSQL GRANT 와 레지스트리 RLS 로, 행위는 행위마다의 SECURITY DEFINER 함수로 구현한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-09-30 (작성일. 커밋 전) |
| 영향 범위 | security, storage, api, bolt, audit |
| 근거 | `engine/src/security/mod.rs`, `engine/src/security/action.rs`, `engine/sql/access.sql`(Write-time security 절, `og__stamp`, `og__audit`), `engine/sql/bootstrap.sql`(`og_catalog.write_grant`, `og_catalog.action`, 스키마 `og_action`), `engine/tests/sql/07_write_security.sql`, `engine/tests/sql/10_evolution_temporal_coverage.sql`(014 구간), [spec 014](../../specs/014-write-security-context/plan.md) |

> **이 문서가 답하는 질문**
> - "이 역할은 Person 을 만들 수 있지만 salary 는 못 쓴다"를 어디서 판정하는가?
> - 왜 엔진이 Cypher 를 보고 검사하지 않는가?
> - 에이전트에게 "행위 하나만" 주는 것은 어떻게 성립하는가?

## 배경

[ADR-025](ADR-025-privilege-model-default-deny.md) 는 `read ⊂ write ⊂ admin` 을 만들었다.
`write` 는 `og_data` 전체에 대한 DML 이다. 에이전트에게 쓰기를 주려면 그보다 잘게 쪼개야
했다: Neo4j 의 `CREATE ON GRAPH g NODES L`, `SET PROPERTY {p} ON … NODES L`, `DELETE ON …`
수준, 그리고 Palantir 의 Action — "쓸 수 있는 행위" 자체를 정의하고 거기에 제출 조건을 붙이는
모델([spec 014](../../specs/014-write-security-context/spec.md) 입력).

## 고려한 선택지

1. **엔진 쪽 검사.** 컴파일러가 Cypher 의 `CREATE`/`SET`/`DELETE` 를 보고 역할별 정책 표와
   대조한다. 오류 메시지를 마음대로 만들 수 있다. 그러나 (a) 파서·컴파일러의 모든 쓰기 경로
   (`MERGE`, `SET n += map`, 프로시저, TypeQL `insert`/`update`/`delete`, `og_create_node`
   등 SQL 함수)가 빠짐없이 검사를 거쳐야 하고, 하나라도 놓치면 조용한 우회가 된다. (b) 원시
   SQL 은 엔진을 거치지 않는다 — `write` 를 가진 역할은 `UPDATE og_data.n_5 SET p_salary …`
   로 끝이다.
2. **PostgreSQL 테이블·컬럼 GRANT.** [ADR-005](ADR-005-typed-property-columns.md) 로 타입은
   테이블, 속성은 컬럼이다. Neo4j 의 세 권한이 `INSERT`/`UPDATE (cols)`/`DELETE` 로 1:1
   대응한다. 어떤 경로로 쓰든 PostgreSQL 이 판정한다. 문제는 공유 레지스트리 —
   `og_node`, `og_edge`, `og_adj`, `og_role_player` 는 모든 생성·삭제가 쓰므로, 여기의 테이블
   GRANT 는 "모든 타입에 대한 쓰기"와 같다.
3. **2안 + 레지스트리 RLS.** 레지스트리 행은 그 행의 타입 저장 테이블에 해당 권한이 있는
   역할만 쓸 수 있게 한다.

행위(Action)에 대해서는:

- **A. 하나의 실행 함수 + 행위별 허용 표.** `og_run_action` 이 표를 보고 허용 여부를 판단한 뒤
  높은 권한으로 본문을 실행한다. 판정이 우리 코드에 있다.
- **B. 행위마다 함수 하나.** `og_action.og_action_<id>(jsonb)`, SECURITY DEFINER, 소유자 =
  `run_as`. "실행할 수 있나"는 그 함수에 대한 EXECUTE, "무엇을 쓸 수 있나"는 `run_as` 의 GRANT.

## 결정

**3안 + B.**

- `og_grant_write(role, graph, type, ops, props)` 는 타입과 하위 타입의 저장 테이블에
  `GRANT INSERT (id[, src, dst], cols…)`, `GRANT UPDATE (cols…)`, `GRANT DELETE` 를 발행한다.
  `props` 가 NULL 이면 테이블 수준 GRANT(미선언 속성 `__ext` 포함)다.
- 의도는 `og_catalog.write_grant` 에 기록되고, 저장 테이블 생성·컬럼 추가·재부모화·스키마
  변경(스펙 012) 때 `replay_write_grants` 가 다시 적용한다 — ADR-025 의 `grantee` 와 같은 이유
  (`ALTER DEFAULT PRIVILEGES` 는 생성자 기준이라 쓸 수 없다).
- 레지스트리 네 개에 RLS 를 켜고, 쓰기 정책을 `og_type_priv(type_id, priv)` 로 건다.
  읽기 정책은 `USING (true)`. `FORCE` 하지 않으므로 소유자와 슈퍼유저는 정책을 거치지 않는다.
- 엔진의 `check_write` 는 PostgreSQL 이 거부하기 전에 `OG_WRITE_DENIED: role 'r' may not set
  properties on 'Person' (property 'salary')` 를 낸다. **판정의 원천이 아니라 메시지다.**
  이 검사가 틀려도(누락되어도) 결과는 PostgreSQL 의 권한 오류일 뿐 우회가 아니다.
- 행위 함수는 `run_as` 소유 SECURITY DEFINER 이고 본문은 `og__action_exec(id, params)`
  호출 하나다. 실행기의 EXECUTE 는 행위 소유 역할에게만 부여된다.

## 근거

- **안전이 파싱 정확도에 의존하지 않는다.** ADR-025 가 `og_cypher` 를 READ 목록에 둔 것과
  같은 논리다 — 경계는 `EXECUTE` 가 아니라 테이블 권한이다
  (`engine/src/catalog/privileges.rs` 모듈 주석).
- **원시 SQL 에도 성립한다.** 07 은 clerk 역할의 원시 `UPDATE … SET p_salary`, `DELETE`,
  다른 타입의 `og_node` INSERT 가 모두 거부됨을 단언한다. 10 은 관계 부여가 없는 역할의
  `og_edge`·`og_adj` 원시 INSERT 가 RLS 로 거부됨을 단언한다.
- **스탬프도 같은 논리다.** `og__stamp()` 는 SECURITY INVOKER 트리거로 `current_user`,
  `og.actor`, `og.source` 를 찍고 행이 가져온 값은 덮어쓴다. 07 은 테이블 전체 UPDATE 권한을
  가진 역할의 원시 SQL 위조가 되돌려짐을 단언한다.
- **B 가 A 보다 작다.** A 는 "허용 표를 보고 높은 권한으로 실행"하는 코드가 권한 상승 지점이
  된다. B 에서 권한 상승은 PostgreSQL 의 SECURITY DEFINER 한 곳이고, 그 범위는 `run_as` 가
  가진 GRANT 로 이미 제한되어 있다. 에이전트는 읽기 권한 + 행위 EXECUTE 만 가지면 다른 쓰기
  경로가 없다(07: 에이전트 자신의 `CREATE` 는 42501).

## 결과

**긍정적**

- 새 권한 시스템이 없다. `\dp`, `has_column_privilege`, `pg_dump` 가 그대로 권한을 보여 준다.
- 쓰기 권한은 Cypher, TypeQL, SQL 함수, 원시 SQL, Bolt 에서 동일하다.
- 레지스트리가 처음으로 RLS 를 가졌다([SEC-08](../07_security/10_fixed.md) 의 쓰기 절반).

**부정적 / 감수한 대가**

- **읽기는 여전히 열려 있다.** 레지스트리 읽기 정책은 `true` 다. ADR-025 가 걱정한 순회 계획
  변화를 피한 대가로, "어떤 id 가 존재하는가"는 읽기 역할 모두에게 보인다.
- **RLS 정책 비용은 비소유자 쓰기에서만 발생하며 측정이 부족하다.** 구현 시 측정은 디버그
  빌드·소유자 역할 기준이다([spec 014 plan](../../specs/014-write-security-context/plan.md)).
- **오류 메시지와 실제 판정이 두 곳에 있다.** `check_write` 가 PostgreSQL 보다 관대하면
  메시지 대신 PostgreSQL 의 원문 권한 오류가 나온다. 그 반대(더 엄격)는 허용된 쓰기를 막는
  버그가 된다.
- **`MERGE` 는 별도 권한이 아니다.** 실제로 수행된 INSERT/UPDATE 로 판정된다.
- **행위의 감사 `db_user` 는 `run_as` 다.** 호출자는 `principal`(session_user)로만 남는다.
- **`og_define_action` 호출자는 `run_as` 로 `SET ROLE` 할 수 있어야 한다**(함수 소유권 이전
  조건). 행위 소유 역할에게는 `og_action` 스키마의 CREATE 가 부여된다.

## 재검토 조건

- 레지스트리 읽기 제한이 필요해질 때 — 그때는 순회 계획 변화를 벤치마크와 함께 다룬다
  (ADR-025 결과 절, 헌법 원칙 X).
- 컬럼이 아닌 곳에 속성이 저장되는 경우(예: `__ext` 안의 개별 키에 대한 권한)가 요구될 때 —
  컬럼 GRANT 로는 표현할 수 없다.
- 레지스트리를 없애거나(TableAM, [ADR-002](ADR-002-no-table-access-method.md)) 구조가
  바뀌면 RLS 정책 네 개를 다시 설계해야 한다.

<!-- affects: security, storage, api, bolt -->
