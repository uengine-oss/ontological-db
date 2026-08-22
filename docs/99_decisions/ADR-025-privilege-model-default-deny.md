# ADR-025: 함수 권한은 기본 거부하고, 역할은 확장이 만들지 않는다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-23 |
| 영향 범위 | security, api, data, operations |
| 근거 | `engine/sql/access.sql` 말미 `DO $$ … REVOKE ALL … FROM PUBLIC`, `engine/src/catalog/privileges.rs`, `engine/sql/bootstrap.sql` 의 `og_catalog.grantee`, [SEC-01~SEC-17](../07_security/09_improvements_security.md) |

> **이 문서가 답하는 질문**
> - 누가 무엇을 실행할 수 있는가?
> - 왜 확장이 롤을 만들어 주지 않는가?
> - 런타임에 생기는 저장 테이블의 권한은 어떻게 되는가?

## 배경

감사 시점(`7d60c82`)까지 `bootstrap.sql` 과 `access.sql` 어디에도 `GRANT` 나
`REVOKE` 가 한 줄도 없었다. 이건 "권한 설정을 깜빡했다"가 아니라 **두 방향으로
동시에 틀린 상태**였다.

PostgreSQL 은 새 함수에 PUBLIC EXECUTE 를 기본으로 준다. 아무도 그걸 되돌리지
않았으므로 클러스터의 모든 롤이 `og_set_setting`(임베딩 파이프라인이 텍스트를
보낼 엔드포인트를 고른다), `og_enable_rls`, `og_map_table`, `og_drop_graph` 를
호출할 수 있었다. 동시에 어떤 테이블에도 GRANT 가 없었으므로 같은 롤은 질의를
하나도 실행할 수 없었다.

닫혀 있어야 할 곳이 열려 있고, 열려 있어야 할 곳이 닫혀 있었다.

## 고려한 선택지

1. **확장이 `og_reader`/`og_writer`/`og_admin` 롤을 만든다.** 설치 후 바로 쓸 수
   있어 가장 친절하다. 그러나 롤은 클러스터 전역이고 `DROP EXTENSION` 이후에도
   남는다. `CREATE ROLE` 은 `superuser = false` 로 선언된 이 확장의 설치자가 못
   가질 수도 있는 권한이라, 설치 자체가 실패할 수 있다. 그리고 같은 클러스터에
   두 번째 데이터베이스에 설치하면 이름이 충돌한다.
2. **`ALTER DEFAULT PRIVILEGES` 로 미래 객체까지 처리한다.** 저장 테이블이
   런타임에 생기는 문제를 표준 기능으로 푸는 것처럼 보인다. 그러나 이 기능은
   **객체를 만든 롤** 을 기준으로 동작하는데, 저장 테이블은 `og_create_type` 을
   호출한 아무나가 만든다. 부여자와 생성자가 갈리는 순간 조용히 아무것도 하지
   않는다.
3. **기본 거부 + 명시적 부여 함수.** 확장은 PUBLIC 에서 EXECUTE 를 회수하는
   것까지만 하고, 롤 생성과 배분은 DBA 에게 남긴다.

## 결정

**3안.**

**기본 거부.** `access.sql` 은 `finalize` 단계라 확장의 모든 함수가 이미
카탈로그에 있다. 그 시점에 확장 스키마의 `og_*` 함수 전부를 돌며
`REVOKE ALL … FROM PUBLIC` 한다.

**되돌려 주는 통로.** `og_grant(role, level)` / `og_revoke(role)`. level 은
`read` ⊂ `write` ⊂ `admin`. 롤은 **이미 존재해야 하며 이 함수가 만들지 않는다.**

**부여를 기억한다.** `og_catalog.grantee` 에 `(role, level)` 을 기록하고,
테이블·뷰가 런타임에 생성될 때 `apply_to_table`/`apply_to_view` 가 다시 부여한다.
2안이 못 하는 일을 이 방식은 생성자와 무관하게 한다.

**함수 분류는 fail-closed.** `READ` 와 `WRITE` 만 열거하고 나머지는 admin 이다.
나중에 추가되는 함수는 누군가 목록에 넣기 전까지 admin 전용이며, 이게 실수가
나야 할 방향이다.

## 결과

**읽기 롤이 `og_cypher` 를 가진다.** Cypher 로 `CREATE` 와 `DELETE` 를 쓸 수
있는데도 그렇다. 컴파일된 질의는 평범한 테이블을 읽고 쓰므로, `og_data` 에
`SELECT` 만 가진 롤은 문장 자체에서 권한 오류를 받는다. **경계는 `EXECUTE` 가
아니라 테이블 권한이다.** 이 선택의 장점은 우리가 질의를 정확히 파싱하는 데
안전이 의존하지 않는다는 것이다 — [ADR-009](ADR-009-read-sql-write-rust-split.md)
가 읽기 경로를 SQL 로 남긴 이유와 같은 성질이다.

**쓰기 롤은 스키마를 바꾸지 못한다.** 선언되지 않은 프로퍼티를 쓰면 컬럼 승격
([ADR-006](ADR-006-write-time-property-promotion.md))이 권한 부족으로 실패한다.
의도한 선이지만 `declare_new_props` 가 실패를 `is_ok()` 로만 보기 때문에 SPI
오류가 트랜잭션을 중단시키는 경로가 남아 있다. 미해결로 기록한다.

**설치 직후에는 소유자만 쓸 수 있다.** 이건 감사 이전과 실질적으로 같은 상태다
— 테이블에 GRANT 가 없었으므로 그때도 소유자만 쓸 수 있었다. 달라진 건 이제
그 상태가 **의도된 것이고 되돌리는 방법이 있다**는 점이다.

**pg13/pg14 는 완전하지 않다.** 함수 권한은 동일하게 동작하지만
`security_invoker`([SEC-06](../07_security/10_fixed.md))가 PostgreSQL 15
기능이라, 그 두 버전에서는 생성 뷰를 통한 RLS 를 신뢰할 수 없다.

## 헌법 원칙과의 관계

원칙 III(하나의 질의 경로)와 충돌하지 않는다 — 경계를 함수가 아니라 테이블에
두었으므로 `og_cypher` 는 여전히 유일한 Cypher 진입점이다. 원칙 X(성능 주장은
벤치마크로 증명한다)는 이 결정이 [SEC-08](../07_security/10_fixed.md)을 남긴
이유이기도 하다: 레지스트리 테이블에 RLS 를 거는 것은 순회 계획을 바꾸므로
측정 없이 넣을 변경이 아니다.
