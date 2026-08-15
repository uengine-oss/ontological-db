# Ontological Constitution

> **Ontological** — PostgreSQL 위에서 동작하는, AI 에이전트 시대를 위한
> 사이퍼(Cypher) 중심 온톨로지 그래프 데이터베이스.
> Neo4j 급 그래프 성능 + TypeDB 급 타입/상속 추론 + pgvector 기반 시맨틱 검색을
> 단일 PostgreSQL 확장으로 제공한다.

## Core Principles

### I. 포크가 아닌 확장이다 (Extension, Not a Fork) — NON-NEGOTIABLE

Ontological은 **패치되지 않은 표준 PostgreSQL(16+)에 `CREATE EXTENSION` 한 줄로 설치**되어야
한다. PostgreSQL 소스 수정, 커스텀 빌드, 커널 패치를 전제하는 설계는 채택하지 않는다.

- 허용 확장점만 사용한다: Table Access Method, Index Access Method, Custom Scan Provider,
  planner/executor hooks, Custom WAL Resource Manager(rmgr), background worker,
  shared memory hook, type/operator/opclass 등록.
- Supabase를 포함한 관리형 PostgreSQL 환경에 배포 가능해야 한다. 어떤 기능도
  "superuser 전용 커널 기능"에 의존해서는 안 된다. 의존한다면 그 기능은 optional
  가속 경로여야 하고, 없을 때도 정상 동작하는 fallback이 있어야 한다.
- 기존 테이블·트랜잭션·백업(pg_dump/PITR)·논리복제와 공존해야 한다. 그래프 데이터만
  따로 백업해야 하는 설계는 위반이다.

**근거**: PostgreSQL 생태계(운영 도구, 드라이버, 관리형 서비스, 확장)를 그대로 상속하는 것이
이 프로젝트의 유일한 비대칭 우위다. 포크하는 순간 그 우위를 잃는다.

### II. 사이퍼가 1급 언어다 (Cypher-First)

Cypher는 SQL 위에 얹은 문자열 함수 래퍼가 아니라 **PostgreSQL이 이해하는 또 하나의 질의
언어**다.

- `cypher('SELECT ...')` 형태로 질의를 문자열에 가두고 결과를 `agtype`으로 반환하는
  Apache AGE 방식은 **명시적으로 거부**한다. 이 방식은 옵티마이저가 그래프 패턴 내부를 볼 수
  없게 만들고 파라미터 바인딩·prepared statement·plan cache를 무력화한다.
- Cypher는 자체 파서 → 논리 계획 → PostgreSQL Custom Scan/Join 물리 연산자로 내려간다.
  통계·비용 모델·EXPLAIN·parallel query·cursor가 그래프 연산자에도 동일하게 적용된다.
- 문법 기준은 **openCypher**이며, 로드맵은 **ISO GQL** 정렬을 향한다. Neo4j 고유 확장은
  호환 계층으로 제공하되 코어 의미론을 오염시키지 않는다.
- Cypher 결과는 표준 PostgreSQL 타입/컴포지트로 반환되어 SQL에서 `JOIN` 가능해야 한다.

### III. 성능은 저장구조에서 나온다 (Native Graph Storage)

"그래프 DB인데 사실은 정규 테이블 두 개"인 구조는 성능을 담보하지 못한다.

- 인접 정보는 **인접 세그먼트(CSR 계열 구조)**로 저장하여 이웃 접근이 인덱스 조회가 아닌
  **순차 메모리 접근**이 되도록 한다. 홉당 비용은 B-tree 조회가 아니라 포인터 추적에 가까워야
  한다.
- 프로퍼티는 로우마다 파싱해야 하는 스키마리스 JSON(JSONB/agtype)에 담지 않는다. 타입 카탈로그가
  아는 고정 스키마를 활용한 **타입 기반 슬롯 저장(고정폭 우선, 가변폭은 오버플로)**을 기본으로
  한다.
- 식별자는 (그래프, 타입, 지역 오프셋)을 인코딩한 **고정폭 정수**여야 한다. 문자열 키 조인으로
  트래버설하는 설계는 금지한다.
- 물리 배치는 **지역성(locality)이 설계 목표**다: 같은 타입·같은 서브그래프·최근 함께 접근된
  이웃은 같은 페이지 근방에 모인다.
- 모든 그래프 구조는 MVCC와 WAL에 참여한다(원칙 IX).

**측정 기준**: 1-hop 이웃 확장 비용은 동일 데이터에 대한 Apache AGE 대비 최소 5배 이상
개선되어야 하며, 그렇지 못한 설계 대안은 채택하지 않는다.

### IV. 온톨로지 우선 타이핑 (Ontology-First Typing)

레이블은 문자열 태그가 아니라 **타입 시스템의 시민**이다. TypeDB의 개념 모델을 벤치마킹한다.

- 1급 개념: `entity type`, `relation type`(+ role), `attribute type`, 그리고 이들 사이의
  **서브타입(상속) 관계**. 다중 상속과 role 상속을 지원한다.
