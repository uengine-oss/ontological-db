# Implementation Plan: 쓰기 시점 보안 컨텍스트

**Branch**: `014-write-security-context` | **Date**: 2026-09-30 | **Spec**: [spec.md](spec.md)

## Summary

세 질문에 PostgreSQL 이 이미 가진 세 메커니즘으로 답한다. 엔진은 규칙을 **강제하지 않는다**
— 강제는 PostgreSQL 이 하고, 엔진은 그 전에 읽기 좋은 오류를 낸다.

| 질문 | 메커니즘 | 원시 SQL 에도 성립? |
|---|---|---|
| 이 역할은 무엇을 쓸 수 있나? | 타입별 저장 테이블의 테이블/컬럼 GRANT + 레지스트리 RLS | 예 |
| 누가, 누구를 대신해, 어디서 썼나? | BEFORE 트리거가 세션에서 찍는 `__*` 컬럼 | 예 |
| 어떤 쓰기만 허용되나? | 행위마다 `run_as` 소유 SECURITY DEFINER 함수, 호출자에겐 EXECUTE 만 | 예(다른 쓰기 경로가 없으므로) |

**핵심 설계 결정** — 근거와 대가는 [ADR-026](../../docs/99_decisions/ADR-026-write-privileges-as-postgres-grants.md)

1. **타입은 테이블, 속성은 컬럼이다.** [ADR-005](../../docs/99_decisions/ADR-005-typed-property-columns.md)
   덕분에 Neo4j 의 `CREATE ON … NODES L` 은 `INSERT ON n_L`, `SET PROPERTY {p}` 는
   `UPDATE (p_p) ON n_L`, `DELETE ON … NODES L` 은 `DELETE ON n_L` 로 1:1 대응한다. 새 권한
   시스템을 만들지 않는다.
2. **공유 레지스트리는 RLS 로 닫는다.** `og_node`/`og_edge`/`og_adj`/`og_role_player` 는
   모든 생성·삭제가 쓰므로 테이블 GRANT 는 "모든 타입에 대한 권한"이 된다. 정책
   `og_type_priv(type_id, priv)` 가 "이 행의 타입 저장 테이블에 그 권한이 있는가"를 묻는다.
   소유자·슈퍼유저는 정책을 우회하므로(FORCE 하지 않음) 벤치마크와 관리 경로는 비용이 없고,
   읽기 정책은 `true` 라 플래너가 버린다.
3. **부여는 기록하고 재적용한다.** 저장 테이블·컬럼은 런타임에 생긴다. `og_catalog.write_grant`
   에 의도를 두고 `replay_write_grants(type)` 를 테이블 생성, 컬럼 추가, 재부모화, 012 의 확장·
   축소 시점에 부른다. ADR-025 의 `grantee` 와 같은 방식이다.
4. **행위는 함수 하나씩이다.** `og_action.og_action_<id>(jsonb)` 는 `run_as` 소유 SECURITY
   DEFINER 이고, 본문은 `og__action_exec(id, params)` 호출 하나다. "이 호출자가 실행할 수
   있나"는 PostgreSQL 의 EXECUTE 검사, "행위가 무엇을 쓸 수 있나"는 `run_as` 의 GRANT 가
   답한다. 실행기 `og__action_exec` 의 EXECUTE 는 행위 소유 역할에게만 있다.
5. **스탬프는 SECURITY INVOKER 트리거.** `current_user` 가 문장을 실행한 역할이어야 하므로.
   값은 세션에서만 오고 행의 값은 덮어쓴다.
6. **감사는 SECURITY DEFINER 한 곳으로.** 모든 감사 행은 `og__audit` 를 거친다. 호출자가
   넘긴 `current_user` 는 `session_user` 가 그 역할의 멤버일 때만 받아들이고, 아니면
   `session_user` 로 대체한다.

## Architecture

