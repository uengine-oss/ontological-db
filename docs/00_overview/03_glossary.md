# 용어집

> **이 문서가 답하는 질문**
> - 이 저장소에서 쓰는 용어는 **정확히** 무엇을 가리키는가?
> - 같은 단어가 Neo4j / TypeDB / PostgreSQL 에서 뜻하는 것과 어떻게 다른가?
> - 어떤 식별자가 코드의 어느 파일에서 정의되는가?

> **LLM에게**: 이 문서에 없는 용어를 이 프로젝트의 용어로 사용하지 말 것.
> 각 항목의 "근거" 열에 있는 파일을 읽지 않고 정의를 추측하지 말 것.

---

## 1. 저장 구조

### CSR 세그먼트 (adjacency segment)

`og_data.og_adj` 테이블의 한 행. **한 노드**의, **한 관계 타입**의, **한 방향**의
이웃을 최대 `CHUNK`(256)개까지 두 개의 정렬된 `int8[]`(`nbr`, `eid`)에 담는다.

- 256 × 8B × 2 = 4KB → 8KB 힙 페이지 하나 안에 들어간다.
- `nbr`, `eid` 컬럼은 `STORAGE MAIN` 으로 TOAST를 막는다 — TOAST되면 방금 산 지역성을 잃는다.
- 기본키 `(src, etype, dir, seq)`. `(etype, dir)` 분할로 타입/방향 가지치기가 공짜,
  `seq` 분할로 슈퍼노드를 materialise 없이 스트리밍한다.

근거: [`engine/sql/bootstrap.sql:186-214`](../../engine/sql/bootstrap.sql),
[`engine/src/storage/adjacency.rs:1-15`](../../engine/src/storage/adjacency.rs)

> **주의**: "CSR"이라는 이름은 구조의 **성격**(연속 배열 인접)을 가리킨다.
> 진짜 압축 희소 행 표현(offset 배열 + 이웃 배열)은
> `og_csr_build()` 가 만드는 **백엔드 로컬 CSR** 쪽이다. 둘은 다른 것이다.

### 백엔드 로컬 CSR (compiled CSR)

`og_csr_build()` 가 만드는, PostgreSQL 백엔드 프로세스의 Rust 힙에 사는 압축 위상 구조.
`Csr { ids: Vec<i64>, fwd: Adj, rev: Adj }`, `Adj { offs: Vec<u32>, nbrs: Vec<u32> }`.

- 노드 id는 64비트 희소값 → 정렬된 `ids` 벡터의 `u32` 위치로 밀집화.
- `thread_local!` 에 보관 — PostgreSQL 메모리 컨텍스트를 쓰면 트랜잭션 끝에 해제되므로.
- **스냅샷이 빌드 시점에 얼어붙고 RLS를 전혀 참조하지 않는다.** 자동 라우팅되지 않는 이유.

근거: [`engine/src/storage/traverse.rs:163-221, 294-334`](../../engine/src/storage/traverse.rs)

### 타입 스토리지 테이블

타입마다 하나씩 생성되는 실제 PostgreSQL 테이블.

| 접두사 | 무엇 | 예 |
|---|---|---|
| `og_data.n_<type_id>` | 엔티티 타입 | `og_data.n_4 (id int8 PK, p_model text, __ext jsonb)` |
| `og_data.e_<type_id>` | 관계 타입 | `og_data.e_7 (id int8 PK, src int8, dst int8, …, __ext jsonb)` |
| `og_data.a_<type_id>` | TypeQL 속성 타입 | `val` 컬럼을 갖는다 |

추상 타입은 스토리지 테이블이 없다 (`og_catalog.type.storage_table IS NULL`).

근거: [`engine/src/catalog/types.rs:68-74, 414-417`](../../engine/src/catalog/types.rs),
[`engine/src/typeql/schema.rs:49`](../../engine/src/typeql/schema.rs)

### `__ext`

