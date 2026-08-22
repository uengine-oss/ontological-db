# ADR 색인 — Architecture Decision Records

> **이 문서가 답하는 질문**
> - 이 저장소가 내린 구조적 결정에는 어떤 것들이 있고, 각각 어디에 기록되어 있는가?
> - 어떤 결정이 아직 유효하고, 어떤 것이 다른 결정으로 대체되었는가?
> - 특정 코드를 바꾸려 할 때 어떤 결정을 먼저 읽어야 하는가?

## 이 디렉터리의 성격

여기에 있는 ADR은 **새로 만든 결정이 아니다.** 이 저장소는 이미 결정을 내렸고, 그 근거가
`README.md`, `.specify/memory/constitution.md`, `specs/00N/plan.md`의 *Complexity Tracking*,
그리고 **소스 주석**에 흩어져 있었다. 각 ADR은 그것을 발굴해 정식화한 것이며, 모든 주장에
`파일:라인` 또는 스펙 번호 인용이 붙어 있다.

**근거를 찾지 못한 결정은 여기에 없다.** 확인되지 않은 것은 "미확인 / 미결정"으로 적는 것이
이 저장소의 규칙이다.

## 사실 (Facts)

| 항목 | 값 |
|---|---|
| 거버넌스 문서 | `.specify/memory/constitution.md` (v1.0.0, 비준 2026-08-05) |
| 헌법 원칙 수 | 10개 (I~X). **NON-NEGOTIABLE**: I(포크 금지), VIII(에이전트 네이티브), IX(ACID) |
| 헌법 이탈 기록 위치 | 각 `specs/NNN-*/plan.md`의 *Complexity Tracking* 절 |
| 스펙 수 | 11개 (`specs/001-*` ~ `specs/011-*`) |
| 커밋 이력 | 6개 커밋, 2026-08-15 ~ 2026-08-17 |

## ADR 색인

| # | 제목 | 상태 | 날짜 | 영향 범위 |
|---|---|---|---|---|
| [001](ADR-001-postgresql-extension-not-fork.md) | PostgreSQL 포크가 아닌 확장으로 구현한다 | Accepted | 2026-08-05 | 전 계층 |
| [002](ADR-002-no-table-access-method.md) | v1에서 Table Access Method를 구현하지 않는다 | Accepted (v2 재평가) | 2026-08-06 | storage |
| [003](ADR-003-int8-identifier-bitfields.md) | 식별자를 `[shard:9][type:18][local:36]` int8 비트필드로 인코딩한다 | Accepted | 2026-08-06 | storage, cypher, typeql, cluster |
| [004](ADR-004-csr-adjacency-segments.md) | 인접 정보를 CSR형 세그먼트(힙 튜플당 이웃 ≤256개)로 저장한다 | Accepted | 2026-08-06 | storage, cypher, security, ops |
| [005](ADR-005-typed-property-columns.md) | 프로퍼티를 타입 카탈로그가 생성한 실제 컬럼으로 저장한다 (미선언은 `__ext`) | Accepted | 2026-08-06 | storage, catalog, cypher, vector, security |
| [006](ADR-006-write-time-property-promotion.md) | 미선언 프로퍼티를 쓰기 시점에 승격하고, 충돌 시 `text`로 단방향 확장한다 | Accepted | 2026-08-17 | storage, catalog, cypher, vector |
| [007](ADR-007-nested-set-interval-labels.md) | 상속 판정을 nested-set 구간 라벨의 단일 범위 비교로 수행한다 | Accepted | 2026-08-06 | catalog, cypher, typeql, adapters |
| [008](ADR-008-cypher-entry-via-function-call.md) | Cypher를 최상위 문장 문법이 아니라 함수 호출로 진입시킨다 | Accepted (원칙 II 부분 미달) | 2026-08-06 | cypher, api, bolt, interop |
| [009](ADR-009-read-sql-write-rust-split.md) | 읽기 경로는 SQL 생성으로, 쓰기 경로는 Rust SPI로 이원화한다 | Accepted | 2026-08-06 | storage, cypher, typeql |
| [010](ADR-010-access-sql-language-sql.md) | `access.sql`의 접근 경로를 전부 `LANGUAGE sql`로 작성한다 | Accepted | 2026-08-06 | storage, cypher, performance |
| [011](ADR-011-single-jsonb-parameter-binding.md) | 사용자 값을 SQL 텍스트로 보간하지 않고 단일 jsonb 파라미터로 바인딩한다 | Accepted | 2026-08-06 | cypher, security, storage |
| [012](ADR-012-visited-set-bfs-rewrite.md) | 가변 길이 경로를 트레일 열거에서 방문집합 BFS로 재작성한다 | Accepted | 2026-08-17 | cypher, storage, performance |
| [013](ADR-013-conservative-bfs-rewrite.md) | 방문집합 BFS 재작성을 보수적으로만 적용한다 (관측 가능성 + 손익분기) | Accepted | 2026-08-17 | cypher, performance, correctness |
| [014](ADR-014-csr-not-automatic.md) | 백엔드-로컬 CSR(`og_csr_build`)을 자동으로 적용하지 않는다 | Accepted | 2026-08-17 | storage, cypher, security, ops |
| [015](ADR-015-two-query-languages.md) | 하나의 그래프 위에 Cypher와 TypeQL 두 질의 언어를 제공한다 | Accepted (원칙 II 이탈 기록됨) | 2026-08-06 | typeql, cypher, catalog, api |
| [016](ADR-016-typeql-relations-reified-as-nodes.md) | TypeQL 관계를 간선이 아니라 노드로 물화한다 | Accepted | 2026-08-06 | typeql, storage, catalog, cypher |
| [017](ADR-017-bolt-gateway-separate-process.md) | Bolt 게이트웨이를 배경 워커가 아니라 별도 프로세스로 둔다 | Accepted | 2026-08-06 | bolt, api, ops, architecture |
| [018](ADR-018-bolt-auth-uses-postgres-roles.md) | Bolt 인증에 PostgreSQL role을 그대로 쓴다 (두 번째 사용자 저장소 없음) | Accepted | 2026-08-06 | bolt, security, api |
| [019](ADR-019-pgvector-and-relationship-embeddings.md) | 벡터는 pgvector를 재사용하고, 임베딩을 관계에도 부여한다 | Accepted | 2026-08-06 | vector, storage, catalog, cypher |
| [020](ADR-020-blocking-ureq-for-genai.md) | 외부 네트워크 호출에 async 대신 blocking `ureq`를 쓴다 | Accepted | 2026-08-17 | vector, compat, security, ops |
| [021](ADR-021-sharding-designed-not-implemented.md) | 샤딩을 설계만 확정하고 구현하지 않는다 (읽기 복제만) | Accepted (원칙 VII 부분 미충족) | 2026-08-06 | cluster, storage, ops, api |
| [022](ADR-022-typedb-example-as-conformance-gate.md) | TypeDB 공식 예제를 무수정으로 통과시키는 것을 적합성 기준으로 삼는다 | Accepted | 2026-08-06 | typeql, testing, docs |
| [023](ADR-023-benchmark-correctness-gate.md) | 벤치마크에 정확성 게이트를 두고, 불일치 시 성능 수치를 무효 처리한다 | Accepted | 2026-08-06 | testing, docs, performance |
| [024](ADR-024-rdf-overflow-table-for-lossless-mapping.md) | RDF 매핑 불가 구문을 `og_triple_overflow`에 원형 보존한다 | Accepted | 2026-08-06 | adapters, data, catalog |
| [025](ADR-025-privilege-model-default-deny.md) | 함수 권한은 기본 거부하고, 역할은 확장이 만들지 않는다 | Accepted | 2026-08-23 | security, api, data, operations |