- 스키마는 카탈로그에 저장되어 옵티마이저·인덱스·질의 검증이 참조한다. 스키마리스 모드는
  "타입이 없다"가 아니라 "암묵 타입으로 승격"으로 처리한다.
- **상속 질의는 상수 시간이어야 한다.** `MATCH (v:Vehicle)`가 `Car`, `Truck`, `EV` 를 찾을 때
  런타임 재귀 CTE로 계층을 펼치는 방식은 금지한다. 계층에 구간 라벨링(nested-set/interval
  encoding)을 부여해 서브타입 판정을 **단일 범위 비교**로 수행한다.
- 제약(cardinality, role player 타입, key, 필수 attribute)은 선언되고 강제된다. Neo4j의
  멀티레이블이 못 하는 부분이 정확히 이 지점이며, 우리의 차별점이다.

### V. 벡터는 노드·엣지·경로 모두에 붙는다 (Vectors on Everything)

AI 에이전트가 쓰는 그래프 DB에서 벡터는 부가기능이 아니라 트래버설과 같은 층위의 연산이다.

- 임베딩은 **노드뿐 아니라 관계(엣지)와 경로/서브그래프**에도 부착·검색 가능해야 한다.
  "이 노드와 비슷한 노드"만이 아니라 **"이 관계와 비슷한 관계"**를 물을 수 있어야 한다.
- 벡터 저장·인덱싱은 자체 구현하지 않고 **pgvector(HNSW/IVFFlat)를 재사용**한다. 원칙 I의
  귀결이다.
- 하이브리드 검색은 **옵티마이저에 통합**된다. 그래프 필터를 무시하고 top-k를 뽑은 뒤 사후
  필터링하는(post-filter) 구현은 금지한다. 그래프 술어는 벡터 탐색 내부로 push-down 되어야
  한다.
- 벡터 결과는 항상 그래프 컨텍스트(타입, 경로, 출처)와 함께 반환된다 — 원칙 VIII의 근거 자료다.

### VI. 코어는 하나, 표준은 어댑터로 (One Core, Adapters at the Edge)

RDF/OWL/SPARQL/SHACL/GraphQL 지원은 **어댑터 계층**에서 제공한다.

- 저장 엔진과 타입 시스템은 하나뿐이다. 트리플 스토어를 따로 두는 이중 저장 구조는 금지한다.
- RDF 트리플, OWL 클래스/프로퍼티는 원칙 IV의 타입 시스템으로 **매핑**된다. SPARQL은 동일
  논리 계획으로 컴파일된다.
- 어댑터는 코어 의미론을 바꿀 수 없다. 어댑터에만 필요한 기능은 어댑터 안에 머문다.
- 중심 언어는 언제나 Cypher다. 어댑터 미지원 구문은 명확히 문서화된 한계로 남긴다.

### VII. 처음부터 확장 가능하게 (Scale-Out by Design)

클러스터링은 나중에 붙이는 기능이 아니다. 단, 단일 노드 성능을 희생하는 대가로 얻지 않는다.

- 파티셔닝은 **그래프 인지적(graph-aware)** 이다. 해시 분산이 기본이지만 엣지 컷을 줄이는
  지역성 기반 배치를 지원한다.
- 트래버설·필터·집계는 가능한 한 원격 노드로 **push-down** 된다. 홉마다 코디네이터로 데이터를
  끌어오는 설계는 금지한다.
- 단일 노드 배포는 분산 코드 경로 때문에 느려지지 않아야 한다(분산 계층은 opt-in).
- 읽기 확장은 복제로, 쓰기 확장은 샤딩으로. 어느 쪽도 ACID(원칙 IX)를 포기하지 않는다.

### VIII. AI 에이전트가 1급 사용자 (Agent-Native) — NON-NEGOTIABLE

사람 개발자만이 아니라 **LLM 에이전트가 직접 쓰는 DB**로 설계한다.

- 스키마는 기계가 읽을 수 있는 형태로 항상 introspect 가능해야 한다(타입 계층, role, 제약,
  통계, 예시값). 에이전트의 Cypher 생성 정확도는 이 API 품질에 비례한다.
- 오류 메시지는 결정적이고 교정 가능해야 한다: 무엇이 틀렸는지 + 가장 가까운 유효 대안.
- 질의 결과는 **출처(provenance)** 를 동반할 수 있어야 한다 — 어떤 노드/엣지/경로가 답에
  기여했는가. RAG 인용의 기반이다.
- 그래프는 시간 축을 갖는다: 사실의 유효 시각과 기록 시각을 보존하여 에이전트가
  "언제 기준으로 참인가"를 물을 수 있어야 한다.

### IX. ACID는 협상 대상이 아니다 (ACID Non-Negotiable)

- 모든 그래프 변경(노드, 엣지, 인접 구조, 타입 카탈로그, 인덱스)은 **트랜잭션·MVCC·WAL**에
  참여한다. crash-safe 하며 PITR·논리복제와 정합한다.
- "성능을 위해 인접 리스트만 비동기로 갱신" 같은 최종적 일관성 지름길은 금지한다. 캐시 계층은
  허용하되, 캐시는 진실의 원천이 될 수 없다.
