# 저장소 지도

> **이 문서가 답하는 질문**
> - 어떤 코드가 어디에 있는가?
> - 각 모듈은 무엇을 책임지고, 무엇을 책임지지 **않는가**?
> - 어떤 파일이 크고, 그래서 어디를 조심해야 하는가?
> - 어떤 스펙 번호가 어떤 디렉터리에 대응하는가?

> 라인 수는 2026-08-22 실측값(`wc -l`)이다. 코드가 바뀌면 이 표도 바뀐다.

---

## 최상위 구조

```
ontological-db/
├─ engine/          PostgreSQL 확장 본체 (Rust / pgrx + 부트스트랩 SQL)
├─ bolt/            Bolt 4.4 게이트웨이 — 별도 Rust 바이너리 (pgrx 아님)
├─ portal/          Studio 콘솔 (Node.js 백엔드 + 순수 JS 프론트)
├─ web/             랜딩/문서 사이트 (단일 HTML 1,916줄)
├─ bench/           벤치마크 하네스 + 결과 JSON
├─ tests/           통합 테스트 (SQL 회귀, TypeQL, Neo4j Movies)
├─ examples/        데모 SQL, TypeDB bookstore, Neo4j MCP 예제
├─ docs/            문서 (영문 8편 + 이 한국어 세트)
├─ specs/           스펙 11개 (spec.md / plan.md / tasks.md)
├─ docker/          개발 이미지 (Dockerfile.dev)
├─ .specify/        거버넌스 — constitution.md 포함
└─ start.sh         PostgreSQL + Bolt 게이트웨이 동시 기동
```

---

## `engine/` — 확장 본체

```
engine/
├─ Cargo.toml               pgrx =0.19.2, serde/serde_json, ureq
├─ ontological.control      default_version = '0.1.0', requires = 'vector'
├─ sql/
│  ├─ bootstrap.sql   448   스키마 2개 + 테이블 전부 + pg_dump 등록
│  └─ access.sql      338   공개 접근 경로 (전부 LANGUAGE sql) + 뷰
├─ src/                     ← 아래 표
└─ tests/
   ├─ sql/                  SQL 회귀 5개 (catalog/cypher/vector/neo4j-compat/reachability)
   └─ pg_regress/           pg_regress setup
```

### `engine/src/` 모듈 지도

