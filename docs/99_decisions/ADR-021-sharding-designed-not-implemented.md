# ADR-021: 샤딩을 설계만 확정하고 구현하지 않는다 (읽기 복제만 제공)

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/007-distributed-cluster/plan.md` 기준일) |
| 영향 범위 | cluster, storage, ops, api |
| 근거 | `specs/007-distributed-cluster/plan.md` Summary·Constitution Check·Complexity Tracking, `.specify/memory/constitution.md` 원칙 VII·IX, `README.md` 스펙 상태표 |

> **이 문서가 답하는 질문**
> - "Scale-Out by Design"이 헌법 원칙인데 왜 샤딩이 없는가?
> - 지금 무엇이 되고 무엇이 안 되는가?

## 배경

헌법 원칙 VII는 *"클러스터링은 나중에 붙이는 기능이 아니다"* 라고 선언한다. spec 007이
그 요구를 받아 그래프 인지 파티셔닝 + 연산 push-down 클러스터를 정의했다.

그러나 원칙 IX(ACID)는 **NON-NEGOTIABLE**이다. 분산 트랜잭션의 원자성(FR-017)과 조용한
부분 결과 금지(FR-020)를 동시에 만족하려면 2PC와 장애 주입 검증이 필요하다.

## 고려한 선택지

1. **일단 샤딩만 붙이기** — 확장성 이야기는 완성된다. `plan.md`가 기각:
   *"'일단 샤딩만 붙이기'는 원칙 IX를 조용히 위반한다."*
2. **2PC + 장애 주입까지 완성하고 릴리스** — 원칙 VII·IX를 모두 충족. 단일 노드 코어가
   안정되기 전에 하기에는 순서가 틀렸다.
3. **P0(읽기 복제)만 구현하고, 샤딩은 설계를 확정한 채 미구현으로 문서화**

## 결정

**3안.** `specs/007-.../plan.md` Summary 원문:
> **현재 릴리스(v0.1) 상태: P0만 구현. 샤딩은 미구현이며 문서에 그렇게 표기한다.**
> 이것은 축소가 아니라 정직한 단계화다 — 부분 구현된 분산은 원칙 IX(ACID)를 조용히 깨뜨린다.

## 근거

### 지금 되는 것 — P0 읽기 복제 (구현됨)

**별도 코드가 필요 없다는 것이 요점이다.** 모든 그래프 구조가 일반 힙 릴레이션이므로
(ADR-001, `engine/sql/bootstrap.sql:9`) PostgreSQL 스트리밍 복제가 그대로 동작한다.
필요한 것은 검증뿐이다 — `og_check_integrity()`를 대기 서버에서 실행해 주 서버와 같은
결과를 확인한다.

### 설계는 확정되어 있다 — P1 파티션 키 샤딩 (미구현)

| 항목 | 결정 내용 |
|---|---|
| 샤드 비트 | 식별자 상위 9비트가 이미 샤드를 담는다 (ADR-003). 재분배 시 `local_id` 불변 |
| 배치 정책 | `og_catalog.placement(graph_id, type_id, strategy, key_prop)` |
| 원격 실행 | `postgres_fdw` 위에 얹는다 — *"원칙 I을 지키면서 push-down을 얻는 유일한 경로"* |
| 분산 계획 형태 | 컴파일러가 이미 SQL을 뱉으므로, **같은 SQL을 원격 노드에서 실행하고 코디네이터에서 병합** (FR-011: 홉마다 데이터를 끌어오지 않는다) |

### 미착수 — P2 그래프 인지 파티셔닝

- Complexity Tracking이 이탈을 정식 기록한다:
  > **원칙 VII 부분 미충족** | 분산 트랜잭션의 원자성(FR-017)과 조용한 부분 결과
  > 금지(FR-020)를 동시에 만족하려면 2PC + 장애 주입 검증이 필요하고, 이는 단일 노드 코어가
  > 안정된 뒤에 해야 한다
- Constitution Check가 원칙 IX를 `✅`로 두는 근거가 흥미롭다:
  *"미구현이므로 깨뜨릴 것이 없다. 2PC 없이 분산 쓰기를 열지 않는다."*
- README 스펙 상태표도 같은 표기를 유지한다:
  `007 | 분산 클러스터 | **read replicas only** — sharding is designed, not implemented.`

## 결과

**긍정적**
- 읽기 확장이 코드 0줄로 성립하고, PostgreSQL 운영 관행을 그대로 쓴다.
- 단일 노드 배포가 분산 코드 경로 때문에 느려지지 않는다 (원칙 VII의 요구를 충족).
- 샤드 비트가 이미 예약되어 있어, 나중에 샤딩을 붙여도 기존 데이터의 ID가 재작성되지 않는다.

**부정적 / 감수한 대가**
- **쓰기 확장 경로가 없다.** 단일 노드 용량이 곧 그래프 용량이다.
- Bolt 게이트웨이의 `ROUTE`가 단일 서버 응답만 돌려준다 (ADR-017). `neo4j://` 스킴 드라이버가
  붙기는 하지만 진짜 라우팅은 아니다.
- 식별자의 샤드 9비트가 쓰이지 않은 채 모든 ID에 실려 있다 (ADR-003의 대가).
- 헌법 원칙 VII를 완전히 충족하지 못한 상태가 릴리스에 남아 있다.

## 재검토 조건

- **샤드 수요가 단일 노드 용량을 넘을 때** — 이것이 이 결정의 1차 트리거다.
- 그 시점에 착수 조건은 셋이다: (a) 단일 노드 코어의 안정화, (b) 2PC 구현,
  (c) **장애 주입 검증**. 셋이 갖춰지지 않은 채 샤딩을 열면 원칙 IX를 조용히 위반한다.
- `postgres_fdw`가 요구되는 push-down(그래프 패턴 전체의 원격 실행)을 지원하지 못한다고
  판명되면, P1 설계 자체를 다시 그려야 한다.

<!-- affects: cluster, storage, ops, api, bolt -->
<!-- requires-update: docs/99_decisions/ADR-003-int8-identifier-bitfields.md, docs/99_decisions/ADR-017-bolt-gateway-separate-process.md -->
