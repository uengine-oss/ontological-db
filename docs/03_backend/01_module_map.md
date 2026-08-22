# 모듈 지도 — 책임, 의존 방향, 호출 관계

> **이 문서가 답하는 질문**
> - `engine/src/` 아래 모듈은 몇 개이고 각자 무엇을 책임지는가?
> - 의존 방향은 어디로 흐르는가? 순환은 있는가?
> - 하나의 Cypher 질의는 어떤 함수들을 거쳐 지나가는가?
> - Rust 코드와 SQL 코드(`bootstrap.sql`, `access.sql`)의 경계는 어디인가?

---

## 1. 사실 — 크레이트 구성

저장소에는 **두 개의 Rust 크레이트**가 있다. 서로 코드를 공유하지 않는다.

| 크레이트 | 경로 | 종류 | 의존성 |
|---|---|---|---|
| `ontological` | `engine/` | `cdylib` (PostgreSQL 확장) | `pgrx =0.19.2`, `serde`, `serde_json`, `ureq` |
| `ontological-bolt` | `bolt/` | 실행 바이너리 | `postgres 0.19`, `serde_json` |

근거: `engine/Cargo.toml:1-30`, `bolt/Cargo.toml:1-14`.

확장 이름은 `ontological`이고 `requires = 'vector'`이므로 `CREATE EXTENSION ontological CASCADE`가
pgvector를 함께 설치한다 (`engine/ontological.control:1-7`).

`ontological-bolt`는 `engine`을 **라이브러리로 링크하지 않는다.** 오직 PostgreSQL 프로토콜로
`og_cypher()` / `og_cypher_check()` / `og_cypher_columns()` / `og_cypher_stats()`를 호출한다
(`bolt/src/session.rs:286,295,432,447`). 이것이 "Cypher는 여기서 해석되지 않는다"는 주장의 구조적 근거다
(`bolt/src/main.rs:4-5`).

---

## 2. 사실 — 확장 진입점

`engine/src/lib.rs:21-37`:

```rust
::pgrx::pg_module_magic!(name, version);

extension_sql_file!("../sql/bootstrap.sql", name = "bootstrap", bootstrap);
extension_sql_file!("../sql/access.sql", name = "access", finalize);
```

설치 순서는 **bootstrap SQL → Rust `#[pg_extern]` 함수들 → access SQL**이다.
`access.sql`이 `finalize`인 이유는 그 안에서 Rust 함수를 참조하기 때문이다 —
예: `ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4) ROWS 100;`
(`engine/sql/access.sql:197`)은 `og_reach`가 이미 존재해야 한다.

---

## 3. 사실 — 모듈별 책임 (실측 라인 수)