모든 타입 스토리지 테이블에 존재하는 `jsonb` 컬럼.
**선언되지 않은(ad-hoc) 프로퍼티**가 떨어지는 곳.

- 인덱스 없음, 통계 없음, `->>` 로 읽으면 text가 된다.
- 스키마리스 사용을 가능하게 하되 빠른 경로를 주지 않음으로써 선언을 유도하는 것이 의도다.
- Cypher 쓰기 경로는 새 프로퍼티를 실컬럼으로 **승격**시키므로, Cypher만 쓰는 앱에서
  `__ext` 는 스칼라가 아닌 값(배열/객체)만 남는 경향이 있다.

근거: [`engine/sql/bootstrap.sql:82-85`](../../engine/sql/bootstrap.sql),
[`engine/src/storage/mod.rs:53-60, 228-240`](../../engine/src/storage/mod.rs)

### 프로퍼티 승격 (property promotion)

Cypher 앱은 스키마를 선언하지 않는다(Neo4j에 선언할 스키마가 없으므로).
그래서 쓰기 시점에 `declare_new_props()` 가 처음 보는 스칼라 프로퍼티를
`og_add_property()` 로 **실컬럼으로 만든다.**

근거: [`engine/src/storage/mod.rs:76-158`](../../engine/src/storage/mod.rs)

### 확장(widening)

승격된 컬럼에 나중에 다른 타입의 값이 쓰이면, 컬럼을 **`text` 로 단방향 확장**한다.
Neo4j는 같은 프로퍼티가 노드마다 다른 타입을 가질 수 있고, `text` 는 그것을 모두 표현할 수
있는 유일한 컬럼 타입이며, 단방향이므로 진동하지 않는다.

**확장 대상은 `WIDENABLE = ["bool", "int8", "float8"]` 뿐이다.**
`vector(1536)` 이나 `timestamptz` 처럼 의도적으로 선언된 타입은 확장하지 않는다
(2026-08-16에 이 가드 없이 벡터 스위트가 깨진 이력이 주석에 기록되어 있다).

근거: [`engine/src/storage/mod.rs:62-73, 121-153`](../../engine/src/storage/mod.rs)

### 식별자 인코딩

```text
 bit 63       54            36                               0
 +---+---------+-------------+--------------------------------+
 | 0 | shard:9 | type_id:18  |          local_id:36           |
 +---+---------+-------------+--------------------------------+
```

- 노드의 타입은 **시프트 + 마스크**이지 카탈로그 조인이 아니다.
- `shard` 9비트는 spec 007을 위해 예약되어 있으나 **현재 항상 0** 이 쓰인다
  (`id::make_id(0, type_id, local)`).
- `type_id` 18비트 → 최대 262,143 타입 (`og_catalog.type_id_seq MAXVALUE 262143`).
- `local_id` 36비트 → 타입당 최대 약 687억 인스턴스.

근거: [`engine/src/id.rs:1-29`](../../engine/src/id.rs),
[`engine/src/storage/mod.rs:24-34`](../../engine/src/storage/mod.rs),
[`engine/sql/bootstrap.sql:47`](../../engine/sql/bootstrap.sql)

---

## 2. 타입 시스템

### 엔티티 타입 / 관계 타입 / 속성 타입

`og_catalog.type.kind` 컬럼의 세 값.

| `kind` | 의미 | Cypher에서 | TypeQL에서 |
|---|---|---|---|
| `'e'` | entity | 노드 라벨 | `entity` |
| `'r'` | relation | 관계 타입 | `relation` |
| `'a'` | attribute | (TypeQL 전용) | `attribute` |

근거: [`engine/sql/bootstrap.sql:35-46`](../../engine/sql/bootstrap.sql)

### 구간 라벨 (interval label / nested-set label)

`og_catalog.type_label` 의 `(lft, rgt, depth)`.
상속 판정을 **인덱스 범위 비교 한 번**으로 만드는 장치.

```text
X ⊑ Y  ⟺  Y.lft ≤ X.lft AND X.rgt ≤ Y.rgt
```