| 함수 / 객체 | 위치 | 스펙 |
|------|------|------|
| `og_grant_write(role, graph, type, ops text[], props text[] = NULL)` | `engine/src/security/mod.rs` | FR-001..FR-003 |
| `og_revoke_write(role, graph, type, ops = NULL)` | 같음 | FR-001 |
| `og_catalog.write_grant`, `replay_write_grants` | `bootstrap.sql`, `security/mod.rs` | FR-003 |
| `og_type_priv(type_id, priv)` + 레지스트리 RLS 정책 | `access.sql` | FR-004 |
| `check_write` → `OG_WRITE_DENIED` | `security/mod.rs`, `storage/mod.rs` 호출 6곳 | FR-005 |
| `check_reserved_keys` → `OG_RESERVED_PROPERTY` | `security/mod.rs`, `storage::plan_props` | FR-006 |
| `og_enable_stamping(graph, type)`, `og__stamp()` 트리거 `og_a_stamp` | `security/mod.rs`, `access.sql` | FR-007, FR-008 |
| `og_meta(id)` | `access.sql` | FR-009 |
| `og__audit(...)` + `og_audit` 확장 컬럼, `protect_records` | `access.sql`, `bootstrap.sql`, `catalog/privileges.rs` | FR-010 |
| Bolt `tx_metadata` → `set_config(…, true)` | `bolt/src/session.rs` | FR-010 |
| `og_define_action`, `og_grant_action`, `og_revoke_action`, `og_drop_action`, `og_run_action`, `og__action_exec` | `engine/src/security/action.rs`, 스키마 `og_action` | FR-011..FR-016 |
| `can_change_schema()` — 스키마 권한 없으면 `__ext` 로 | `security/mod.rs`, `storage::declare_new_props` | FR-017 |
| `og__ensure_view` (SECURITY DEFINER) | `engine/src/cypher/views.rs` | FR-018 |

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| **I. 포크가 아닌 확장** | ✅ | GRANT, RLS, SECURITY DEFINER, 트리거, 커스텀 GUC — 전부 표준 PG |
| II. Cypher 1급 | ✅ | Cypher 무변경. 권한은 컴파일된 SQL 이 만나는 테이블에서 판정 |
| III. 저장구조 | ✅ | 레지스트리 RLS 는 소유자에게 적용되지 않고, 읽기 정책은 `true`. 순회 계획은 읽기 역할에서도 바뀌지 않는다 — 아래 성능 참고 |
| IV. 온톨로지 우선 타이핑 | ✅ | 부여가 계층을 따라 내려간다 |
| V. 벡터 | ✅ | 해당 없음 |
| VI. 코어는 하나 | ✅ | Cypher·TypeQL·원시 SQL·Bolt 가 같은 테이블 권한을 만난다 |
| VII. 확장성 | ✅ | 해당 없음 |
| **VIII. 에이전트 네이티브** | ✅ | 에이전트에게 "행위만" 줄 수 있다. 거부 오류가 역할·연산·타입·속성과 부여 방법을 말한다 |
| **IX. ACID** | ✅ | 스탬프·감사·이력이 같은 트랜잭션 |
| X. 벤치마크로 증명 | ⚠️ | 아래 성능 — 저장소 하네스에 포함된 측정이 아니다 |

**품질 게이트 "새 SQL 노출 API 에 대한 RLS/권한 테스트 부재 → 차단"**: 07_write_security.sql
이 그 테스트다. 비소유자 역할 다섯 개(`og_t_reader`, `og_t_writer`, `og_t_clerk`, `og_t_agent`,
`og_t_runner`)로 실행하며, 이 과정에서 선행 결함 네 개가 드러나 수정되었다
([10_fixed.md](../../docs/07_security/10_fixed.md) 6절).

**성능.** 구현 시 디버그 빌드에서 측정(노드 5000 생성, 간선 4999, Cypher CREATE 1000, SET
1000, 1-hop 읽기 500)한 결과 변경 전 빌드와 같은 범위였다고 기록되어 있다(예: 노드 생성
673–703 ms 대 705–765 ms). 이 측정은 스키마 권한 검사(`can_change_schema`)를 "할 일이 있을
때만" 하도록 바꾼 **뒤**의 수치다. 측정 스크립트는 `bench/` 하네스에 들어 있지 않고, 릴리스
빌드·비소유자 역할(RLS 정책이 실제로 평가되는 경우)로는 측정하지 않았다.