| 모듈 | 라인 | 책임 | 스펙 |
|---|---|---|---|
| `lib.rs` | 48 | 진입점, 모듈 선언, SQL 파일 로드, `ontological_version()` | — |
| `id.rs` | 112 | 64bit 식별자 인코딩 `shard(9)/type(18)/local(36)` | 001 FR-004 |
| `spiu.rs` | 48 | SPI 헬퍼 `one` / `two` / `one_mut` | — |
| `stats.rs` | 110 | Neo4j 철자의 쓰기 카운터 (백엔드-로컬) | 011 |
| `storage/mod.rs` | 559 | 노드/엣지 생성·삭제, 프로퍼티 컬럼 승격 | 001 |
| `storage/adjacency.rs` | 97 | CSR 인접 세그먼트 append / remove / degree | 001 FR-002/003 |
| `storage/traverse.rs` | 476 | `og_reach` (힙 BFS), `og_csr_*` (백엔드-로컬 CSR) | 003 |
| `storage/stats.rs` | 263 | 그래프 통계, 차수 분포, 재구성, 무결성 검사 | 001/009 |
| `catalog/types.rs` | 711 | 타입/프로퍼티/롤 선언, 스토리지 테이블 DDL, 별칭 뷰 | 002 |
| `catalog/labeling.rs` | 250 | 구간(nested-set) 라벨, `og_subtypes`/`og_supertypes`/`og_is_subtype` | 002 FR-009..014 |
| `cypher/lexer.rs` | 302 | 토크나이저 | 003 FR-001 |
| `cypher/parser.rs` | 1,177 | 재귀 하강 파서 | 003 FR-001..008 |
| `cypher/ast.rs` | 263 | AST 정의, `is_aggregate()`, `default_alias()` | 003 |
| `cypher/compile.rs` | **1,591** | ★ AST → SQL 컴파일러 | 003 FR-010..016 |
| `cypher/views.rs` | 177 | 타입별 UNION ALL 뷰 생성/폐기 | 002↔003 |
| `cypher/eval.rs` | 296 | 쓰기 절 전용 Rust 표현식 평가기 | 003 |
| `cypher/mod.rs` | 823 | 공개 함수, 플랜 캐시, 쓰기 실행, 진단 | 003/008 |
| `typeql/lexer.rs` | 349 | 하이픈 포함 라벨, 무따옴표 datetime | 010 T001 |
| `typeql/parser.rs` | 1,108 | 스테이지 파이프라인 파서 | 010 |
| `typeql/ast.rs` | 232 | 스테이지/패턴 AST | 010 T002 |
| `typeql/compile.rs` | 817 | `match` → SQL, 연결 성분 분해 | 010 T026..036 |
| `typeql/schema.rs` | 572 | `define` 5-패스 실행 | 010 T010..018 |
| `typeql/write.rs` | 688 | `insert`/`put`/`update`/`delete` | 010 T019..025 |
| `typeql/dump.rs` | 133 | 카탈로그 → TypeQL `define` 역직렬화 | 010 T047 |
| `typeql/mod.rs` | 529 | 공개 함수, 읽기 파이프라인 조립, 쓰기 파이프라인 | 010 |
| `vector/mod.rs` | 442 | 임베딩 선언, HNSW, 하이브리드 RRF | 004 |
| `interop/mod.rs` | 219 | RLS, 관계형 매핑, PostgREST 리포트 | 005 |
| `adapters/mod.rs` | 89 | RDF 진입점, prefix 등록 | 006 |
| `adapters/rdf.rs` | 883 | N-Triples/Turtle 부분집합 파싱·덤프 | 006 |
| `agent/mod.rs` | 545 | 스키마 인트로스펙션, 오류 교정, 히스토리, 감사, 역할 제한 | 008 |
| `compat/ddl.rs` | 343 | `CREATE INDEX` / `CREATE CONSTRAINT` / `DROP` | Neo4j 호환 |
| `compat/procs.rs` | 291 | `db.*` / `apoc.*` 프로시저 플래너 | Neo4j 호환 |
| `compat/meta.rs` | 284 | `apoc.meta.schema` | Neo4j 호환 |
| `compat/genai.rs` | 177 | `genai.vector.encode` — **유일한 외부 네트워크 호출** | Neo4j 호환 |

---

## 4. 사실 — 의존 방향

```
                    ┌──────────────┐
                    │    lib.rs    │
                    └──────┬───────┘
                           │ (선언만)
   ┌─────────┬─────────┬───┴────┬─────────┬─────────┬──────────┐
   ▼         ▼         ▼        ▼         ▼         ▼          ▼
 cypher   typeql   compat   vector   interop  adapters      agent
   │         │        │        │         │        │            │
   └────┬────┴────────┴────────┴─────────┴────────┴────────────┘
        ▼
   ┌────────────────────────┐
   │  storage/  ·  catalog/ │   ← 도메인 코어
   └────────┬───────────────┘
            ▼
      ┌───────────────┐
      │ spiu.rs · id.rs · stats.rs │   ← 무의존 유틸
      └───────────────┘
```

**규칙: 위에서 아래로만 부른다.** 실측된 예외는 아래 두 가지다.

1. `catalog/labeling.rs:175` → `crate::cypher::views::drop_all_views()`
   — `catalog`가 `cypher`를 역참조한다. 스키마가 바뀌면 생성된 뷰가 무효해지기 때문.
