# ADR-001: PostgreSQL 포크가 아닌 확장으로 구현한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-05 (헌법 비준일) |
| 영향 범위 | 전 계층 (storage, cypher, vector, interop, ops) |
| 근거 | `.specify/memory/constitution.md` 원칙 I, `engine/sql/bootstrap.sql:8`, `engine/src/lib.rs:23-24`, `README.md` "Why this exists" |

> **이 문서가 답하는 질문**
> - 왜 자체 그래프 서버나 PostgreSQL 포크를 만들지 않았는가?
> - 이 결정이 이후 모든 설계에 어떤 제약을 걸었는가?

## 배경

Neo4j 급 트래버설 성능과 TypeDB 급 타입 추론을 동시에 원하는 제품에게 가장 자연스러운
경로는 **전용 엔진을 새로 쓰는 것**이다. 실제로 이 프로젝트가 비교 대상으로 삼는 세 제품 중
둘(Neo4j, TypeDB)이 그 길을 갔다.

그러나 제품을 쓰는 쪽은 이미 PostgreSQL 위에 운영 도구·드라이버·백업·관리형 서비스(Supabase
포함)를 갖고 있다. 그래프 데이터만 별도 서버·별도 백업 경로로 분리하면 그 자산이 전부
무효화된다.

## 고려한 선택지

1. **독립 그래프 서버** — 저장 포맷·실행기·프로토콜을 전부 자유롭게 설계.
   장점: 물리 배치·인덱스·실행 모델에 제약 없음. 단점: 운영 생태계를 처음부터 다시
   만들어야 하고, 기존 관계형 데이터와 하나의 트랜잭션 안에 들어가지 못한다.
2. **PostgreSQL 포크 / 커널 패치** — 상위 파서 훅, 커스텀 저장 계층을 커널에 직접 추가.
   장점: 원칙 II(Cypher 1급 문법)를 문자 그대로 달성 가능. 단점: 관리형 PostgreSQL에
   배포 불가, 업스트림 추종 비용이 영구적으로 발생.
3. **표준 PostgreSQL 확장** — `CREATE EXTENSION` 한 줄로 설치, 허용된 확장점만 사용.

## 결정

**3안.** 패치되지 않은 표준 PostgreSQL 16 이상에 `CREATE EXTENSION ontological CASCADE`
한 줄로 설치되는 확장으로만 구현한다. 헌법 원칙 I은 **NON-NEGOTIABLE**로 지정되어 다른
모든 원칙보다 우선한다.

## 근거

- 헌법 원칙 I: *"PostgreSQL 생태계(운영 도구, 드라이버, 관리형 서비스, 확장)를 그대로
  상속하는 것이 이 프로젝트의 유일한 비대칭 우위다. 포크하는 순간 그 우위를 잃는다."*
- 부트스트랩 SQL이 이 제약을 코드 주석으로 못 박고 있다 —
  `engine/sql/bootstrap.sql:8`: *"Constitution I: nothing here requires a patched
  PostgreSQL."*, `:9` *"Constitution IX: every structure below is an ordinary heap
  relation, so it inherits MVCC / WAL / vacuum / pg_dump for free."*
- 그 귀결이 실제 이득으로 회수된 지점이 여럿 확인된다:
  - RLS가 트래버설 중간 노드에 **자동으로** 적용된다. 컴파일된 Cypher가 참조하는 것이
    일반 테이블이기 때문이다 (`specs/005-postgres-supabase-interop/plan.md` "RLS 적용 지점").
  - 벡터 인덱싱을 자체 구현하지 않고 pgvector를 재사용한다 (ADR-019).
  - 읽기 복제가 코드 0줄로 성립한다 (`specs/007-distributed-cluster/plan.md` "P0").

## 결과

**긍정적**
- 그래프 데이터가 `pg_dump`/PITR/물리복제에 그대로 참여한다. 별도 백업 경로가 없다.
- 기존 관계형 테이블과 **하나의 트랜잭션**에 들어간다.
- Supabase 등 관리형 환경에 배포 가능하다 (superuser 전용 기능 의존 0).

**부정적 / 감수한 대가**
- Cypher를 최상위 문장 문법으로 만들 수 없다. PG16에 상위 파서 교체 훅이 없기 때문이며,
  이것이 헌법 원칙 II의 부분 미달로 기록되어 있다 (ADR-008).
- Table Access Method를 쓰지 않는 선택과 결합되어, 튜플 레이아웃을 우리가 통제하지 못한다
  (ADR-002).
- 백엔드-로컬 CSR 같은 프로세스 지역 최적화는 확장 안에서 "선택적 가속 경로"로만 존재할 수
  있다 (ADR-014).

## 재검토 조건

- PostgreSQL 업스트림이 **상위 파서 대체 훅**을 제공하면, 원칙 II 미달분(ADR-008)을
  포크 없이 해소할 수 있는지 재평가한다.
- 관리형 PostgreSQL 배포를 제품 목표에서 제외하기로 결정하는 경우에만 이 ADR을 다시 연다.
  현재로서는 헌법이 NON-NEGOTIABLE로 지정했으므로 **요구사항 쪽을 수정한다**.

<!-- affects: architecture, storage, cypher, ops, security -->
<!-- requires-update: docs/99_decisions/ADR-002-no-table-access-method.md, docs/99_decisions/ADR-008-cypher-entry-via-function-call.md -->