- 라벨은 `GAP = 1024` 간격으로 배분되어, 계층 중간에 타입을 끼워 넣어도 대개
  빈 공간을 소비할 뿐 전체를 재번호하지 않는다.
- **다중 상속**: 루트에서 오는 경로마다 라벨 행이 하나씩 생긴다 (`path_id` 0..n).
  그래서 다중 상속에서도 판정이 여전히 범위 비교다.
- 이 인덱스가 `type_label_range_idx (graph_id, lft, rgt)` 다.

근거: [`engine/src/catalog/labeling.rs:1-30, 73-110`](../../engine/src/catalog/labeling.rs),
[`engine/sql/bootstrap.sql:59-80`](../../engine/sql/bootstrap.sql)

### 재라벨링 (`og_relabel`)

계층 구조가 바뀌면 `relabel_graph(graph_id)` 가 그래프 전체의 구간 라벨을 다시 계산한다.
`DELETE FROM og_catalog.type_label WHERE graph_id = $1` 후 DFS로 재배분.
사이클을 발견하면 `error!` 로 중단한다.

근거: [`engine/src/catalog/labeling.rs:112-170, 246-250`](../../engine/src/catalog/labeling.rs)

### 역할 (role) / 롤 플레이어 (role player)

관계 타입의 **이름 있는 참여자 슬롯**. TypeDB에서 온 개념이며,
Neo4j의 문자열 타입 관계가 표현할 수 없는 지점이다.

```sql
SELECT og_add_role('kb', 'ACTED_IN', 'actor',      'Person', 0);
SELECT og_add_role('kb', 'ACTED_IN', 'production', 'Work',   1);
```

- `ordinal`: `0` = src, `1` = dst, `2..` = n-ary.
- `player_type_id`: 이 슬롯에 들어갈 수 있는 타입. 위반 시 엣지 생성이 거부된다.
- **n-ary 롤 플레이어**는 `og_data.og_role_player (edge_id, role_id, player_id)` 에 저장된다.
- **역할 특수화(role specialisation)**: `parent_role_id` 로 상위 역할을 지정하면,
  상위 역할로 매칭할 때 그것을 특수화한 모든 역할에 도달해야 한다 (spec 010 FR-009).

근거: [`engine/sql/bootstrap.sql:103-131`](../../engine/sql/bootstrap.sql),
[`engine/src/storage/mod.rs:454-484, 530-559`](../../engine/src/storage/mod.rs)

### 별칭 뷰 (alias view)

`og_data."TypeName"` — 타입 이름 그대로 붙는 사람이 읽을 수 있는 뷰.
`CREATE VIEW og_data."MeetingRoom" AS SELECT * FROM og_data.n_12` 형태.
psql이나 BI 도구가 이 데이터베이스를 가리켰을 때 바로 보이게 하는 편의 장치다.

**절대 치명적이지 않다**: 이름이 이미 있는 무언가와 충돌해도 타입 생성은 계속된다
(로그만 남긴다).

근거: [`engine/src/catalog/types.rs:76-106`](../../engine/src/catalog/types.rs)

### 타입 유니온 뷰 (`v_<tid>` / `ve_<tid>`)

별칭 뷰와 **다른 것**이다. `MATCH (v:Vehicle)` 이 Vehicle·Car·EV를 프로퍼티 컬럼과 함께
한 릴레이션으로 보게 하기 위해, 구체 서브타입 테이블들의 `UNION ALL` 로 만든 생성 뷰.

- 노드: `og_data.v_<tid>`, 엣지: `og_data.ve_<tid>`.
- 후손 집합은 **컴파일 타임에** 구간 인덱스 범위 스캔 한 번으로 얻는다.
- 스키마가 바뀌면 `drop_all_views()` 가 `v\_%` / `ve\_%` 를 전부 `DROP VIEW ... CASCADE` 한다.