2. `catalog/types.rs:564-566` → `crate::cypher::compile::sql_str(prop)`
   — SQL 문자열 리터럴 이스케이프 유틸을 `cypher` 모듈에서 빌려 쓴다.

이 둘은 **순환 의존**이며 개선 대상이다 → [`11_improvements_code.md`](11_improvements_code.md) `CODE-12`.

`typeql`은 `cypher`를 부르지 않는다. 반대도 마찬가지다. 두 언어는 `catalog` + `storage` 위에서만 만난다
(`engine/src/typeql/mod.rs:1-6`).

---

## 5. 사실 — 한 질의의 호출 경로

### 5.1 읽기 Cypher

```
og_cypher(graph, query, params)                 cypher/mod.rs:84
 └─ stats::reset()                              cypher/mod.rs:92
 └─ parser::parse(query)                        cypher/parser.rs:22
     └─ Lexer::tokenize()                       cypher/lexer.rs:81
 └─ is_write(&ast) == false                     cypher/mod.rs:33
 └─ run_read()                                  cypher/mod.rs:137
     └─ compile_cached()                        cypher/mod.rs:47
         └─ [PLAN_CACHE 히트 시 즉시 반환]       cypher/mod.rs:49
         └─ Compiler::compile_read(&ast)        cypher/compile.rs:351
             ├─ compile_match → compile_pattern → bind_node / join_rel
             │   ├─ types::resolve_label_set()  catalog/types.rs:152
             │   │   └─ labeling::og_is_subtype() catalog/labeling.rs:233
             │   └─ views::ensure_view(tid)     cypher/views.rs:93   ← DDL 발생 가능
             └─ build_select → build_core       cypher/compile.rs:602,477
     └─ exec_json(sql, params)                  cypher/mod.rs:145
         └─ client.select(sql, None, &[JsonB(params)])   ← $1 로 바인딩
 └─ audit()                                     cypher/mod.rs:122
```

핵심: **읽기는 SQL 문장 하나로 끝난다.** 조인 순서·스캔 방식·병렬성은 전부 PostgreSQL 플래너가 정한다
(`cypher/compile.rs:3-7`).

### 5.2 쓰기 Cypher

```
og_cypher(...)                                  cypher/mod.rs:84
 └─ is_write(&ast) == true
 └─ run_write()                                 cypher/mod.rs:158
     ├─ [DDL 절이면] compat::ddl::run()          compat/ddl.rs:18
     ├─ 1단계: 선행 MATCH/UNWIND만 컴파일해 바인딩 행 생성   cypher/mod.rs:171-236
     │    └─ Compiler::build_select_pub()       cypher/compile.rs:310
     ├─ rename_label() — REMOVE n:Old SET n:New 특례    cypher/mod.rs:517
     └─ 2단계: 바인딩 행마다 쓰기 절 적용         cypher/mod.rs:251-321
          ├─ create_pattern → storage::create_node_inner / create_edge_inner
          ├─ merge_pattern  → SELECT 후 없으면 create        cypher/mod.rs:451
          ├─ apply_set      → storage::set_node_props_inner  cypher/mod.rs:576
          └─ Delete         → storage::delete_edge_inner / delete_node_inner
     └─ fold_aggregates()  — RETURN count(*) 접기  cypher/mod.rs:341
```

핵심: **쓰기는 SQL 문장 하나가 아니다.** 행마다 Rust가 SPI 호출을 반복한다.
이유는 [`02_write_path.md`](02_write_path.md) 참조.

### 5.3 TypeQL

```
og_typeql(graph, query)                         typeql/mod.rs:49
 └─ run_script → run_query                      typeql/mod.rs:134,143
     ├─ [define] schema::run_define()  (5 패스)  typeql/schema.rs:56
     ├─ [읽기] compile_read()                    typeql/mod.rs:247
     │    ├─ compile_match()                     typeql/mod.rs:186
     │    │    └─ compile::components(&pats)     typeql/compile.rs:735  ← 연결 성분 분해
     │    │    └─ Compiler::compile_patterns()   typeql/compile.rs:169
     │    └─ 스테이지마다 서브쿼리로 감싸기        typeql/mod.rs:251-354
     └─ [쓰기] run_write_pipeline()              typeql/mod.rs:447
```