## Phasing

| Phase | 내용 | 상태 |
|-------|------|------|
| P0 | 비소유자 역할로 기존 경로 실행 → 선행 결함 4건 수정 | 완료 |
| P1 | `og_grant_write` + 레지스트리 RLS + 친절한 오류 + 예약 이름 | 완료 |
| P2 | 스탬프, `og_meta`, 감사 확장, 감사·이력 보호 | 완료 |
| P3 | 행위 | 완료(TypeQL 파라미터 제외) |
| P4 | Bolt `tx_metadata` | 완료 |
| P5 | `og.reason` 기록 | 완료 |
| P6 | 읽기 제한(레지스트리) | 미착수 |

## Complexity Tracking

**레지스트리 읽기는 여전히 열려 있다(SEC-08 의 절반).** 이 스펙은 레지스트리에 RLS 를
걸었지만 쓰기 정책만이다. 읽기 정책 `USING (true)` 는 ADR-025 가 걱정한 "모든 순회 계획이
바뀐다"를 피하기 위한 선택이다. 타입별 읽기 제한이 필요하면 005 의 `og_enable_rls` 로 저장
테이블을 막아야 하고, 레지스트리와 인접 세그먼트로는 "어떤 id 가 존재하는가"가 여전히 보인다.

**행위 실행기의 스레드 로컬 상태는 drop guard 로 복원한다(수정됨, 07 이 단언).**
`og__action_exec` 는 감사용으로 `CURRENT_ACTION` 을 설정한다. 처음에는 본문이 끝난 뒤 복원
코드로 되돌렸기 때문에, 본문이 오류로 끝나면(권한 거부, 제약 위반 등) 같은 백엔드의 다음
질의들의 감사 행에 그 행위 이름이 남았다. 지금은 `ActionGuard` 의 `Drop` 이 되돌린다.
PostgreSQL 오류는 Rust 패닉으로 이 함수를 빠져나가고 unwind 중에 `Drop` 이 실행되므로, 정상
종료·전제 조건 실패·본문 오류 모두에서 복원된다(`engine/src/security/action.rs`).

**행위의 감사 행 `db_user` 는 호출자가 아니라 `run_as` 다.** SECURITY DEFINER 안의
`current_user` 이기 때문이다(07 이 이것을 단언한다). 호출자는 `principal`(= `session_user`)
로만 남으므로, 호출자가 `SET ROLE` 로 바꾼 신원은 행위 감사 행에 남지 않는다.

**`og__audit` 는 읽기 역할이 직접 부를 수 있다.** 모든 질의가 감사 행을 남겨야 하므로 READ
목록에 있다. 그 결과 호출자는 질의 텍스트·행 수·`action` 값을 임의로 넣은 감사 행을 **추가**
할 수 있다. `db_user` 는 검증되고, 기존 행을 고치거나 지울 수는 없다. 감사 로그는 "추가 전용"
이지 "내용 보증"은 아니다.

**`og.reason` 은 감사 로그에만 기록된다.** Bolt 가 `tx_metadata.reason` 을 `og.reason` 으로
설정하고, `og__audit` 가 이를 `og_data.og_audit.reason` 에 쓴다(07 이 단언). 스탬프에는 이유
컬럼이 없다 — 이유는 행이 아니라 질의에 붙는 것이기 때문이다.

**TypeQL 행위 본문은 파라미터를 받지 않는다.** `typeql::run_action_body(graph, body)` 는
파라미터 인자가 없다. 계약 검사는 통과해야 하지만 값은 쓰이지 않는다.

**감사의 `schema_version` 은 전역 시퀀스 값이다.** `og_catalog.schema_version_seq` 의
`last_value` 이며, 질의한 그래프의 버전이 아니라 데이터베이스 전체에서 마지막으로 발급된
버전 번호다.

**`og_type_priv` 는 저장 테이블이 이미 없으면 참이다.** `og_drop_type` 이 저장 테이블을 먼저
지우고 레지스트리 행을 나중에 지우기 때문이다. 레지스트리에 DELETE 를 가진 역할이면 그런
고아 행을 지울 수 있다.