| 파일 | 줄 | 스펙 | 책임 | 책임 아님 |
|---|---:|---|---|---|
| `lib.rs` | 48 | — | 확장 진입점, 모듈 선언, 두 SQL 파일 로드, `ontological_version()` | 로직 없음 |
| `id.rs` | 112 | 001 | 64비트 식별자 인코딩/디코딩, 범위 검증 | 할당(그건 `storage`) |
| `spiu.rs` | 48 | — | SPI 얇은 래퍼. "행 없음"과 "고장남"을 분리 | 트랜잭션 제어 |
| `stats.rs` | 110 | 011 | Neo4j 철자의 쓰기 카운터 (백엔드 로컬, 호출마다 리셋) | 트랜잭션 로그 아님 |
| **catalog/** | | **002** | | |
| `catalog/types.rs` | 711 | 002 | 타입 생성/삭제, 스토리지 테이블 DDL, 프로퍼티 선언, 별칭 뷰, 라벨셋 해소 | 구간 라벨 계산 |
| `catalog/labeling.rs` | 250 | 002 | 구간(nested-set) 라벨 계산·재계산, `og_subtypes`/`og_supertypes`/`og_is_subtype` | 타입 CRUD |
| **storage/** | | **001** | | |
| `storage/mod.rs` | 559 | 001 | **쓰기 경로**: 노드/엣지 생성·삭제, id 할당, 프로퍼티 승격/확장, role 검증 | 읽기 경로(의도적으로 없음) |
| `storage/adjacency.rs` | 97 | 001 | CSR 세그먼트 append/remove, `og_degree` | 순회 |
| `storage/traverse.rs` | 476 | 001/003 | `og_reach` 방문집합 BFS, 백엔드 로컬 CSR 빌드/워크/양방향 최단거리 | 컴파일 결정 |
| `storage/stats.rs` | 263 | 001 | 그래프/차수 통계, `og_reorganize`, `og_check_integrity` | — |
| **cypher/** | | **003** | | |
| `cypher/lexer.rs` | 302 | 003 | 토크나이저 | — |
| `cypher/parser.rs` | 1,177 | 003 | 재귀 하강 파서 | 의미 검증 |
| `cypher/ast.rs` | 263 | 003 | AST 정의, 집계 판별, 기본 컬럼명 규칙 | — |
| `cypher/views.rs` | 177 | 003 | 타입 유니온 뷰(`v_`/`ve_`) 생성·전량 폐기 | 별칭 뷰 |
| **`cypher/compile.rs`** | **1,591** | 003 | **AST → SQL.** 패턴/표현식/프로젝션/OPTIONAL/WITH/CALL, 타입 힌트 캐스팅, 도달성 재작성 판단 | 쓰기 실행 |
| `cypher/eval.rs` | 296 | 003 | 쓰기 절의 표현식 평가기 (Rust 측) | 읽기 |
| `cypher/mod.rs` | 823 | 003 | 공개 SQL 함수, 컴파일 캐시, 쓰기 절차 실행, EXPLAIN, 감사, 진단 | 컴파일 |
| **typeql/** | | **010** | | |
| `typeql/lexer.rs` | 349 | 010 | 토크나이저 | — |
| `typeql/parser.rs` | 1,108 | 010 | 파서 | — |
| `typeql/ast.rs` | 232 | 010 | AST | — |
| `typeql/compile.rs` | 817 | 010 | `match`/`fetch` → SQL. isa/has/role 조인 | 쓰기 |
| `typeql/schema.rs` | 572 | 010 | `define` — 타입/속성/역할/함수 저장 | 질의 |
| `typeql/write.rs` | 688 | 010 | `insert`/`put`/`delete`/`update` | 질의 |
| `typeql/dump.rs` | 133 | 010 | 스키마 역직렬화 (`og_typeql_schema`) | — |
| `typeql/mod.rs` | 529 | 010 | 공개 SQL 함수, 파이프라인 스테이지 래핑, 감사 | — |
| **vector/** | | **004** | | |
| `vector/mod.rs` | 442 | 004 | 임베딩 선언, HNSW 인덱스, 벡터/유사/하이브리드 RRF 검색, 스테일 판정 | 저장(그건 타입 컬럼) |
| **interop/** | | **005** | | |
| `interop/mod.rs` | 219 | 005 | RLS 활성화, PostgREST용 JSON 반환, 관계형 테이블 매핑, 리포트 | 뷰 정의(그건 `access.sql`) |
| **adapters/** | | **006** | | |
| `adapters/mod.rs` | 89 | 006 | RDF/OWL 진입점, prefix, IRI 바인딩, 매핑 리포트 | 파싱 |
| `adapters/rdf.rs` | 883 | 006 | Turtle/N-Triples 파싱·덤프, OWL→타입 계층 매핑, overflow 기록 | SPARQL(미구현) |
| **agent/** | | **008** | | |
| `agent/mod.rs` | 545 | 008 | 스키마 인트로스펙션(토큰 예산), 오류 교정, dry-run 추정, 역할/한도, 히스토리, 시점 질의 | 실행 |
| **compat/** | | Neo4j 호환면 | | |
| `compat/mod.rs` | 19 | — | 모듈 선언 + 설계 의도 주석 | — |
| `compat/ddl.rs` | 343 | — | Neo4j 스타일 인덱스/제약 DDL | — |
| `compat/procs.rs` | 291 | — | `db.*` / `apoc.*` 프로시저를 네이티브 함수에 매핑 | 새 의미론 |
| `compat/meta.rs` | 284 | — | `apoc.meta.schema` | — |
| `compat/genai.rs` | 177 | — | `genai.vector.encode` — **유일한 외부 네트워크 호출** (ureq) | — |

**엔진 Rust 소스 합계: 약 12,895줄** (위 표의 합).

### `#[pg_extern]` 분포 (총 78개)

| 모듈 | 개수 |
|---|---:|
| `agent/mod.rs` | 11 |
| `vector/mod.rs` | 8 |
| `catalog/types.rs` | 8 |
| `storage/traverse.rs` | 6 |
| `storage/mod.rs` | 6 |
| `cypher/mod.rs` | 6 |
| `interop/mod.rs` | 5 |
| `adapters/mod.rs` | 5 |
| `typeql/mod.rs` | 4 |
| `storage/stats.rs` | 4 |
| `id.rs` | 4 |
| `catalog/labeling.rs` | 4 |
| `storage/adjacency.rs` | 2 |
| `compat/genai.rs` | 2 |
| `typeql/dump.rs`, `compat/meta.rs`, `lib.rs` | 각 1 |

전체 함수 목록은 [`docs/api.md`](../api.md) 를 볼 것.

---

## `bolt/` — Bolt 4.4 게이트웨이

```
bolt/
├─ README.md              지원 매트릭스 (권위 문서)
└─ src/
   ├─ main.rs        81   TcpListener, 연결당 스레드, 환경변수 설정
   ├─ session.rs    606   핸드셰이크, HELLO/RUN/PULL/BEGIN/COMMIT/…, 결과 매핑
   └─ packstream.rs 348   PackStream 인코더/디코더 + 청크 프레이밍
```

**합계 1,035줄.** pgrx 확장이 **아니다** — 평범한 Rust 바이너리이며 확장 ABI와 무관하다.
의존성은 `postgres`(동기 클라이언트)와 `serde_json`.
Bolt 서버 크레이트를 쓰지 않은 이유는 지원 매트릭스를 직접 통제하기 위해서다
([`specs/011-bolt-protocol-gateway/plan.md`](../../specs/011-bolt-protocol-gateway/plan.md)).

**책임 아님**: Cypher 파싱, 계획, 캐시, 사용자 저장소. 하나도 갖지 않는다.

---

## `portal/` — Studio

```
portal/
├─ server/index.js   376   pg Pool, HTTP 라우팅 9개 엔드포인트, 정적 파일 서빙
└─ web/
   ├─ app.js         880   질의 스트림, force-directed 그래프, 테이블/JSON/SQL 탭
   └─ benchmark.js   378   bench/results/ JSON 렌더러
```

엔드포인트: `GET /api/benchmark|health|schema|audit`,
`POST /api/cypher|explain|diagnose|expand|sql`
([`portal/server/index.js:140-309`](../../portal/server/index.js)).

> **주의**: `POST /api/sql` 은 임의 SQL을 풀의 고정 역할로 실행한다.
> 인증 계층이 없다. [`../01_architecture/06_protocol_surfaces.md`](../01_architecture/06_protocol_surfaces.md) 참조.

---

## `web/` — 랜딩 사이트

`web/index.html` 단일 파일 1,916줄. 빌드 단계가 없다.

---

## `bench/` — 벤치마크

```
bench/
├─ harness.py      1,275   AGE/Neo4j/TypeDB/CTE 비교, 정답 게이트, 무결성 검사, 회귀 비교
├─ pggraph_cost.sql         pgGraph 비용 측정
├─ csr/
│  ├─ deep.py               깊은 순회 측정
│  ├─ gen.sql, gen_shape.sql  픽스처 생성
│  ├─ cypher_ab.sql         og_vlp vs og_reach A/B
│  └─ results/
├─ results/                 baseline.json + 타임스탬프 결과 JSON
└─ README.md                방법론과 재현 방법
```

**하네스는 시스템들의 답이 서로 다르면 자기 타이밍을 무효화한다.** (헌법 원칙 X)

---

## `tests/` — 통합 테스트

| 경로 | 무엇 | 실행 |
|---|---|---|
| `tests/run.sh` | SQL 회귀 스위트 (`engine/tests/sql/*.sql` 5개) | `./tests/run.sh` |
| `tests/typeql/run.py` | TypeDB bookstore 예제 대조 (28개 속성) | `python3 tests/typeql/run.py` |
| `tests/neo4j-movies/run.py` | Neo4j 공식 Movie 샘플을 postgres/bolt/neo4j 세 경로로 대조 | `python3 tests/neo4j-movies/run.py` |

Rust 단위 테스트(파서, 렉서, RDF, 식별자)는 모듈 옆에 살고 `cargo test` 로 돈다
([`engine/src/lib.rs:42-47`](../../engine/src/lib.rs), 예: [`engine/src/id.rs:93-111`](../../engine/src/id.rs)).

---

## `specs/` — 스펙

```
specs/
├─ README.md                스펙 목록, 의존 관계, 권장 진행 순서
├─ 001-graph-storage-engine/       spec.md 251 · plan.md 167
├─ 002-ontology-type-system/       spec.md 307 · plan.md 129
├─ 003-cypher-query-engine/        spec.md 299 · plan.md 102
├─ 004-vector-hybrid-search/       spec.md 278 · plan.md  86
├─ 005-postgres-supabase-interop/  spec.md 284 · plan.md  58
├─ 006-semantic-web-adapters/      spec.md 295 · plan.md  67
├─ 007-distributed-cluster/        spec.md 300 · plan.md  44
├─ 008-agent-native-interface/     spec.md 331 · plan.md  57
├─ 009-benchmark-conformance/      spec.md 305 · plan.md  55
├─ 010-typeql-query-surface/       spec.md 346 · plan.md 114
└─ 011-bolt-protocol-gateway/      spec.md 225 · plan.md  84
```

모든 `plan.md` 는 **Constitution Check** 섹션을 갖고,
헌법 원칙 이탈은 **Complexity Tracking** 에 기록된다.
상태는 [`05_spec_status.md`](05_spec_status.md) 를 볼 것.

---

## 스펙 → 코드 대응

| 스펙 | 주 구현 위치 |
|---|---|
| 001 저장 엔진 | `engine/src/storage/`, `engine/src/id.rs`, `engine/sql/bootstrap.sql` (og_data) |
| 002 타입 시스템 | `engine/src/catalog/`, `engine/sql/bootstrap.sql` (og_catalog) |
| 003 Cypher 엔진 | `engine/src/cypher/`, `engine/sql/access.sql` (og_expand, og_vlp) |
| 004 벡터/하이브리드 | `engine/src/vector/` |
| 005 인터op | `engine/src/interop/`, `engine/sql/access.sql` (og_node_view 등) |
| 006 시맨틱 웹 | `engine/src/adapters/` |
| 007 분산 | **없음** — 읽기 복제는 코드가 필요 없고, 샤딩은 미구현 |
| 008 에이전트 | `engine/src/agent/` |
| 009 벤치마크 | `bench/` |
| 010 TypeQL | `engine/src/typeql/`, `engine/sql/access.sql` (og_typeql_* 뷰) |
| 011 Bolt | `bolt/` |
| (스펙 없음) | `engine/src/compat/` — Neo4j 호환면 |

---

## 큰 파일 경고

| 파일 | 줄 | 왜 주의해야 하는가 |
|---|---:|---|
| `web/index.html` | 1,916 | 단일 파일 랜딩 사이트. 빌드 단계 없음 |
| `engine/src/cypher/compile.rs` | **1,591** | **단일 파일 컴파일러.** 패턴/표현식/프로젝션/최적화 판단이 한 `impl Compiler` 에 모여 있다. [ARCH-04](../01_architecture/08_improvements_architecture.md) |
| `bench/harness.py` | 1,275 | 5개 시스템 어댑터 + 정답 게이트 + 리포트 |
| `engine/src/cypher/parser.rs` | 1,177 | 재귀 하강 파서 |
| `engine/src/typeql/parser.rs` | 1,108 | TypeQL 파서 |
| `engine/src/adapters/rdf.rs` | 883 | RDF 파싱/덤프 |
| `portal/web/app.js` | 880 | 프론트 전체 |

---

## Facts

- 엔진 Rust: 약 12,895줄 / Bolt: 1,035줄 / Studio: 1,634줄 / 랜딩: 1,916줄.
- 부트스트랩 + 접근 SQL: 786줄.
- 확장 의존성은 pgvector 하나 (`requires = 'vector'`).
- 외부 네트워크 호출은 `compat/genai.rs` 하나뿐이며, 기본 비활성이다.

## Decisions

- **결정**: Bolt 게이트웨이를 배경 워커가 아니라 별도 프로세스로 둔다.
  SPI는 스레드 안전하지 않아서 배경 워커로 넣으면 세션 간 질의가 직렬화된다.
  얻는 것이 설치 편의뿐이라면 잘못된 거래다
  ([`specs/011-bolt-protocol-gateway/plan.md`](../../specs/011-bolt-protocol-gateway/plan.md) 설계 결정 1).
- **결정**: 읽기 경로는 `storage/` 에 두지 않는다. Cypher 컴파일러가 `og_data.og_adj` 를
  직접 건드리는 SQL을 뱉어야 플래너가 순회 전체를 본다
  ([`engine/src/storage/mod.rs:7-10`](../../engine/src/storage/mod.rs)).

---

## Forbidden / Required

**Forbidden**
- `storage/mod.rs` 에 읽기 경로 헬퍼를 추가하지 말 것. 그것은 옵티마이저 장벽이 된다.
- `access.sql` 에 `LANGUAGE plpgsql` 집합 반환 함수를 추가하지 말 것.
  인라인되지 않아 플래너가 순회 스캔을 못 본다
  ([`engine/sql/access.sql:1-9`](../../engine/sql/access.sql)).
  (현재 `og_node_json`, `og_edge_json`, `og_capture_history` 만 plpgsql이며,
  이들은 스칼라/트리거로서 인라인 대상이 아니다.)
- Bolt 게이트웨이에 Cypher 파싱·캐시·사용자 저장소를 넣지 말 것 (spec 011 설계 결정 1, 2).

**Required**
- 새 모듈을 추가하면 이 문서의 모듈 표에 **책임과 "책임 아님"** 을 함께 적을 것.
- 파일이 1,000줄을 넘으면 [`08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md)
  에 분할 축을 제안할 것.

<!-- affects: overview, architecture, backend -->
<!-- requires-update: 00_overview/03_glossary.md, 01_architecture/02_layer_boundaries.md, 01_architecture/08_improvements_architecture.md -->
