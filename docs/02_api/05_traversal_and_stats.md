# 순회 · 통계 API

> **이 문서가 답하는 질문**
> - `og_vlp` / `og_reach_sql` / `og_reach` / `og_csr_reach`는 무엇이 다른가?
> - 백엔드 로컬 CSR은 무엇을 포기하고 무엇을 얻는가?
> - `dir` 인자에 정확히 무엇을 넣어야 하는가?
> - 그래프 통계와 무결성 검사는 무엇을 알려주는가?
> - 이 함수들의 병렬 안전성은 왜 서로 다른가?

---

## 1. 사실 — 도달성을 답하는 네 가지 구현

같은 질문("`src`에서 `k`홉 안에 닿는 노드")에 네 개의 구현이 있고, **의미론과
비용과 가시성이 각각 다르다**. 근거: [engine/src/storage/traverse.rs:1](../../engine/src/storage/traverse.rs#L1) 모듈 주석.

| 함수 | 언어 | 반환 | 행 수 상한 | 스냅숏 / RLS | 정의 |
|---|---|---|---|---|---|
| `og_vlp` | SQL (재귀 CTE) | `(node, depth, path int8[])` | **`degree^k`** — 트레일 열거 | 현재 트랜잭션 | [engine/sql/access.sql:138](../../engine/sql/access.sql#L138) |
| `og_reach_sql` | SQL (`UNION` 재귀 CTE) | `(node, depth)` | `O(k · \|V\|)` | 현재 트랜잭션 | [engine/sql/access.sql:169](../../engine/sql/access.sql#L169) |
| `og_reach` | Rust (SPI BFS + visited set) | `(node, depth)` | `O(\|V\| + \|E\|)` | 현재 트랜잭션 | [engine/src/storage/traverse.rs:80](../../engine/src/storage/traverse.rs#L80) |
| `og_csr_reach` | Rust (백엔드 로컬 CSR) | `(node, depth)` | `O(\|V\| + \|E\|)` | **빌드 시점 스냅숏, RLS 미적용** | [engine/src/storage/traverse.rs:359](../../engine/src/storage/traverse.rs#L359) |

**결정(Decision)**: 도달성에는 경로가 필요 없다. 프론티어와 방문 집합은 각 노드를
최대 한 번 건드리므로, 질문이 아무리 깊어도 작업량이 `|V| + |E|`로 묶인다.
반대로 트레일 열거는 `degree^k` 행을 만들어 평균 차수 20인 그래프에서 6홉 근처가
계산 한계다([traverse.rs:6](../../engine/src/storage/traverse.rs#L6)).

측정 수치는 [docs/deep-traversal.md](../../docs/deep-traversal.md) / [docs/benchmark.md](../../docs/benchmark.md)를 직접 확인할 것.

---

## 2. `dir` 인자 규칙 (필수)

인접 테이블 `og_data.og_adj`는 방향 문자를 저장한다.

| 값 | 의미 |
|---|---|
| `'o'` | outgoing — `src`에서 나가는 방향 |
| `'i'` | incoming — `src`로 들어오는 방향 |
| `'b'` | both — `dir IN ('o','i')` |

**타입이 함수마다 다르다.**

| 함수 | `dir` SQL 타입 | 근거 |
|---|---|---|
| `og_reach`, `og_vlp`, `og_reach_sql`, `og_expand`, `og_expand_batch` | `"char"` | [engine/sql/access.sql:197](../../engine/sql/access.sql#L197) `ALTER FUNCTION og_reach(int8, int4[], "char", int4, int4)` |
| `og_csr_build` | `text` | `default!(&str, "'o'")` — [traverse.rs:298](../../engine/src/storage/traverse.rs#L298) |
| `og_degree`, `og_degree_all` | `text` | [engine/src/storage/adjacency.rs:77](../../engine/src/storage/adjacency.rs#L77) |

따라서 캐스트를 반드시 명시할 것:

```sql
SELECT * FROM og_reach(412316860417, NULL, 'o'::"char", 1, 3);   -- "char"
SELECT * FROM og_csr_build(NULL, 'o');                            -- text
SELECT og_degree_all(412316860417, 'o');                          -- text
```

> ⚠️ 같은 개념의 인자가 세 함수군에서 `"char"`와 `text`로 갈린다
> → [12_improvements_api.md](12_improvements_api.md) **API-02**.

잘못된 값을 넣으면:
`direction must be 'o', 'i' or 'b', not '<x>'`
([traverse.rs:39](../../engine/src/storage/traverse.rs#L39), [:92](../../engine/src/storage/traverse.rs#L92)).

---

## 3. 힙 위의 순회

### `og_expand(src int8, etypes int4[], dir "char") RETURNS TABLE(nbr int8, eid int8)`

정의: [engine/sql/access.sql:14](../../engine/sql/access.sql#L14) · `LANGUAGE sql STABLE PARALLEL SAFE ROWS 50`

**무엇을 하는가**: 노드 하나의 이웃을 펼친다. 힙 튜플 1개당 이웃 최대 256개를 순차 배열 읽기로 훑는다(spec 001 FR-002).

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 시작 노드 |
| `etypes` | `int4[]` | 필수(NULL 허용) | — | 관계 타입 id 목록. `NULL`이면 전체 |
| `dir` | `"char"` | 필수 | — | `'o'` / `'i'` |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `nbr` | `int8` | 아니오 | 이웃 노드 id |
| `eid` | `int8` | 아니오 | 그 이웃으로 가는 엣지 id |

**결정(Decision)**: `LANGUAGE sql`인 이유는 PostgreSQL이 단순 집합 반환 SQL
함수를 **인라인**하기 때문이다. 그래야 플래너가 인접 스캔 자체를 보고 통계·조인
순서·병렬성을 쓴다. PL/pgSQL이나 C 집합 반환 함수는 최적화 장벽이 되고, 그것이
헌법 원칙 II가 금하는 실수다([engine/sql/access.sql:4](../../engine/sql/access.sql#L4)).

**예제**

```sql
SELECT nbr, eid FROM og_expand(412316860417, NULL, 'o'::"char");
```

**실패 조건**: 없음 — 매치가 없으면 0행.

### `og_expand_batch(srcs int8[], etypes int4[], dir "char") RETURNS TABLE(src int8, nbr int8, eid int8)`

정의: [engine/sql/access.sql:29](../../engine/sql/access.sql#L29) · `LANGUAGE sql STABLE PARALLEL SAFE ROWS 500`

`og_expand`의 다중 시작점 버전. 멀티홉 계획이 행마다 왕복하는 형태로 퇴화하지
않게 한다.

---

### `og_vlp(src int8, etypes int4[], dir "char", minhop int, maxhop int) RETURNS TABLE(node int8, depth int, path int8[])`

정의: [engine/sql/access.sql:138](../../engine/sql/access.sql#L138) · `LANGUAGE sql STABLE PARALLEL SAFE ROWS 100`

**무엇을 하는가**: 가변 길이 경로를 **트레일 의미론**으로 열거한다(spec 003 FR-004).
한 경로 안에서 같은 엣지를 반복하지 않으므로 사이클이 무한히 확장되지 않는다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 시작 노드 |
| `etypes` | `int4[]` | 필수(NULL 허용) | — | 관계 타입 id. `NULL`이면 전체 |
| `dir` | `"char"` | 필수 | — | `'o'` / `'i'` / `'b'` |
| `minhop` | `int` | 필수 | — | 최소 깊이 (포함) |
| `maxhop` | `int` | 필수 | — | 최대 깊이 (포함) |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `node` | `int8` | 아니오 | 도달한 노드 |
| `depth` | `int` | 아니오 | 이 **경로**의 홉 수 |
| `path` | `int8[]` | 아니오 | 지나온 엣지 id 배열. `depth = 0`이면 빈 배열 |

**행 수 경고**: 경로 1개당 1행이다. 평균 차수 `d`, 깊이 `k`면 대략 `d^k` 행.

**예제**

```sql
SELECT node, depth, path
  FROM og_vlp(412316860417, NULL, 'o'::"char", 1, 3)
 ORDER BY depth;
```

**실패 조건**: 없음. 다만 깊이가 크면 시간/메모리가 폭발한다.

---

### `og_reach_sql(src int8, etypes int4[], dir "char", minhop int, maxhop int) RETURNS TABLE(node int8, depth int)`

정의: [engine/sql/access.sql:169](../../engine/sql/access.sql#L169) · `LANGUAGE sql STABLE PARALLEL SAFE ROWS 1000`

**무엇을 하는가**: 경로 없는 도달성을 순수 SQL로 계산한다. 재귀 분기가 `UNION ALL`이
아니라 **`UNION`** 이라서 PostgreSQL이 워크테이블을 중복 제거한다.

**진짜 방문 집합은 아니다** — 깊이 2에서 찾은 노드가 깊이 3에서 다시 나온다
(`(node, depth)`가 다른 행이므로). 그래서 작업량은 `O(|V| + |E|)`가 아니라
`O(k · |V|)` 다([engine/sql/access.sql:164](../../engine/sql/access.sql#L164)).

**용도**: `og_reach`가 측정되는 **바닥선(floor)**. Rust 없이 얻을 수 있는 것.

**반환**: `node`, `min(depth)` (노드당 한 행).

---

### `og_reach(src int8, etypes int4[], dir "char", minhop int4, maxhop int4) RETURNS TABLE(node int8, depth int4)`

정의: [engine/src/storage/traverse.rs:80](../../engine/src/storage/traverse.rs#L80) ·
휘발성: `STABLE` · 병렬: `PARALLEL RESTRICTED` · `ROWS 100` ([engine/sql/access.sql:197](../../engine/sql/access.sql#L197))

**무엇을 하는가**: 방문 집합을 쓰는 레벨 동기 BFS. 각 노드를 **처음 도달한 깊이에 한 번만** 보고한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 시작 노드 |
| `etypes` | `int4[]` | 필수(NULL 허용) | — | 관계 타입 id. `NULL`이면 전체 |
| `dir` | `"char"` | 필수 | — | `'o'` / `'i'` / `'b'` |
| `minhop` | `int4` | 필수 | — | 최소 깊이 |
| `maxhop` | `int4` | 필수 | — | 최대 깊이 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `node` | `int8` | 아니오 | 도달한 노드 |
| `depth` | `int4` | 아니오 | **최초** 도달 깊이 |

**시작 노드 취급 (확인된 동작, [traverse.rs:122](../../engine/src/storage/traverse.rs#L122))**
- `minhop <= 0`이면 `(src, 0)`을 먼저 방출한다.
- 시작 노드는 깊이 0에서 방문 처리되어 프론티어가 다시 확장하지 않지만,
  사이클이 돌아오면 여전히 **답**이다 — `(a)-[*1..k]->(b)`가 `b = a`를 바인딩한다.
  이 경우 돌아온 깊이로 한 번 방출된다.

**구현 결정**: SPI 커넥션과 준비된 계획을 **전체 순회에 하나만** 쓴다. 지름이 큰
그래프에서는 레벨 수가 6자리가 될 수 있어, 루프 안에서 재연결·재계획하면 같은
순회를 평문 재귀 CTE로 쓴 것보다 10배 느려졌다는 측정이 이 설계를 만들었다
([traverse.rs:100](../../engine/src/storage/traverse.rs#L100)).

**시그니처 결정**: `dir`이 `text`가 아니라 `"char"`인 이유는 `og_vlp`과
**경로 컬럼 앞까지 시그니처를 일치**시켜, Cypher 컴파일러가 같은 LATERAL 조인에
둘 중 아무거나 방출할 수 있게 하기 위함이다([traverse.rs:77](../../engine/src/storage/traverse.rs#L77)).

**`ROWS 100` 결정**: pgrx는 집합 반환 함수에 PostgreSQL 기본 추정치 1000행을 준다.
`og_vlp`은 100을 선언한다. 같은 질문에 답하는 두 함수가 자릿수 하나만큼 다르게
비용 산정되면 플래너가 서로 다른 조인 순서를 고르고, 비교가 그 추정치를 측정하게
된다 — 그래서 강제로 맞췄다([engine/sql/access.sql:192](../../engine/sql/access.sql#L192)).

**예제**

```sql
SELECT count(*) FROM og_reach(412316860417, NULL, 'b'::"char", 1, 6);
```

**실패 조건**

| 조건 | 오류 |
|---|---|
| `dir`이 `o`/`i`/`b`가 아님 | `direction must be 'o', 'i' or 'b', not '<c>'` ([traverse.rs:92](../../engine/src/storage/traverse.rs#L92)) |
| 계획 수립 실패 | `adjacency scan could not be planned: <e>` ([traverse.rs:116](../../engine/src/storage/traverse.rs#L116)) |
| 스캔 실패 | `adjacency scan failed: <e>` ([traverse.rs:137](../../engine/src/storage/traverse.rs#L137)) |

---

## 4. 백엔드 로컬 CSR — pgGraph 형태의 실험

**결정(Decision)**: 토폴로지를 한 번 컴파일해 **조밀한 `u32` 인덱스의 CSR**로
백엔드 메모리에 두고, SPI도 힙도 플래너도 없이 걷는다. 더 빠르고, 힙을 떠나면서
잃는 것을 정확히 잃는다 — **스냅숏이 빌드 시점에 고정되고 RLS를 전혀 참조하지
않는다**([traverse.rs:19](../../engine/src/storage/traverse.rs#L19)).

메모리는 Rust 힙에 잡는다. PostgreSQL 메모리 컨텍스트면 트랜잭션 끝에 해제되는데,
다음 문이 이미 빌드된 것을 찾는 게 요점이기 때문이다
([traverse.rs:205](../../engine/src/storage/traverse.rs#L205)).

> ⚠️ **필수 인지 사항**: CSR은 **백엔드(커넥션)마다** 하나다. 커넥션 풀을 쓰는
> 애플리케이션은 어느 백엔드에 CSR이 있는지 보장할 수 없다. `og_csr_reach`가
> `no compiled graph in this backend`로 실패할 수 있다
> → [12_improvements_api.md](12_improvements_api.md) **API-13**.

### `og_csr_build(etypes int4[], dir text DEFAULT 'o') RETURNS TABLE(nodes int8, edges int8, bytes int8, build_ms float8)`

정의: [engine/src/storage/traverse.rs:295](../../engine/src/storage/traverse.rs#L295) · 휘발성: 기본값(`VOLATILE`) · 병렬: 기본값

**무엇을 하는가**: 토폴로지를 이 백엔드의 메모리로 컴파일하고, 무엇을 만들었는지 보고한다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `etypes` | `int4[]` | 필수(NULL 허용) | — | 포함할 관계 타입 id. `NULL`이면 전체 |
| `dir` | `text` | 선택 | `'o'` | `'o'` / `'i'` / `'b'`. **`text`다** |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `nodes` | `int8` | 아니오 | 조밀 인덱스로 들어간 고유 노드 수 |
| `edges` | `int8` | 아니오 | 정방향 CSR의 이웃 항목 수 |
| `bytes` | `int8` | 아니오 | `ids` + 정/역방향 배열의 총 바이트 |
| `build_ms` | `float8` | 아니오 | 빌드 소요 시간(밀리초) |

**구현 사실**
- `og_adj` 한 번 스캔으로 정·역방향을 모두 만든다 — `'o'` 세그먼트마다 `'i'`
  세그먼트가 이미 있으므로 역방향 CSR이 추가 I/O 없이 나온다([traverse.rs:242](../../engine/src/storage/traverse.rs#L242)).
- 노드 id는 64비트 희소이므로 정렬된 id 벡터에 대한 `u32` 위치로 바꾼다.
  배열이 절반이 되고 프론티어 스캔이 캐시 안에 머문다([traverse.rs:184](../../engine/src/storage/traverse.rs#L184)).
- 역방향까지 만드는 이유: 양방향 최단 경로가 그것 없이는 옳지 않기 때문.

**예제**

```sql
SELECT * FROM og_csr_build(NULL, 'o');
--  nodes  |  edges  |  bytes   | build_ms
-- 1000000 | 5000000 | 48000004 |   4.9
```

**실패 조건**
- `dir` 부적합 → `direction must be 'o', 'i' or 'b', not '<x>'`
- 스캔 실패 → `adjacency scan failed: <e>` ([traverse.rs:251](../../engine/src/storage/traverse.rs#L251))

### `og_csr_reach(src int8, minhop int4, maxhop int4) RETURNS TABLE(node int8, depth int4)`

정의: [engine/src/storage/traverse.rs:359](../../engine/src/storage/traverse.rs#L359) · 휘발성: `STABLE` · 병렬: `PARALLEL RESTRICTED`

**무엇을 하는가**: 컴파일된 배열 안에서만 도달성을 걷는다. SPI·힙·플래너 없음.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 시작 노드 |
| `minhop` | `int4` | 필수 | — | 최소 깊이 |
| `maxhop` | `int4` | 필수 | — | 최대 깊이 |

`etypes`/`dir`은 **빌드 시점에 고정**되므로 인자가 없다.

**반환**: `og_reach`와 동일 (`node`, `depth`). 시작 노드 취급도 동일.

**예제**

```sql
SELECT * FROM og_csr_build(NULL, 'o');
SELECT count(*) FROM og_csr_reach(412316860417, 1, 6);
SELECT og_csr_drop();
```

**실패 조건**
- 빌드 안 함 → `no compiled graph in this backend — call og_csr_build() first`
  ([traverse.rs:340](../../engine/src/storage/traverse.rs#L340))
- `src`가 CSR에 없으면 **오류가 아니라 0행**([traverse.rs:366](../../engine/src/storage/traverse.rs#L366))

### `og_csr_hops(src int8, dst int8, maxhop int4 DEFAULT 32) RETURNS int4`

정의: [engine/src/storage/traverse.rs:442](../../engine/src/storage/traverse.rs#L442) · 휘발성: `STABLE` · 병렬: `PARALLEL RESTRICTED`

**무엇을 하는가**: 두 노드 사이 최단 경로의 홉 수. `maxhop` 안에 닿지 않으면 `NULL`.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 시작 노드 |
| `dst` | `int8` | 필수 | — | 목표 노드 |
| `maxhop` | `int4` | 선택 | `32` | 탐색 상한 |

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| (스칼라) | `int4` | **예** | 홉 수. `src = dst`면 `0`. 미도달/미등록 노드면 `NULL` |

**구현 결정 (양방향 탐색)**
- 두 프론티어가 중간에서 만나 `d^k` 대신 대략 `2·d^(k/2)` 노드를 탐색한다.
- 역방향 프론티어는 역방향 CSR을 걷는다 → 방향 그래프로 빌드했으면 **방향 최단 경로**.
- **레벨을 끝까지 다 본 뒤에만 답한다.** 첫 만남에서 멈추면 최단보다 한 홉 긴
  경로를 보고하는 일이 무시 못 할 빈도로 생긴다([traverse.rs:440](../../engine/src/storage/traverse.rs#L440)).
- 매 반복마다 **더 작은 프론티어**를 확장해 편향된 그래프에서 균형을 유지한다
  ([traverse.rs:457](../../engine/src/storage/traverse.rs#L457)).

**예제**

```sql
SELECT og_csr_hops(412316860417, 481036337153);      -- 3
SELECT og_csr_hops(412316860417, 481036337153, 2);   -- NULL
```

**실패 조건**: CSR 미빌드 시 위와 동일한 오류.

### `og_csr_stats() RETURNS TABLE(built_for text, nodes int8, edges int8, bytes int8)`

정의: [engine/src/storage/traverse.rs:322](../../engine/src/storage/traverse.rs#L322) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 이 백엔드가 지금 무엇을 들고 있는지 보고한다.

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `built_for` | `text` | **예** | 빌드 키. `"<dir>:<정렬된 etypes>"` 또는 `"<dir>:*"`. 미빌드면 `NULL` |
| `nodes` / `edges` / `bytes` | `int8` | 아니오 | 미빌드면 모두 `0` |

빌드 키 형식: [traverse.rs:212](../../engine/src/storage/traverse.rs#L212) `build_key`.
예: `o:*`, `b:[3, 7]`.

**예제**

```sql
SELECT * FROM og_csr_stats();
--  built_for | nodes | edges | bytes
--  o:*       | 1000  | 5000  | 48004
```

### `og_csr_drop() RETURNS void`

정의: [engine/src/storage/traverse.rs:316](../../engine/src/storage/traverse.rs#L316) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 이 백엔드의 컴파일된 그래프를 버린다. 미빌드 상태에서 불러도 오류가 아니다.

---

## 5. 통계 · 진단

### `og_degree(src int8, etype int4, dir text) RETURNS int8`

정의: [engine/src/storage/adjacency.rs:76](../../engine/src/storage/adjacency.rs#L76) · 휘발성: `STABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

**무엇을 하는가**: 관계 타입 하나 · 방향 하나에 대한 노드 차수(spec 001 FR-015). Cypher 플래너가 확장 순서를 고를 때 쓴다.

**인자**

| 이름 | 타입 | 필수/선택 | 기본값 | 설명 |
|---|---|---|---|---|
| `src` | `int8` | 필수 | — | 노드 id |
| `etype` | `int4` | 필수 | — | 관계 타입 id (`og_type_id(graph, name)`) |
| `dir` | `text` | 필수 | — | `'o'` / `'i'` |

**반환**: `int8`. 세그먼트가 없으면 `0`.

**예제**

```sql
SELECT og_degree(412316860417, og_type_id('default','ACTED_IN'), 'o');
```

### `og_degree_all(src int8, dir text) RETURNS int8`

정의: [engine/src/storage/adjacency.rs:88](../../engine/src/storage/adjacency.rs#L88) · 휘발성: `STABLE` · 병렬: `PARALLEL SAFE`, `STRICT`

모든 관계 타입에 걸친 방향별 총 차수.

---

### `og_graph_stats(graph text) RETURNS jsonb`

정의: [engine/src/storage/stats.rs:11](../../engine/src/storage/stats.rs#L11) · 휘발성: `STABLE` · 병렬: `STRICT`

**무엇을 하는가**: 타입별 인스턴스 수와 인접 세그먼트 상태를 하나의 jsonb로 준다(spec 001 FR-015/FR-019).

**반환 구조**

| 키 | 타입 | 설명 |
|---|---|---|
| `graph` | text | 그래프 이름 |
| `nodes` / `edges` | int | 그래프 전체 개수 |
| `types[]` | array | `{name, kind, abstract, instances}` |
| `adjacency.segments` | int | `og_data.og_adj`의 총 행 수 (**그래프별이 아니라 전역**) |
| `adjacency.avg_fill` | float | 세그먼트당 평균 이웃 수 |
| `adjacency.chunk_size` | float | `256` ([adjacency.rs:15](../../engine/src/storage/adjacency.rs#L15)) |
| `adjacency.packing_ratio` | float | `avg_fill / 256`. **1.0이 완벽 압축**, 낮으면 재구성이 도움됨 |
| `adjacency.chunked_supernodes` | int | `seq > 0`인 세그먼트 수 = 256개를 넘긴 노드 |

> ⚠️ `adjacency.*` 블록은 `og_data.og_adj` 전체를 집계한다 — `graph` 인자로
> 필터링하지 않는다([stats.rs:46](../../engine/src/storage/stats.rs#L46)). 여러 그래프를
> 쓰는 인스턴스에서는 그래프별 값이 아니다
> → [12_improvements_api.md](12_improvements_api.md) **API-14**.

**예제**

```sql
SELECT jsonb_pretty(og_graph_stats('default'));
```

### `og_degree_distribution(graph text) RETURNS jsonb`

정의: [engine/src/storage/stats.rs:86](../../engine/src/storage/stats.rs#L86) · 휘발성: `STABLE` · 병렬: `STRICT`

**무엇을 하는가**: 차수 히스토그램. 슈퍼노드가 아프기 전에 발견하기 위한 것(FR-020).

**반환**: `{graph, buckets: [{bucket, nodes, max_degree}]}`.
버킷은 `width_bucket(deg, 0, 1024, 8)` — 0..1024를 8구간으로 나눈다
([stats.rs:99](../../engine/src/storage/stats.rs#L99)). `dir = 'o'`만 집계한다.

### `og_reorganize(graph text) RETURNS int8`

정의: [engine/src/storage/stats.rs:121](../../engine/src/storage/stats.rs#L121) · 휘발성: 기본값 · 병렬: 기본값

**무엇을 하는가**: 파편화된 인접 세그먼트를 온라인으로 재압축한다(spec 001 FR-018). 리더를 막지 않는다 — 노드별 세그먼트를 각각 작은 트랜잭션 로컬 업데이트로 다시 쓴다.

**대상 선정**: `count(*) > 1 AND sum(n) <= count(*) * 256 - 256` —
세그먼트가 둘 이상인데 전체가 한 개 덜 쓸 만큼만 차 있는 (`src, etype, dir`) 조합
([stats.rs:132](../../engine/src/storage/stats.rs#L132)).

**반환**: 재압축한 (`src`, `etype`, `dir`) 조합의 수.

**예제**

```sql
SELECT og_graph_stats('default') -> 'adjacency' ->> 'packing_ratio';  -- 0.31
SELECT og_reorganize('default');                                       -- 842
SELECT og_graph_stats('default') -> 'adjacency' ->> 'packing_ratio';  -- 0.98
```

### `og_check_integrity() RETURNS TABLE(kind text, entity_id int8, detail text)`

정의: [engine/src/storage/stats.rs:172](../../engine/src/storage/stats.rs#L172) · 휘발성: `STABLE` · 병렬: 기본값

**무엇을 하는가**: 레지스트리·타입 테이블·인접 세그먼트를 교차 검사한다(spec 009 FR-015). **빈 결과가 통과 조건이다.**

**반환**

| 컬럼 | 타입 | NULL 가능 | 설명 |
|---|---|---|---|
| `kind` | `text` | 아니오 | 아래 4종 중 하나 |
| `entity_id` | `int8` | 예 | 관련 엔티티 id |
| `detail` | `text` | 아니오 | 사람이 읽을 설명 |

| `kind` | 의미 |
|---|---|
| `dangling_adjacency` | 인접 항목이 존재하지 않는 엣지를 가리킴 |
| `missing_adjacency` | 엣지가 두 끝점 중 하나에서 도달 불가 |
| `segment_length_mismatch` | `n` 값과 `nbr`/`eid` 배열 길이가 불일치 |
| `orphan_node` | 노드가 알 수 없는 타입을 참조 |

**각 검사는 `LIMIT 100`** ([stats.rs:188](../../engine/src/storage/stats.rs#L188) 등) —
결과가 비어 있지 않으면 "최소 이만큼"으로 읽을 것. 인자가 없어 **그래프별 검사가
불가능**하다 → [12_improvements_api.md](12_improvements_api.md) **API-14**.

**예제**

```sql
SELECT * FROM og_check_integrity();
-- (0 rows)  ← pass
```

---

## 6. 병렬 안전성 — 왜 다른가

| 함수 | 병렬 | 이유 (코드 근거) |
|---|---|---|
| `og_expand`, `og_expand_batch`, `og_vlp`, `og_reach_sql` | `PARALLEL SAFE` | 순수 SQL, 부수 효과 없음 ([access.sql](../../engine/sql/access.sql)) |
| `og_degree`, `og_degree_all`, `og_subtypes`, `og_supertypes`, `og_is_subtype` | `PARALLEL SAFE` | 읽기 전용 조회 |
| `og_reach` | `PARALLEL RESTRICTED` | SPI를 사용한다 — 병렬 워커에서 SPI는 안전하지 않다 |
| `og_csr_reach`, `og_csr_hops` | `PARALLEL RESTRICTED` | 백엔드 로컬 `thread_local` 상태를 읽는다. 워커에는 그 상태가 없다 |
| `og_csr_build`, `og_csr_drop`, `og_csr_stats` | 기본값(`PARALLEL UNSAFE`) | 백엔드 로컬 상태를 **변경**한다 |
| `og_cypher_stats` | `PARALLEL UNSAFE` (명시) | 커넥션 범위 카운터 |

---

## 7. 금지 / 필수

- **필수**: `dir` 인자에 함수별 올바른 타입을 캐스트할 것(§2).
- **필수**: `og_csr_*`를 쓰기 전에 **같은 커넥션**에서 `og_csr_build()`를 부를 것.
  풀링된 커넥션에서는 매 사용 전에 `og_csr_stats()`로 확인하거나 다시 빌드할 것.
- **금지**: RLS가 적용된 그래프에서 `og_csr_reach` / `og_csr_hops`를 권한 경계로
  믿지 말 것 — **RLS를 전혀 참조하지 않는다**([traverse.rs:22](../../engine/src/storage/traverse.rs#L22)).
- **금지**: `og_csr_*`의 결과를 "현재 상태"로 믿지 말 것 — 스냅숏은 빌드 시점이며
  같은 트랜잭션의 미커밋 쓰기도 보이지 않는다.
- **금지**: 큰 그래프에서 깊은 `og_vlp`을 부르지 말 것. 도달성만 필요하면
  `og_reach`를 쓸 것.
- **필수**: `og_check_integrity()` 결과가 비어 있지 않으면 각 검사가 `LIMIT 100`
  이라는 점을 기억할 것.

---

## 8. 관련 문서

- Cypher가 어느 함수를 고르는지 → [03_cypher.md §5](03_cypher.md)
- 측정 수치와 배경 → [docs/deep-traversal.md](../../docs/deep-traversal.md), [docs/benchmark.md](../../docs/benchmark.md)
- 쓰기 경로가 인접 세그먼트를 유지하는 방식 → [02_data_dml.md](02_data_dml.md)

<!-- affects: api, backend, data -->
<!-- requires-update: 02_api/03_cypher.md, 02_api/12_improvements_api.md -->