근거: [`engine/src/cypher/views.rs:1-17, 91-138, 158-177`](../../engine/src/cypher/views.rs)

---

## 3. 질의 실행

### 트레일 의미론 (trail semantics)

가변 길이 매치에서 **한 경로 안에서 같은 엣지를 두 번 지나지 않는다**는 규칙.
사이클이 무한 확장되는 것을 막는 것이 이 규칙의 역할이다.
`og_vlp()` 가 `path int8[]` 배열에 지나온 엣지를 담고 `NOT (u.eid = ANY (w.path))` 로 강제한다.

근거: [`engine/sql/access.sql:128-156`](../../engine/sql/access.sql)

### 동형 매칭 (isomorphic matching)

Cypher는 **한 MATCH 절 안에서** 같은 관계를 두 번 순회하지 못한다.
`(a)-[:ACTED_IN]->(m)<-[:ACTED_IN]-(b)` 가 `a` 를 자기 자신의 공연 상대로 돌려주지 않는 이유다.
컴파일러는 절 단위로 `rel_ids` 를 유지하며 `{u}.eid <> {other}` 술어를 붙인다.
가변 길이 홉은 제외된다 — `og_vlp` 가 이미 트레일을 걷는다.

근거: [`engine/src/cypher/compile.rs:277-286, 908-914`](../../engine/src/cypher/compile.rs)

### 방문집합 BFS (visited-set BFS) — `og_reach`

트레일 열거 대신 **레벨 동기 BFS + 방문집합**. 각 노드를 처음 도달한 깊이에 한 번만 낸다.
행 수가 걸음 수가 아니라 도달 가능한 노드 수로 제한된다.

- SPI 연결과 준비된 문장을 **전체 워크에 하나씩만** 쓴다. 레벨마다 연결/재계획하면
  지름이 큰 그래프(체인 100,000홉)에서 재귀 CTE보다 10배 느려졌던 측정 이력이 있다.
- 힙을 읽으므로 MVCC, RLS, 이 트랜잭션의 미커밋 쓰기가 모두 그대로 적용된다.

근거: [`engine/src/storage/traverse.rs:66-161`](../../engine/src/storage/traverse.rs)

### 다중도 무관 (multiplicity-blind)

질의가 "몇 개의 **경로**가 노드에 도달하는가"를 관측할 수 **없을** 때만
`og_vlp` → `og_reach` 재작성이 허용된다. 판정은 의도적으로 좁다:

- `WITH` 가 하나라도 있으면 실격 (그 안쪽을 이 패스가 보지 않으므로).
- `RETURN DISTINCT …` 는 통과 (중복 행이 살아남을 수 없으므로).
- 그 외에는 프로젝션이 집계를 포함해야 하고, 모든 집계가 중복에 둔감해야 한다:
  `count(DISTINCT x)` 는 둔감, `count(x)` 는 민감, `min`/`max` 는 인자에 따름.
- 경로 변수나 관계 변수를 바인딩하는 패턴은 홉 단계에서 따로 거부된다.

근거: [`engine/src/cypher/compile.rs:80-100, 318-349`](../../engine/src/cypher/compile.rs)

### 손익분기 깊이 (`prefer_reachability`)

`og_reach` 는 Rust 집합 반환 함수라 인라인되지 않고 레벨마다 SPI 셋업 비용을 낸다.
그래서 **예상 걸음 수 `Σ degreeⁱ` 가 512를 넘을 때**만 전환한다.
`degree` 는 `pg_class.reltuples` 에서 읽는 카탈로그 조회이며 스캔이 아니다.
통계가 없는 데이터베이스는 깊이만으로 판단한다 (`DEEP = 4`).

근거: [`engine/src/cypher/compile.rs:20-78`](../../engine/src/cypher/compile.rs)

### 컴파일 캐시 (PLAN_CACHE)

`(graph, query)` → `(SQL, columns)` 의 `thread_local!` HashMap.
512개를 넘으면 통째로 비운다. 파싱·컴파일이 반복 비용의 대부분이고,
결과 SQL의 **플랜** 캐싱은 PostgreSQL이 담당한다.