**Superseded / Deprecated 항목은 현재 없다.** 대체된 결정이 발생하면 상태 열을
`Superseded by ADR-0NN`으로 바꾸고, 대체 ADR에 근거를 남긴다.

## 헌법 원칙 ↔ ADR 대응표

| 원칙 | 요지 | 관련 ADR | 현재 상태 |
|---|---|---|---|
| I. 포크가 아닌 확장 (NON-NEGOTIABLE) | 표준 PG16에 `CREATE EXTENSION` | 001, 002, 008, 019, 021 | ✅ 충족 |
| II. Cypher 1급 | 문자열 함수 래퍼 거부 | 008, 010, 015 | ⚠️ **부분** — 진입점이 함수 호출 (ADR-008), 두 번째 언어 추가 (ADR-015) |
| III. 성능은 저장구조에서 | CSR형 인접, 실컬럼, 고정폭 ID | 002, 003, 004, 005 | ⚠️ **부분** — TableAM 미사용 (ADR-002) |
| IV. 온톨로지 우선 타이핑 | 상속 판정 상수 시간 | 007, 016 | ✅ 충족 |
| V. 벡터는 노드·엣지·경로 모두에 | pgvector 재사용, push-down | 019 | ⚠️ 노드·관계는 충족, **경로/서브그래프 임베딩은 미확인** |
| VI. 코어는 하나, 표준은 어댑터로 | 이중 저장 금지 | 015, 016, 017, 024 | ✅ 충족 |
| VII. 처음부터 확장 가능하게 | 복제 + 샤딩 | 003, 021 | ⚠️ **부분** — 샤딩 미구현 (ADR-021), Bolt `ROUTE` 형식만 |
| VIII. 에이전트 네이티브 (NON-NEGOTIABLE) | introspection, 교정 가능한 오류 | 022 (간접) | 미확인 — spec 008 범위, 별도 ADR 없음 |
| IX. ACID (NON-NEGOTIABLE) | 모든 그래프 변경이 MVCC/WAL | 002, 004, 009, 012, 014, 021 | ✅ 충족 (CSR은 자동 적용하지 않음으로써 유지) |
| X. 벤치마크로 증명 | 재현 가능한 측정 | 012, 013, 022, 023 | ✅ 충족 |

## 목적별 진입점 — 무엇을 바꾸기 전에 무엇을 읽어야 하는가