---

## 6. 사실 — Rust와 SQL의 경계

| 놓인 곳 | 무엇이 | 왜 |
|---|---|---|
| `bootstrap.sql` | 스키마 2개, 테이블 20여 개, 인덱스, `pg_extension_config_dump` 등록 | 확장 설치 시점에 존재해야 하고, 백업 등록은 SQL로만 가능 |
| `access.sql` | `og_expand`, `og_vlp`, `og_reach_sql`, `og_subtype_ids`, `og_type_name` 등 | **`LANGUAGE sql`은 인라인된다** → 플래너가 순회 스캔 자체를 본다 (`access.sql:4-8`) |
| Rust (`#[pg_extern]`) | 쓰기 경로, 컴파일러, `og_reach`, CSR | 다중 구조를 잠금-스텝으로 유지하거나, 집합 자료구조가 필요한 경우 |

`access.sql`에도 예외가 둘 있다. `og_node_json` / `og_edge_json` / `og_capture_history`는
`LANGUAGE plpgsql`이다 (`access.sql:209,238,274`). 동적 `EXECUTE format(...)`으로
스토리지 테이블 이름을 런타임에 결정해야 하므로 SQL 함수로 쓸 수 없다.
그 대가로 이들은 인라인되지 않는다 — 컴파일 시점에 타입을 모르는 프로퍼티 접근이
`og_node_json(n.id)->>'x'` 로 컴파일되면 최적화 장벽이 된다
(`cypher/compile.rs:991`).

---

## 7. 결정 — 왜 이 배치인가

| 결정 | 근거 |
|---|---|
| 읽기 경로 함수를 Rust로 감싸지 않는다 | `storage/mod.rs:7-10` — "컴파일러는 `og_data.og_adj`를 직접 건드리는 SQL을 낸다. 그것이 함수 호출 파이프라인과의 차이다." |
| `access.sql`은 전부 `LANGUAGE sql` (plpgsql 3개 예외) | `access.sql:4-8` — 헌법 원칙 II가 최적화 장벽을 금지 |
| `og_reach`만 Rust | `storage/traverse.rs:12-18` — 방문집합이 필요하고, 그것은 SQL에 없다 |
| `og_vlp`는 SQL로 남긴다 | `access.sql:129-137` — LATERAL 조인으로 시작 행마다 붙어야 하므로 |
| 두 질의 언어가 서로를 부르지 않는다 | `typeql/mod.rs:1-6` — 같은 카탈로그/스토리지/트랜잭션 위의 **동등한 두 표면** |

---

## 금지 / 필수

- **금지**: `catalog/`나 `storage/`에서 `cypher::`/`typeql::`를 부르는 새 코드를 추가하는 것.
  현재 2곳(`labeling.rs:175`, `types.rs:564`)은 알려진 부채다.
- **금지**: `bolt/`에서 Cypher를 파싱하거나 쓰기 여부를 키워드로 판정하는 것.
  반드시 `og_cypher_check()`에 물어본다 (`bolt/src/session.rs:438-461`).
- **필수**: 새 공개 SQL 함수를 추가할 때는 해당 모듈의 `#[pg_extern]`으로 선언하고,
  순수 SQL로 표현 가능하면 `access.sql`에 `LANGUAGE sql`로 쓴다.
- **필수**: 새 사용자 데이터 테이블을 `bootstrap.sql`에 추가하면 반드시
  `pg_catalog.pg_extension_config_dump()`에 등록한다 (`bootstrap.sql:390-427`).
  등록하지 않으면 `pg_dump`가 내용을 건너뛰고 **조용히 빈 그래프로 복원**된다.

<!-- affects: backend, architecture -->
<!-- requires-update: 01_architecture/, 02_api/ -->