> **주의**: 스키마 버전이 캐시 키에 없다.
> [`../01_architecture/08_improvements_architecture.md`](../01_architecture/08_improvements_architecture.md) ARCH-02 참조.

근거: [`engine/src/cypher/mod.rs:26-67`](../../engine/src/cypher/mod.rs)

---

## 4. 벡터 검색

### 하이브리드 RRF (Reciprocal Rank Fusion)

벡터 유사도 순위와 다른 순위(예: 텍스트/그래프 조건)를 순위 역수의 합으로 융합하는 방식.
`og_hybrid_search()` 가 제공한다.

근거: [`engine/src/vector/mod.rs`](../../engine/src/vector/mod.rs) (`og_hybrid_search`)

### 필터 푸시다운 (구조적)

"벡터로 top-k 뽑고 나서 그래프 조건으로 거른다"(post-filter)를 헌법 원칙 V가 금지한다.
여기서는 **임베딩이 타입 테이블의 컬럼**이기 때문에, 벡터 검색이 실행될 시점에는
라벨이 이미 구체 테이블로 해소되어 있고 그래프 술어와 ANN 인덱스가 **같은 릴레이션** 위에 있다.
post-filter가 숨을 자리가 없다.

근거: [`engine/src/vector/mod.rs:9-12`](../../engine/src/vector/mod.rs)

### 스테일 임베딩 (stale embedding)

`og_data.og_embedding_state (entity_id, prop, source_hash, embedded_at)` 가
임베딩이 무엇으로부터 계산되었는지를 기록한다. 원본이 바뀌면 조용히 낡은 벡터를 주는 대신
**stale로 드러난다** (`og_stale_embeddings`).

근거: [`engine/sql/bootstrap.sql:277-285`](../../engine/sql/bootstrap.sql)

---

## 5. 프로토콜 / 도구

### PackStream

Bolt 프로토콜의 이진 직렬화 포맷. null, bool, int(전 폭), float, string, list,
dictionary, structure를 다룬다. `Node` / `Relationship` 구조체를 인코딩한다.
`Path` 는 인코딩하지 않는다 — 경로 변수는 홉의 리스트로 도착한다.

근거: [`bolt/src/packstream.rs`](../../bolt/src/packstream.rs), [`bolt/README.md`](../../bolt/README.md)

### Bolt 게이트웨이

`bolt/` 의 **별도 Rust 바이너리** (pgrx 확장 아님).
연결 하나당 스레드 하나, 세션 하나당 PostgreSQL 연결 하나.
파서·플래너·캐시·사용자 저장소를 갖지 않는다.
`HELLO` 가 나르는 자격 증명으로 PostgreSQL에 접속하며, 접속 실패가 곧 인증 실패다.

근거: [`bolt/src/main.rs:1-15, 59-80`](../../bolt/src/main.rs),
[`bolt/src/session.rs:163-193`](../../bolt/src/session.rs)

### Studio

`portal/` 의 Node.js 백엔드 + 순수 JS 프론트. Neo4j Browser 형태의 콘솔.
질의 스트림, force-directed 그래프, 테이블/JSON 뷰, 그리고 **Cypher가 컴파일된 SQL을
보여주는 SQL 탭**. `/benchmark.html` 로 벤치마크 리포트도 서빙한다.

근거: [`portal/server/index.js:1-29, 140-309`](../../portal/server/index.js)

### pgrx

Rust로 PostgreSQL 확장을 쓰기 위한 프레임워크. 버전 `=0.19.2` 로 고정되어 있다.
`#[pg_extern]` 어트리뷰트가 Rust 함수를 SQL 함수로 노출한다.
`extension_sql_file!` 이 `bootstrap.sql`(bootstrap 단계)과 `access.sql`(finalize 단계)를 싣는다.

