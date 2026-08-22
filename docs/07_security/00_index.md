# 07_security — 보안

> ⚠️ **이 문서는 감사 커밋 `7d60c82` 시점의 스냅샷이다.** 이후 Critical 5건과
> High 8건(4건 수정 · 4건 부분)이 반영되었으므로, 여기 서술된 결함 중 일부는 **현재 코드에 더 이상
> 존재하지 않는다.** 현재 상태는 [10_fixed.md](10_fixed.md) 를 볼 것.


> **이 문서가 답하는 질문**
> - 이 카테고리에는 무엇이 들어 있고, 어떤 순서로 읽어야 하는가?
> - 이 시스템의 보안 표면은 왜 일반적인 웹 애플리케이션과 다른가?
> - 지금 이 저장소의 보안 상태를 한 문장으로 말하면?

---

## 이 카테고리의 역할

`07_security/`는 **실제 소스 코드를 읽고 수행한 보안 감사 결과**를 담는다.
일반론이나 체크리스트 복사가 아니라, 이 저장소의 특정 파일·특정 줄을 근거로 한
사실 기록이다. 모든 주장에는 `파일:줄` 근거가 붙어 있고, 확인하지 못한 것은
"미확인"이라고 적혀 있다.

이 카테고리는 두 종류의 독자를 상정한다.

| 독자 | 먼저 읽을 문서 |
|---|---|
| 이 시스템을 **배포**하려는 사람 | [`08_secure_deployment.md`](08_secure_deployment.md) → [`06_network_exposure.md`](06_network_exposure.md) |
| 이 시스템을 **수정**하려는 사람 | [`04_injection_surface.md`](04_injection_surface.md) → [`05_process_safety.md`](05_process_safety.md) |
| 이 시스템을 **평가**하려는 사람 | [`01_threat_model.md`](01_threat_model.md) → [`09_improvements_security.md`](09_improvements_security.md) |
| **LLM 에이전트** | [`10_fixed.md`](10_fixed.md) 로 현재 상태를 먼저 확인한 뒤 [`09_improvements_security.md`](09_improvements_security.md) 의 표를 볼 것 |

---

## 이 프로젝트의 보안 표면이 특수한 이유

일반적인 애플리케이션 보안 모델은 "애플리케이션 프로세스가 죽어도 DB는 산다"를
전제한다. 여기서는 그 전제가 성립하지 않는다.

1. **확장이 DB 서버 프로세스 안에서 돈다.**
   `engine/`은 pgrx 0.19.2로 빌드된 cdylib이고, PostgreSQL 백엔드 프로세스에
   `dlopen` 되어 그 프로세스의 주소 공간에서 실행된다. 여기서의 세그멘테이션 폴트는
   백엔드 하나가 아니라 **클러스터 전체의 크래시 복구**를 유발한다
   ([`05_process_safety.md`](05_process_safety.md)).

2. **질의 언어 2종이 SQL로 컴파일된다.**
   주입(injection) 방어선이 웹 프레임워크가 아니라 `engine/src/cypher/compile.rs`
   (1,591줄)와 `engine/src/typeql/compile.rs`(817줄) 안에 있다. 방어가 되어 있는
   지점과 되어 있지 않은 지점을 코드 줄 단위로 구분해야 한다
   ([`04_injection_surface.md`](04_injection_surface.md)).

3. **인증이 위임된다.**
   자체 사용자 저장소가 없다. Bolt 게이트웨이는 HELLO의 자격 증명을 그대로
   PostgreSQL 접속에 쓴다(`bolt/src/session.rs:168-193`). 이는 설계상 옳지만,
   그 위임 경로가 평문이면 위임 자체가 무의미해진다
   ([`02_authn_authz.md`](02_authn_authz.md), [`06_network_exposure.md`](06_network_exposure.md)).

4. **RLS가 격리 수단으로 광고된다.**
   `docs/architecture.md:264`, `docs/comparison.md:180`은 "행 수준 보안이 순회
   중간에도 적용된다"고 명시한다. 이 주장이 코드에서 실제로 성립하는지는
   [`03_rls_and_isolation.md`](03_rls_and_isolation.md)에서 검증했다. **결론은
   "부분적으로만 성립"이다.**

---

## 문서 목록