| 바꾸려는 것 | 먼저 읽을 ADR |
|---|---|
| `og_data.og_adj` 스키마, 인접 갱신 로직 | 004, 002, 009 |
| 프로퍼티 저장·타입 추론·컬럼 승격 | 005, 006, 011 |
| Cypher 컴파일러의 가변 길이 경로 처리 | 012, 013, 010 |
| `og_reach` / `og_csr_*` 순회 함수 | 012, 014, 010 |
| 타입 계층·상속 판정 | 007, 003 |
| Bolt 게이트웨이 | 017, 018, 008 |
| TypeQL 파서·컴파일러·저장 매핑 | 015, 016, 022 |
| 벡터 검색·임베딩 | 019, 005, 020 |
| 벤치마크 하네스·성능 주장 | 023, 012, 013 |
| 분산·복제·샤드 | 021, 003 |
| RDF/OWL 어댑터 | 024, 007 |

## 필수 / 금지 (Required / Forbidden)

이 목록은 위 ADR들에서 파생된 **강제 규칙**이다. 위반하려면 해당 ADR을 먼저 개정해야 한다.

### 금지 (Forbidden)

- **PostgreSQL 커널 패치·포크를 전제하는 설계** — ADR-001, 헌법 원칙 I (NON-NEGOTIABLE).
- **프로퍼티를 `agtype`/JSONB 블롭 하나에 담는 구조** — ADR-005, 헌법 "금지 사항".
- **상속 판정을 런타임 재귀 CTE로 하는 것** — ADR-007, 헌법 "금지 사항".
- **사용자 값을 SQL 텍스트에 보간하는 것** — ADR-011, spec 003 FR-026.
- **`access.sql`에 `LANGUAGE plpgsql` / C 집합 반환 함수를 추가하는 것** — ADR-010
  (옵티마이저 장벽이 된다).
- **`og_csr_*` 경로를 Cypher 컴파일러가 자동으로 선택하게 만드는 것** — ADR-014
  (MVCC·RLS를 조용히 포기하게 된다).
- **벡터 검색 후 그래프 조건을 사후 필터링하는 것** — ADR-019, 헌법 원칙 V.
- **인접 구조를 비동기로 갱신하는 것** — 헌법 원칙 IX, ADR-009.
- **2PC와 장애 주입 검증 없이 분산 쓰기를 여는 것** — ADR-021.
- **시스템 간 결과 대조 없이 성능 수치를 발표하는 것** — ADR-023.

### 필수 (Required)

- 모든 그래프 구조는 일반 힙 릴레이션이어야 한다 (MVCC/WAL/`pg_dump` 상속) — ADR-001, 002.
- 가변 길이 경로 재작성은 **두 게이트를 모두** 통과해야 한다: 다중도 관측 불가 +
  손익분기 — ADR-013. 새 케이스는 `engine/tests/sql/05_reachability.sql`에 양방향으로
  고정한다.
- 인접 세그먼트를 읽는 컴파일 결과는 **항상 대상 노드 뷰와 조인**해야 한다 —
  `og_adj`에는 RLS가 걸리지 않기 때문 (ADR-004, spec 005).
- 헌법 원칙 이탈은 해당 `plan.md`의 *Complexity Tracking*에 (a) 위반 원칙,
  (b) 위반 없이 불가능한 이유, (c) 제거 계획을 기록해야 한다 — 헌법 Governance.
  **기록 없는 위반은 리뷰에서 반려된다.**
- 미지원 기능은 "부분은 부분"으로 명시한다 — spec 010 SC-010, ADR-022.

## ADR 작성 규칙

새 ADR은 다음 형식을 따른다. 번호는 `00_index.md`의 마지막 번호 + 1.

```markdown
# ADR-0NN: <결정 제목>

| 항목 | 값 |
|---|---|
| 상태 | Accepted / Superseded by ADR-0NN / Deprecated |
| 날짜 | YYYY-MM-DD (확인 가능한 경우만, 아니면 "미상") |
| 영향 범위 | storage, cypher, api ... |
| 근거 | README.md#..., engine/src/...:123, specs/00N/plan.md |

## 배경
## 고려한 선택지
## 결정
## 근거          ← 반드시 코드/문서 인용과 함께
## 결과          ← 긍정적 영향 / 부정적 영향·감수한 대가
## 재검토 조건    ← 형식적으로 채우지 말 것. 실제 트리거를 쓸 것

<!-- affects: ... -->
```

**날짜 규칙**: `git log`로 확인 가능한 것만 실제 날짜를 쓴다. 스펙 문서에 기록된 날짜는
그 문서의 기준일임을 함께 밝힌다. 확인 불가하면 "미상".

**정직성 규칙**: 문서는 완벽할 필요는 없지만 **거짓이어서는 안 된다.** 계획서와 구현이
어긋나 있으면(예: ADR-004의 `CHUNK_SIZE`) 그 사실을 적는다.

<!-- affects: architecture, docs -->
<!-- requires-update: docs/99_decisions/*.md -->