근거: [`engine/Cargo.toml`](../../engine/Cargo.toml), [`engine/src/lib.rs:20-24`](../../engine/src/lib.rs)

### SPI (Server Programming Interface)

확장 안에서 SQL을 실행하는 PostgreSQL의 C API. 이 저장소의 거의 모든 Rust 코드가
SPI를 통해 카탈로그와 데이터를 읽고 쓴다.

`engine/src/spiu.rs` 가 얇은 래퍼를 제공한다 — `Spi::get_one_with_args` 가
"행이 없음"을 `InvalidPosition` 오류로 만들어 "매칭 없음"과 "고장남"을 뒤섞기 때문에,
`one()` / `two()` / `one_mut()` 는 빈 결과를 `Ok(None)` 으로 돌려준다.

근거: [`engine/src/spiu.rs:1-48`](../../engine/src/spiu.rs)

### 감사 로그 (`og_audit`)

모든 `og_cypher()` / `og_typeql()` 호출이 질의 텍스트, 언어, 반환 행 수, 소요 시간,
오류 코드를 `og_data.og_audit` 에 남긴다. `principal` 은 `session_user` 기본값이다.

근거: [`engine/sql/bootstrap.sql:377-390`](../../engine/sql/bootstrap.sql),
[`engine/src/cypher/mod.rs:122-135`](../../engine/src/cypher/mod.rs)

---

## 6. TypeQL 매핑 용어

### reify (관계의 노드화)

TypeQL의 관계 인스턴스는 여기서 **노드**로 저장된다.
그래서 `og_typeql_role` 뷰의 `relation_id` 는 노드 id다.
관계가 세 개 이상의 역할을 가질 수 있다.

### `$has`

TypeQL 속성 소유를 나르는 **내부 관계 타입**의 이름.
`(owner)-[:$has]->(attribute_instance)` 형태의 엣지가 만들어진다.

```sql
-- 같은 그래프를 Cypher로
SELECT og_cypher('bookstore', $$ MATCH (b:ebook)-[:`$has`]->(t:title) RETURN t.val $$);
```

근거: [`engine/sql/access.sql:306-321`](../../engine/sql/access.sql),
[`engine/src/typeql/schema.rs:19`](../../engine/src/typeql/schema.rs)

### 값 중복 제거 (value-deduplicated attribute)

TypeQL에서 속성 인스턴스는 값으로 공유된다. 같은 `title "Dune"` 을 여러 책이 소유하면
속성 인스턴스는 하나이고 `$has` 엣지가 여럿이다.
그래서 `og_typeql_attribute` 뷰는 `(owner, attribute)` 쌍마다 한 행이다.

근거: [`engine/sql/access.sql:297-321`](../../engine/sql/access.sql)

---

## Forbidden / Required

**Forbidden**
- "CSR"이라는 단어를 수식 없이 쓰지 말 것. `og_adj` 세그먼트인지 `og_csr_build()` 의
  백엔드 로컬 CSR인지 명시할 것 — 둘은 다른 구조이고 다른 보장을 준다.
- 별칭 뷰(`og_data."Type"`)와 타입 유니온 뷰(`og_data.v_<tid>`)를 혼동하지 말 것.
- `og_reach` 와 `og_reach_sql` 을 같은 것으로 쓰지 말 것.
  전자는 Rust 방문집합 BFS, 후자는 `UNION`(ALL 아님) 재귀 CTE이며 O(k·|V|)다
  ([`engine/sql/access.sql:158-190`](../../engine/sql/access.sql)).

**Required**
- 새 식별자/개념이 코드에 도입되면 이 문서에 항목과 `파일:라인` 근거를 함께 추가할 것.
- 용어가 Neo4j/TypeDB의 동명 용어와 의미가 다르면 그 차이를 명시할 것.

<!-- affects: overview, architecture, data, api, llm -->
<!-- requires-update: 00_overview/04_repository_map.md, 01_architecture/04_storage_architecture.md, 01_architecture/05_type_system_architecture.md -->
