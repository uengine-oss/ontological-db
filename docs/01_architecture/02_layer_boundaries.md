# 계층 경계 — 왜 이 선이 여기 있는가

> **이 문서가 답하는 질문**
> - 왜 읽기 경로는 SQL이고 쓰기 경로는 Rust인가?
> - 이 경계를 넘으면 정확히 무엇이 깨지는가?
> - 어떤 코드를 어디에 넣으면 안 되는가? (그리고 그 규칙의 근거는?)

---

## 경계 목록

이 시스템에는 다섯 개의 **의도적인** 경계가 있다.

| # | 경계 | 한쪽 | 다른 쪽 | 해결한 원칙 충돌 |
|---|---|---|---|---|
| B-1 | 읽기/쓰기 | 컴파일된 SQL | Rust SPI 절차 | 원칙 II(옵티마이저 가시성) vs 원칙 IX(ACID) |
| B-2 | 코어/어댑터 | `cypher`, `catalog`, `storage` | `adapters`, `compat`, `typeql` | 원칙 VI |
| B-3 | 확장/게이트웨이 | `ontological.so` (프로세스 내) | `ontological-bolt` (별도 프로세스) | 원칙 VI + SPI 스레드 안전성 |
| B-4 | 컴파일 타임/런타임 | 라벨 해소, 타입 캐스팅 | 힙 스캔, 조인 | 원칙 IV(상속 상수 시간) |
| B-5 | 힙/백엔드 로컬 메모리 | `og_adj` + MVCC + RLS | 컴파일된 CSR (스냅샷 동결) | 원칙 IX vs 성능 |

---

## B-1. 읽기는 SQL, 쓰기는 Rust

### 규칙

> **읽기 경로 코드는 `storage/` 에 없다.**
> Cypher 컴파일러가 `og_data.og_adj` 를 직접 건드리는 SQL을 뱉는다.

```rust
//! Read paths are deliberately **not** here. The Cypher compiler emits SQL that
//! touches `og_data.og_adj` directly so the PostgreSQL planner sees the whole
//! traversal — that is the difference between this design and a function-call
//! pipeline the optimiser cannot look into.
```
— [`engine/src/storage/mod.rs:7-10`](../../engine/src/storage/mod.rs)

### 왜 읽기가 SQL이어야 하는가

Apache AGE의 실패는 "Cypher를 문자열로 받는다"가 아니라
**"프로퍼티가 `agtype` 이라 컬럼 통계와 일반 인덱스가 없다"** 에 가깝다
([`docs/comparison.md`](../comparison.md) 의 정정 참조).
이 프로젝트는 컴파일 결과가 **평범한 SQL** 이기 때문에:

- `n1.p_born > 1960` 이 인덱스 가능한 실 컬럼 술어다
- 조인 순서를 플래너가 **실제 통계로** 고른다
- 병렬 질의, 커서, `EXPLAIN`, 플랜 캐시가 그대로 적용된다

같은 이유로 `engine/sql/access.sql` 은 **전부 `LANGUAGE sql`** 이다:

```sql
-- Everything here is LANGUAGE SQL on purpose: PostgreSQL inlines simple
-- set-returning SQL functions into the calling query, so the planner sees the
-- adjacency scan itself — statistics, join order, parallelism and all.  A
-- PL/pgSQL or C set-returning function would be an optimisation barrier, which
-- is precisely the mistake Constitution principle II forbids.
```
— [`engine/sql/access.sql:4-8`](../../engine/sql/access.sql)

**예외 3개**가 `access.sql` 에 있고, 모두 정당하다:
`og_node_json`, `og_edge_json`(스칼라 반환, 동적 테이블명 때문에 `EXECUTE` 필요),
`og_capture_history`(트리거). 셋 다 집합 반환 함수가 아니므로 인라인 대상이 아니다.

### 왜 쓰기가 Rust여야 하는가

한 엣지 생성은 **세 구조를 한 트랜잭션 안에서** 맞춰야 한다 (spec 001 FR-012):

```rust
// engine/src/storage/mod.rs:429-446
Spi::run_with_args("INSERT INTO og_data.og_edge (id, type_id, src, dst) …")  // 1. 레지스트리
Spi::run_with_args(&sql, …)                                                   // 2. 타입 테이블
adjacency::append(src, tid, 'o', dst, eid);                                   // 3a. 정방향 인접
adjacency::append(dst, tid, 'i', src, eid);                                   // 3b. 역방향 인접
```

