# ADR-018: Bolt 인증에 PostgreSQL role을 그대로 쓰고, 두 번째 사용자 저장소를 만들지 않는다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/011-bolt-protocol-gateway/plan.md` 기준일) |
| 영향 범위 | bolt, security, api |
| 근거 | `specs/011-bolt-protocol-gateway/plan.md` 설계 결정 2, `bolt/README.md` "How the two worlds line up", `specs/003-cypher-query-engine/spec.md` FR-024a |

> **이 문서가 답하는 질문**
> - Bolt로 접속할 때 쓰는 사용자/비밀번호는 무엇인가?
> - 게이트웨이를 통과하면 RLS가 우회되는가?

## 배경

Bolt의 `HELLO` 메시지는 사용자명과 비밀번호를 실어 온다. 게이트웨이가 이를 어떻게 처리할
것인가는 곧 **권한 모델이 하나인가 둘인가**의 문제다.

spec 003 FR-024a는 원래 Bolt를 비목표로 선언했는데, 그 조항이 지키려던 성질이 정확히
*"두 번째 인증·감사 경로를 만들지 않는다"* 였다. spec 011은 그 조항을 대체하되 성질은
유지한다고 명시했다.

## 고려한 선택지

1. **게이트웨이 자체 사용자 저장소** — Neo4j와 같은 사용자 관리 경험을 준다. 그러나
   권한·RLS·감사가 두 벌이 되고, 어느 쪽이 진실인지 모호해진다.
2. **서비스 계정 하나로 프록시** — 게이트웨이가 고정 계정으로 접속하고 애플리케이션 사용자를
   따로 관리. **RLS가 무의미해진다** — 모든 질의가 같은 role로 실행되기 때문이다.
3. **`HELLO`의 자격 증명으로 PostgreSQL에 접속** — 접속 실패가 곧 인증 실패.

## 결정

**3안.** `specs/011-.../plan.md` 설계 결정 2 원문:
> **인증 저장소를 만들지 않는다.** `HELLO`가 나르는 자격 증명으로 PostgreSQL에 접속한다.
> 접속에 실패하면 그게 인증 실패다. 권한·RLS·감사가 하나로 유지되며, 이것이 003 FR-024a가
> 지키려던 바로 그 성질이다.

대응 관계는 `bolt/README.md`가 표로 정리한다.

| Neo4j | 여기 |
|---|---|
| database (`session(database="x")`) | **graph** `x` |
| `HELLO`의 user / password | **PostgreSQL role**과 그 비밀번호 — 두 번째 사용자 저장소는 없다 |
| explicit transaction | 세션 연결 위의 PostgreSQL 트랜잭션 |

## 근거

- `bolt/README.md`가 보안 속성을 한 문장으로 못 박는다:
  > Permissions, RLS and audit stay PostgreSQL's. A role that cannot see a row over
  > psql cannot see it over Bolt — the gateway never connects as anyone but the
  > authenticated user.
- 이는 ADR-001(확장으로 구현)과 spec 005의 RLS 설계가 성립해야만 가능한 속성이다.
  컴파일된 Cypher가 참조하는 것이 일반 테이블이므로 RLS가 트래버설 중간 노드에도 적용된다
  (`specs/005-postgres-supabase-interop/plan.md` "RLS 적용 지점").
- 게이트웨이가 상태를 갖지 않는다는 ADR-017의 성질이 이 결정의 전제다 — 사용자 저장소는
  가장 큰 상태다.

## 결과

**긍정적**
- 권한·RLS·감사 경로가 하나뿐이다. Bolt가 우회로가 되지 않는다.
- 멀티테넌트 격리가 Bolt에서도 자동으로 성립한다.
- 사용자 관리 도구가 PostgreSQL 것 그대로다 (`CREATE ROLE`, `GRANT`, `pg_hba.conf`).

**부정적 / 감수한 대가**
- **Bolt 연결 하나가 PostgreSQL 연결 하나를 점유한다.** 연결 풀링이 게이트웨이 층에서
  불가능하다 — 사용자마다 다른 role로 접속해야 하기 때문이다. 동시성 한계가
  `max_connections`와 같아진다 (`plan.md` 설계 결정 4가 이를 명시적으로 인정).
- Neo4j의 사용자/롤 관리 API(`dbms.security.*`)는 제공되지 않는다.
- **TLS를 종단하지 않으므로 자격 증명이 평문으로 오간다.** 운영에서는 TLS 종단 프록시가
  필수 전제이며, 이는 `bolt/README.md` 지원 매트릭스에 명시된 한계다.
  이 ADR의 보안 주장은 **그 전제 위에서만** 성립한다.

## 재검토 조건

- **TLS 종단을 게이트웨이 안에서 지원하기로 하면** 이 ADR의 가장 큰 실무적 구멍이 닫힌다.
  현재는 프록시 전제로 미룬 상태다.
- 연결 수 한계가 실제 배포에서 병목이 되면, 같은 role의 연결을 공유하는 풀링을 재평가한다.
  단, "같은 role"이라는 조건을 깨는 순간 RLS 보장이 무너지므로 신중해야 한다.
- Bolt 5.x 지원 시 인증 확장(`LOGON`/`LOGOFF` 메시지)이 추가되면 이 매핑을 다시 그려야 한다.

<!-- affects: bolt, security, api -->
<!-- requires-update: docs/99_decisions/ADR-017-bolt-gateway-separate-process.md -->
