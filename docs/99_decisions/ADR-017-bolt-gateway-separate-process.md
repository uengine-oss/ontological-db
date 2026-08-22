# ADR-017: Bolt 게이트웨이를 확장 내부 배경 워커가 아니라 별도 프로세스로 둔다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/011-bolt-protocol-gateway/plan.md` 기준일) |
| 영향 범위 | bolt, api, ops, architecture |
| 근거 | `specs/011-bolt-protocol-gateway/plan.md` 설계 결정 1·4, `bolt/README.md`, `.specify/memory/constitution.md` 원칙 VI |

> **이 문서가 답하는 질문**
> - "확장 하나면 끝"이라고 해놓고 왜 별도 바이너리를 띄워야 하는가?
> - 게이트웨이가 Cypher를 해석하는가?

## 배경

Neo4j 드라이버가 URI만 바꿔 접속하게 하려면 Bolt 프로토콜을 종단해야 한다. PostgreSQL
확장 안에서 이를 하는 자연스러운 방법은 **배경 워커(background worker)** 다. 그러면
설치가 `CREATE EXTENSION` 하나로 끝난다.

## 고려한 선택지

1. **확장 내부 배경 워커** — 설치가 단순하다. 그러나 SPI는 스레드 안전하지 않다.
2. **별도 프로세스(평범한 PostgreSQL 클라이언트)** — 설치 단계가 하나 늘어난다.

## 결정

**2안.** `bolt/`는 pgrx가 아닌 평범한 Rust 바이너리(`ontological-bolt`)이며, 확장 ABI와
무관하다.

`specs/011-.../plan.md` 설계 결정 1 원문:
> 배경 워커(background worker)로 확장 안에 넣으면 "확장 하나면 끝"이라는 이야기에는
> 맞지만, SPI는 스레드 안전하지 않아 세션 간 질의가 직렬화된다 — 그 대가로 얻는 것이
> 설치 편의뿐이라면 잘못된 거래다. 게이트웨이는 연결마다 PostgreSQL 연결을 갖는 평범한
> 프로세스이고, 동시성은 PostgreSQL이 이미 해결한 방식으로 해결된다.
> 헌법 원칙 VI("코어는 하나, 표준은 어댑터로")의 프로토콜 판이다.

## 근거

- **게이트웨이는 의미론을 갖지 않는다.** `bolt/README.md`:
  > The gateway holds no state: no parser, no planner, no cache, no user store. Every
  > query it receives goes to `og_cypher()`, so Cypher semantics, compilation, error
  > messages and transactions are spec 003's — reached through a different transport,
  > not a second implementation.
- 동시성 모델도 같은 논리다 (`plan.md` 설계 결정 4): *"연결 하나에 스레드 하나. …
  비동기 런타임을 들이지 않는 이유이며, 동시성 한계는 PostgreSQL 연결 수 한계와 같아진다."*
- 프로토콜 크레이트를 쓰지 않고 직접 구현한 이유도 기록되어 있다 (`plan.md` Dependencies):
  *"Bolt 서버 크레이트를 쓰면 지원 매트릭스를 우리가 통제하지 못한다(FR-020)."*
- `EXPLAIN`조차 게이트웨이가 파싱하지 않는다 — `og_cypher_check()`에 물어본다
  (`bolt/README.md` 마지막 문단).
- 검증도 우리 클라이언트가 아니라 **공식 드라이버·공식 샘플**로 한다 (`plan.md` Testing):
  *"우리가 만든 클라이언트로 우리 서버를 테스트하는 것은 증거가 아니다."*
  README는 Neo4j 자체 MCP 서버가 무수정으로 동작함을 근거로 든다
  (`examples/meeting-rooms/`).

## 결과

**긍정적**
- SPI 직렬화 병목이 없다. 동시성이 PostgreSQL의 연결 모델과 동일해진다.
- 게이트웨이를 죽여도 `og_cypher()` 사용자는 영향받지 않는다
  (`bolt/README.md`: *"Nothing on the PostgreSQL path depends on it"*).
- Cypher 의미론이 한 벌뿐이다. 오류 메시지도 컴파일러 것이 그대로 전달된다.

**부정적 / 감수한 대가**
- **배포 단계가 하나 늘어난다.** `CREATE EXTENSION` 만으로는 Bolt가 열리지 않는다.
  `start.sh`가 PostgreSQL 옆에서 함께 띄우는 형태로 대응한다.
- 프로세스가 하나 더 있으므로 모니터링·재시작·설정(`OG_BOLT_*` 환경변수) 대상이 늘어난다.
- **TLS를 종단하지 않는다.** 평문 Bolt만 받으며, 운영에서는 TLS 종단 프록시를 앞에 두는
  것이 전제다 — Complexity Tracking에 문서화된 한계로 기록되어 있다.
- `ROUTE`는 단일 서버 응답만 돌려준다. 원칙 VII에 대한 부분 이탈로 기록되어 있으며,
  진짜 라우팅 테이블은 spec 007(ADR-021)의 클러스터가 있어야 의미를 갖는다.

## 재검토 조건

- PostgreSQL이 스레드 안전한 백엔드 내 질의 실행 경로를 제공하면(SPI의 근본 제약이
  해소되면) 배경 워커 안이 다시 선택지가 된다.
- spec 007의 샤딩이 구현되면 `ROUTE` 응답을 실제 라우팅 테이블로 바꿔야 한다.
- Bolt 5.x 요구가 실제 드라이버 협상 실패로 나타나면 지원 버전을 재평가한다. 현재는
  드라이버가 버전 **범위**를 제안하므로 4.4로 충족된다 (`bolt/README.md` 지원 매트릭스).

<!-- affects: bolt, api, ops, architecture -->
<!-- requires-update: docs/99_decisions/ADR-018-bolt-auth-uses-postgres-roles.md -->