이걸 단일 SQL 문장으로 표현하려면 트리거나 CTE 부작용에 의존해야 하고,
그러면 검증이 어려워진다. **정확성이 단일 문장 계획보다 우선한다**
([`specs/003-cypher-query-engine/plan.md`](../../specs/003-cypher-query-engine/plan.md) 설계 결정 3).

### 이 경계가 만드는 위험 (Facts)

**쓰기 경로의 SQL 생성은 문자열 조립이다.** 다만 사용자 값은 절대 보간되지 않는다:

```rust
// engine/src/storage/mod.rs:42-46 (주석)
/// All values are extracted from ONE bound jsonb parameter, so no
/// user value is ever interpolated into SQL text (spec 003 FR-026).
```

컬럼 이름은 `column_name()` 이 결정적으로 생성하고
([`catalog/types.rs:46-66`](../../engine/src/catalog/types.rs)),
테이블 이름은 `og_data.n_<int>` 형태다. 값은 `$2` 로 바인딩된다.

**그러나 읽기와 쓰기가 같은 불변식을 서로 다른 언어로 가정한다.**
읽기 SQL은 "`og_adj` 에 정방향/역방향이 모두 있다"를 가정하고,
그 보장은 오직 `storage/mod.rs` 의 Rust 코드에만 있다.
→ [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-08**

---

## B-2. 코어와 어댑터

### 규칙 (헌법 원칙 VI)

> 저장 엔진과 타입 시스템은 하나뿐이다.
> 어댑터는 코어 의미론을 바꿀 수 없다. 어댑터에만 필요한 기능은 어댑터 안에 머문다.

### 실제 의존 방향 (Facts)

`grep -rn "crate::compat" engine/src/` 로 확인한 결과:

```
engine/src/cypher/compile.rs:421:  use crate::compat::procs;
engine/src/cypher/mod.rs:165:      return crate::compat::ddl::run(graph, stmt, params);
```

그리고 반대 방향:

```
engine/src/compat/ddl.rs:10-11:    use crate::cypher::ast::*;  use crate::cypher::eval;
engine/src/compat/procs.rs:47:     crate::cypher::compile::sql_str(s)
engine/src/compat/procs.rs:226:    crate::cypher::views::ensure_view(tid, false)
```

→ **양방향 의존이다.** Cypher 코어가 Neo4j 호환면을 알고 있고,
호환면이 Cypher 내부(`ast`, `eval`, `compile::sql_str`, `views`)를 알고 있다.

또한 `compile.rs:1545` 는 `genai.vector.encode` 라는 **Neo4j 고유 함수명을
코어 컴파일러의 함수 디스패치 테이블에 하드코딩**하고 있다.

이것은 원칙 VI("어댑터는 엣지에")와 어긋난다.
[`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-06** 에 분리 제안을 적었다.

### 잘 지켜진 곳

- **`adapters/rdf.rs`**: 매핑되지 않는 트리플을 `og_data.og_triple_overflow` 에 원문 보존한다.
  그 테이블은 **두 번째 질의 경로가 아니다** — 매핑 리포트와 직렬화기만 읽는다
  ([`bootstrap.sql:346-349`](../../engine/sql/bootstrap.sql)).
- **`typeql/`**: `crate::cypher` 를 전혀 참조하지 않는다. 카탈로그와 스토리지만 공유한다.
- **`bolt/`**: 별도 크레이트라 컴파일 타임에 코어와 결합될 수 없다.

---

## B-3. 확장과 게이트웨이

### 규칙

> Bolt 게이트웨이는 파서도, 플래너도, 캐시도, 사용자 저장소도 갖지 않는다.
> Cypher 한 글자도 게이트웨이에서 해석되지 않는다.

— [`bolt/src/main.rs:4-5`](../../bolt/src/main.rs), [`bolt/README.md`](../../bolt/README.md)

### 왜 배경 워커가 아닌가

배경 워커로 확장 안에 넣으면 "확장 하나면 끝"이라는 이야기에는 맞지만,
**SPI는 스레드 안전하지 않아서 세션 간 질의가 직렬화된다.**
그 대가로 얻는 것이 설치 편의뿐이라면 잘못된 거래다
([`specs/011-bolt-protocol-gateway/plan.md`](../../specs/011-bolt-protocol-gateway/plan.md) 설계 결정 1).

### 이 경계가 강제하는 것 (Facts)

| 항목 | 결과 | 근거 |
|---|---|---|
| 인증 | `HELLO` 자격 증명으로 PostgreSQL 접속. 접속 실패 = 인증 실패 | [`session.rs:168-184`](../../bolt/src/session.rs) |
| 트랜잭션 | Bolt `BEGIN`/`COMMIT` → 세션 연결의 PostgreSQL `BEGIN`/`COMMIT` (1:1) | [`session.rs:208-219`](../../bolt/src/session.rs) |
| 동시성 | 연결당 스레드 1개, 세션당 PG 연결 1개 → 한계는 PostgreSQL `max_connections` | [`main.rs:69-79`](../../bolt/src/main.rs) |
| 읽기/쓰기 판정 | `og_cypher_check()` 에 물어본다. 재구현하지 않는다 | [`session.rs:262-266`](../../bolt/src/session.rs) |
| 컬럼 순서 | `og_cypher_columns()` 에 물어본다. jsonb가 키를 정렬하므로 행에서 복원 불가 | [`session.rs:281-289`](../../bolt/src/session.rs) |

### 이 경계의 대가 (Facts)

- **평문 두 구간**: 드라이버↔게이트웨이 TLS 미종단, 게이트웨이↔PostgreSQL `NoTls`
  ([`session.rs:182`](../../bolt/src/session.rs)).
- **왕복 2회**: `RUN` 하나가 `og_cypher_columns` + `og_cypher` 두 문장이다.
- **스트리밍 아님**: `pg.query()` 로 전체 결과를 게이트웨이 메모리에 모은 뒤
  `PULL n` 을 그 `Vec` 에서 서빙한다 ([`session.rs:291-322`](../../bolt/src/session.rs)).
- **연결 풀 없음**: 세션 하나가 PG 연결 하나를 점유한다.

→ **ARCH-07**

---

## B-4. 컴파일 타임과 런타임

### 규칙

> 라벨은 컴파일 타임에 사라진다.
> 실행 시점에 계층 판정 비용은 0이어야 한다 (헌법 원칙 IV).

### 컴파일 타임에 결정되는 것 (Facts)

| 결정 | 언제 | 근거 |
|---|---|---|
| 라벨 → 구체 테이블 유니온 뷰 | 컴파일 | [`compile.rs:770-775`](../../engine/src/cypher/compile.rs), [`views.rs:93-138`](../../engine/src/cypher/views.rs) |
| 관계 타입 → 서브타입 id 배열 리터럴 | 컴파일 | [`compile.rs:826-849`](../../engine/src/cypher/compile.rs) |
| 프로퍼티 → 실 컬럼 or `__ext->>` | 컴파일 | [`compile.rs:976-993`](../../engine/src/cypher/compile.rs) |
| 파라미터 캐스팅 타입 | 컴파일 (타입 힌트 전파) | [`compile.rs:1336-1368, 1397-1413`](../../engine/src/cypher/compile.rs) |
| `og_vlp` vs `og_reach` | 컴파일 (플래너 통계 조회) | [`compile.rs:865-873`](../../engine/src/cypher/compile.rs) |

### 이 경계가 만드는 결합

**컴파일 타임 결정이 카탈로그 상태에 의존하므로, 컴파일 결과는 카탈로그 버전에 묶인다.**
그런데 `PLAN_CACHE` 의 키는 `(graph, query)` 뿐이고 스키마 버전이 없다
([`cypher/mod.rs:29`](../../engine/src/cypher/mod.rs)).
그리고 `bump_schema_version()` 은 `drop_all_views()` 를 부르지만 `PLAN_CACHE` 를 건드리지 않는다
([`labeling.rs:172-182`](../../engine/src/catalog/labeling.rs)).
→ **ARCH-02**

**더 나아가, 컴파일이 DDL(`CREATE OR REPLACE VIEW`)을 실행한다.**
`og_cypher_sql` 은 `#[pg_extern(stable)]` 로 선언되어 있는데
([`cypher/mod.rs:74`](../../engine/src/cypher/mod.rs)),
그 안에서 `views::ensure_view()` → `Spi::run("CREATE OR REPLACE VIEW …")` 가 일어난다
([`views.rs:135`](../../engine/src/cypher/views.rs)).
→ **ARCH-03**

---

## B-5. 힙과 백엔드 로컬 메모리

### 규칙

> 백엔드 로컬 CSR로는 **자동 라우팅하지 않는다.**

```rust
//! * [`og_csr_build`] / [`og_csr_reach`] — the pgGraph bet. … Faster, and it
//!   gives up exactly what leaving the heap gives up: the snapshot is frozen at
//!   build time and RLS is never consulted.
```
— [`engine/src/storage/traverse.rs:19-23`](../../engine/src/storage/traverse.rs)

### 경계의 양쪽 (Facts)

| | `og_reach` (힙) | `og_csr_reach` (백엔드 로컬) |
|---|---|---|
| MVCC | ✅ 적용 | ❌ 빌드 시점 스냅샷 동결 |
| RLS | ✅ 적용 | ❌ 전혀 참조하지 않음 |
| 미커밋 자기 쓰기 | ✅ 보임 | ❌ 안 보임 |
| SPI | 워크 전체에 연결 1개 + 준비 문장 1개 | 없음 |
| 6홉 (50k/975k) | 67~71 ms | 4.86~4.9 ms |
| 빌드 비용 | 없음 | 백엔드마다 8.4~9.2 MiB, 119~229 ms |

근거: [`traverse.rs:99-106, 205-210`](../../engine/src/storage/traverse.rs),
[`docs/deep-traversal.md`](../deep-traversal.md).

**자동 라우팅하지 않는 이유**: 헌법 원칙 IX는 캐시 계층을 허용하되
"캐시는 진실의 원천이 될 수 없다"고 못박는다. 스냅샷이 동결된 구조를
Cypher가 자동으로 고르면, 같은 트랜잭션 안에서 방금 쓴 엣지가 보이지 않게 된다.

---

## Decisions

| # | 결정 | 대안이 기각된 이유 |
|---|---|---|
| D-1 | 읽기 경로를 `storage/` 에 두지 않는다 | Rust 헬퍼는 옵티마이저 장벽이 되어 원칙 II의 실질을 잃는다 |
| D-2 | `access.sql` 은 전부 `LANGUAGE sql` | PL/pgSQL / C 집합 반환 함수는 인라인되지 않는다 |
| D-3 | 쓰기는 절차적 Rust | 단일 SQL로 표현하면 트리거·CTE 부작용 의존이 되어 검증이 어렵다 |
| D-4 | Bolt는 별도 프로세스 | 배경 워커는 SPI 스레드 비안전성 때문에 세션을 직렬화한다 |
| D-5 | CSR 경로는 옵트인 | 원칙 IX. 스냅샷 동결을 기본값으로 두지 않는다 |
| D-6 | 사용자 값은 SQL 텍스트로 절대 보간하지 않는다 | 인덱스 유지 + 주입 구조적 불가 (spec 003 FR-026) |

---

## Forbidden / Required

**Forbidden**

- ❌ `engine/src/storage/` 에 읽기 경로 헬퍼(집합 반환 함수 등)를 추가하는 것.
- ❌ `engine/sql/access.sql` 에 `LANGUAGE plpgsql` **집합 반환** 함수를 추가하는 것.
- ❌ Cypher/TypeQL 컴파일러에서 사용자 값을 SQL 문자열로 보간하는 것.
  값은 반드시 `$1` jsonb 파라미터를 거쳐야 한다 ([`compile.rs:17-18`](../../engine/src/cypher/compile.rs)).
- ❌ Bolt 게이트웨이에 Cypher 파싱·플랜 캐시·사용자 저장소를 추가하는 것.
- ❌ Studio나 PostgREST 경로에 코어 의미론을 추가하는 것 (헌법 원칙 VI).
- ❌ 상속 판정을 런타임 재귀 CTE로 하는 것 (헌법 원칙 IV 안티패턴).
- ❌ 벡터 검색 후 그래프 조건을 사후 필터링하는 것 (헌법 원칙 V 안티패턴).
- ❌ 인접 구조를 비동기로 갱신하는 것 (헌법 원칙 IX 안티패턴).
- ❌ 백엔드 로컬 CSR을 Cypher가 자동으로 선택하게 만드는 것.

**Required**

- ✅ 모든 그래프 변경(노드/엣지/인접/카탈로그/인덱스)은 호출자 트랜잭션 안에서 일어날 것.
- ✅ 컴파일 타임 결정이 카탈로그를 읽으면, 그 결정을 캐시할 때 **스키마 버전을 키에 포함**할 것.
- ✅ 새 SQL 노출 API에는 RLS/권한 테스트를 함께 추가할 것 (헌법 품질 게이트).
- ✅ 경계를 넘는 새 의존(`crate::X` 참조)을 추가하면 이 문서의 의존 방향 절을 갱신할 것.

<!-- affects: architecture, backend, api, security -->
<!-- requires-update: 01_architecture/03_query_pipeline.md, 01_architecture/08_improvements_architecture.md, 03_backend/, 07_security/ -->