| 문서 | 다루는 것 |
|---|---|
| [`01_threat_model.md`](01_threat_model.md) | 신뢰 경계, 자산, 공격자 모델, 진입점별 신뢰 수준 |
| [`02_authn_authz.md`](02_authn_authz.md) | PostgreSQL 역할 위임, Bolt 인증, Studio 인증 부재 |
| [`03_rls_and_isolation.md`](03_rls_and_isolation.md) | `og_enable_rls`가 실제로 만드는 것, RLS가 닿지 않는 경로 |
| [`04_injection_surface.md`](04_injection_surface.md) | 값 바인딩·식별자 인용의 코드 근거, 방어가 없는 지점 |
| [`05_process_safety.md`](05_process_safety.md) | 백엔드 종료 경로, 메모리 컨텍스트, SPI 재진입, panic |
| [`06_network_exposure.md`](06_network_exposure.md) | Bolt 포트/TLS, genai 아웃바운드, SSRF, 비밀 관리 |
| [`07_audit_and_history.md`](07_audit_and_history.md) | 감사 로그, 히스토리 캡처, 시점 조회의 보안적 한계 |
| [`08_secure_deployment.md`](08_secure_deployment.md) | 복사-붙여넣기 가능한 안전 배포 설정 |
| [`09_improvements_security.md`](09_improvements_security.md) | **SEC-01 ~ SEC-33 개선 포인트 (감사 시점 단일 진실 원천)** |
| [`10_fixed.md`](10_fixed.md) | **실제로 수정된 것과 남은 것 — 현재 코드 상태** |

---

## 감사 범위와 방법 (사실)

- **감사 대상 커밋**: `7d60c82` (main), 2026-08-22 기준.
- **방법**: 수동 소스 리뷰(SAST 성격). 동적 테스트(DAST)·실제 익스플로잇 실행은
  수행하지 않았다. 따라서 "재현 조건"은 코드 경로 서술이며, 실측 PoC가 아니다.
- **감사한 파일**: `engine/src/` 전체 32개 파일, `engine/sql/bootstrap.sql`,
  `engine/sql/access.sql`, `bolt/src/` 3개 파일, `portal/server/index.js`,
  `start.sh`, `docker/Dockerfile.dev`, `engine/ontological.control`,
  `engine/Cargo.toml`, `bolt/Cargo.toml`, `portal/package.json`.
- **감사하지 않은 것 (미확인)**:
  - `portal/web/app.js`(880줄) 프론트엔드 XSS 표면 — `04_frontend` 담당 범위와
    겹쳐 본 감사에서는 제외했다.
  - 의존성 CVE 스캔(`cargo audit` / `npm audit`) — 실행하지 않았다.
  - pgrx 0.19.2 내부의 panic↔ereport 변환 세부 동작 — 문서화된 계약만 참조했다.
  - 실제 배포 환경의 `pg_hba.conf` / `postgresql.conf` — 저장소에 없다.

---

## 한 문장 요약 (사실)

**질의 컴파일러의 값 바인딩은 견고하다. 그 바깥이 문제다.** — 사용자 값은
jsonb 파라미터 `$1` 하나로만 바인딩되어(`engine/src/cypher/compile.rs:18`,
`engine/src/cypher/mod.rs:148`) spec 003 FR-026의 주장이 코드에서 성립한다.
반면 Studio 백엔드의 무인증 임의 SQL 엔드포인트(`portal/server/index.js:296-308`),
Bolt PackStream의 인증 전 자원 고갈 경로(`bolt/src/packstream.rs:224-225`),
그리고 생성 뷰의 `security_invoker` 누락으로 인한 RLS 우회
(`engine/src/cypher/views.rs:135`)는 개별적으로 배포를 막을 만한 결함이다.

---

## Forbidden (금지)

- **이 카테고리의 문서에 "일반적으로 권장되는" 조언을 추가하지 말 것.**
  이 저장소의 코드 줄에 근거하지 않은 항목은 여기 들어오지 않는다.
- **취약점을 "수정했다"고 쓰기 전에 코드를 확인하지 말 것 — 반대로,
  코드를 확인하기 전에는 아무것도 쓰지 말 것.**
- **`09_improvements_security.md`의 SEC 번호를 재사용하지 말 것.**
  수정된 항목은 삭제하지 말고 상태를 `fixed (커밋해시)`로 바꿀 것.
- 실제 익스플로잇 페이로드를 이 문서군에 적지 말 것. 재현 조건 서술로 충분하다.

## Required (필수)

- 새 `#[pg_extern]` 함수를 추가하면 [`04_injection_surface.md`](04_injection_surface.md)의
  분류표에 한 줄을 추가할 것 (값 바인딩 / 식별자 인용 / 원시 SQL 중 어디인지).
- `Spi::run(&format!(...))` 를 새로 쓰면 그 줄을
  [`04_injection_surface.md`](04_injection_surface.md) 의 동적 SQL 목록에 등록할 것.
- 네트워크 포트를 새로 여는 코드를 추가하면 [`06_network_exposure.md`](06_network_exposure.md)와
  [`08_secure_deployment.md`](08_secure_deployment.md)를 같은 PR에서 갱신할 것.
- RLS 관련 코드를 수정하면 [`03_rls_and_isolation.md`](03_rls_and_isolation.md)의
  "RLS가 적용되지 않는 경로" 표를 재검증할 것.

<!-- affects: security, backend, api, ops -->
<!-- requires-update: 07_security/09_improvements_security.md -->