- 스키마(타입) 변경은 트랜잭션 안에서 일어나며 롤백 가능하다.

### X. 성능 주장은 벤치마크로 증명한다 (Benchmarked or It Didn't Happen)

- 성능 주장에는 재현 가능한 벤치마크가 동반되어야 한다. 기준 워크로드는 **LDBC SNB
  (Interactive + BI)**, 비교 대상은 **Apache AGE, Neo4j, 순수 PostgreSQL 재귀 CTE**다.
- 정확성 기준선은 **openCypher TCK** 통과율이다. 통과율은 회귀할 수 없다.
- CI는 성능 회귀 게이트를 갖는다. 핵심 지표가 기준선 대비 유의미하게 나빠지면 병합을 차단한다.
- 벤치마크 하네스, 데이터셋 생성기, 결과는 저장소에 공개된다.

## Technology Constraints & Standards

**필수 기반**

- PostgreSQL 16 이상 (패치 없는 표준 빌드). PostgreSQL 확장 ABI 규약 준수.
- pgvector — 벡터 저장/인덱싱의 유일한 구현체 (원칙 V).
- Supabase 호환: PostgREST 노출, RLS, Realtime, 마이그레이션 도구와 공존.

**질의 언어**

- 1차: openCypher (목표: ISO GQL 정렬).
- 어댑터: SPARQL 1.1, GraphQL, SQL/PGQ 스타일 SQL 내장 문법.
- 표준 준수 여부는 문서화된 매트릭스로 관리한다 — "지원함"이라는 모호한 표현 금지.

**구현 언어**

- 코어 저장/AM/실행 계층은 C 또는 Rust(pgrx). **하나를 선택해 저장소 전체에 일관 적용**하며,
  선택 근거는 첫 `plan.md`에 기록된다. 혼합 사용은 명시적 ADR 없이는 금지한다.
- 어느 쪽이든 PostgreSQL 메모리 컨텍스트·에러 처리(elog/ereport) 규약을 따른다. 확장 안에서의
  panic/unwind가 서버를 죽여서는 안 된다.

**금지 사항 (Anti-Patterns)**

- 노드/엣지를 각각 하나의 일반 힙 테이블에 담고 프로퍼티를 JSONB/agtype으로 저장하는 구조.
- Cypher를 문자열 인자로 받는 함수 인터페이스.
- 상속 판정을 위한 런타임 재귀 CTE.
- 벡터 검색 후 그래프 조건 사후 필터링.
- 그래프 데이터를 위한 별도 백업·복제 경로.

## Development Workflow & Quality Gates

**Spec-Driven 개발**

1. 모든 기능은 `specs/NNN-*/spec.md` 에서 시작한다 (`/speckit-specify`).
2. `plan.md` 는 반드시 **Constitution Check** 섹션을 포함하고, 각 원칙에 대한 준수/위반 여부와
   위반 시 정당화를 기록한다.
3. `tasks.md` 없이 구현하지 않는다.
4. 스펙 간 의존성은 각 스펙의 Assumptions에 명시한다.

**품질 게이트 (병합 차단 조건)**

- openCypher TCK 통과율 회귀 → 차단.
- 벤치마크 핵심 지표 회귀(임계치는 009 스펙에서 정의) → 차단.
- `CREATE EXTENSION` → 워크로드 → `pg_dump`/restore → 재검증 사이클 실패 → 차단.
- crash recovery 테스트(비정상 종료 후 그래프 구조 정합성) 실패 → 차단.
- 새 SQL 노출 API에 대한 RLS/권한 테스트 부재 → 차단.

**테스트 계층**

- 단위(연산자·자료구조), 통합(SQL 레벨 회귀 = `pg_regress`/`pgTAP`), 적합성(TCK, SPARQL 테스트
  스위트), 성능(LDBC), 내구성(crash/replication).

## Governance

- 본 헌법은 다른 모든 관행·문서·관습에 우선한다. 충돌 시 헌법이 이긴다.
- 원칙 위반이 필요한 설계는 해당 `plan.md` 의 Complexity Tracking에 (a) 위반 원칙,
  (b) 위반이 없으면 불가능한 이유, (c) 제거 계획을 기록해야 한다. 기록 없는 위반은 리뷰에서
  반려한다.
- **NON-NEGOTIABLE** 로 표시된 원칙(I, VIII, IX)은 예외를 허용하지 않는다. 이를 어겨야만
  달성되는 요구사항은 요구사항 쪽을 수정한다.
- 개정 절차: 변경 제안 → 영향받는 스펙/템플릿 동기화 → 버전 증가 → 저장소 커밋.
  MAJOR = 원칙 삭제/재정의, MINOR = 원칙 추가 또는 실질적 확장, PATCH = 문구 정리.
- 모든 리뷰(사람·에이전트)는 헌법 준수를 확인해야 한다. 런타임 개발 지침은 `CLAUDE.md` 를
  참조한다.

**Version**: 1.0.0 | **Ratified**: 2026-08-05 | **Last Amended**: 2026-08-05
