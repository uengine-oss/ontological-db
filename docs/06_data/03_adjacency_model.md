# 03. 인접(adjacency) 모델 — CSR 세그먼트

> **이 문서가 답하는 질문**
> - 세그먼트 한 행은 물리적으로 무엇을 담고 있고 얼마나 큰가?
> - 세그먼트는 언제 쪼개지고 언제 합쳐지는가?
> - 왜 양방향으로 두 번 저장하는가?
> - `(src, dir, etype)` 접근이 실제로 어떤 인덱스를 타는가?
> - TOAST / fillfactor / MVCC가 이 설계에 무엇을 하는가?

**정본**: [`engine/sql/bootstrap.sql:186-214`](../../engine/sql/bootstrap.sql),
[`engine/src/storage/adjacency.rs`](../../engine/src/storage/adjacency.rs) (97줄),
[`engine/sql/access.sql:11-37`](../../engine/sql/access.sql).

---

## 사실 — 한 행이 담는 것

```sql
CREATE TABLE og_data.og_adj (
    src   int8   NOT NULL,   -- 이 세그먼트의 주인 노드
    etype int4   NOT NULL,   -- 관계 타입 id
    dir   "char" NOT NULL,   -- 'o' outgoing | 'i' incoming
    seq   int4   NOT NULL,   -- 같은 (src, etype, dir) 안의 세그먼트 순번, 0부터
    n     int4   NOT NULL,   -- 살아 있는 원소 수
    nbr   int8[] NOT NULL,   -- 이웃 노드 id
    eid   int8[] NOT NULL,   -- 그 이웃에 닿는 엣지 id (nbr과 인덱스 정렬)
    PRIMARY KEY (src, etype, dir, seq)
) WITH (fillfactor = 80);
```

**불변식** (코드가 지켜야 하고 제약이 강제하지 않는 것):
1. `n = array_length(nbr, 1) = array_length(eid, 1)`
2. `nbr[i]`와 `eid[i]`는 **같은 엣지**를 가리킨다
3. `dir ∈ {'o', 'i'}`
4. `seq`는 `(src, etype, dir)` 안에서 0부터 연속

`og_check_integrity()`의 검사 3이 불변식 1을 확인한다
(`engine/src/storage/stats.rs:225-241`). **불변식 2·3·4를 검사하는 코드는 없다.**

한 세그먼트의 최대 이웃 수:
```rust
pub const CHUNK: i32 = 256;
```
(`engine/src/storage/adjacency.rs:15`)

---

## 결정 — 왜 256인가

주석이 산수를 밝힌다: `256 * 8B * 2 arrays = 4 KB`, 8KB 힙 페이지 안에 들어간다
(`engine/src/storage/adjacency.rs:13-14`, `engine/sql/bootstrap.sql:188-191`).

**의미**: 한 노드를 확장하는 것이 `degree`번의 인덱스 프로브 + `degree`번의 랜덤 힙
페치가 아니라 **한 번의 순차 배열 읽기**가 된다. 이것이 Apache AGE와의 구조적 차이다
(`engine/src/storage/adjacency.rs:7-9`).

실측 근거는 `docs/benchmark.md`에 있다. AGE의 엣지 테이블은 같은 규모에서
약 7,200 페이지이고(페이지당 135 엣지 행), 3홉 질의가 1,800만 행을 만들어
360행을 남긴다(`docs/benchmark.md:184, 214-215`).

---

## 사실 — 실제 튜플 크기와 페이지 점유

한 세그먼트의 **논리적** 크기 (256개 꽉 찬 경우):

| 구성 | 바이트 |
|---|---|
| `int8[]` varlena 헤더 × 2 | 약 24 × 2 = 48 |
| 원소 데이터 | 256 × 8 × 2 = 4,096 |
| 고정 컬럼 (`src`,`etype`,`dir`,`seq`,`n`) + 정렬 | 약 24 |
| 힙 튜플 헤더 | 23 + 정렬 |
| **합계** | **약 4.2 KB** |

8KB 페이지의 사용 가능 공간은 약 8,160바이트다. 따라서 **꽉 찬 세그먼트는
fillfactor와 무관하게 페이지당 하나만 들어간다** (4.2KB × 2 = 8.4KB > 8.16KB).

> **미측정**: 위는 압축을 고려하지 않은 논리 크기다. `SET STORAGE MAIN`은
> 압축을 허용하므로(아래 참조) 실제 저장 크기는 더 작을 수 있다. 노드 id는
> 상위 27비트(shard 0 + type_id)를 공유하므로 리틀엔디언 배열에서 8바이트 주기의
> 반복 패턴이 생기고, pglz가 이를 잡을 여지가 있다. **측정 방법**:
> ```sql
> SELECT n, pg_column_size(nbr) AS nbr_bytes, pg_column_size(eid) AS eid_bytes
>   FROM og_data.og_adj ORDER BY n DESC LIMIT 5;
>
> SELECT count(*) AS segments, avg(n)::numeric(6,1) AS avg_fill,
>        pg_relation_size('og_data.og_adj') AS bytes,
>        (pg_relation_size('og_data.og_adj') / GREATEST(count(*),1))::int AS bytes_per_segment
>   FROM og_data.og_adj;
> ```

---

## 사실 — TOAST 동작

```sql
ALTER TABLE og_data.og_adj ALTER COLUMN nbr SET STORAGE MAIN;
ALTER TABLE og_data.og_adj ALTER COLUMN eid SET STORAGE MAIN;
```
(`engine/sql/bootstrap.sql:210-211`)

의도는 주석에 있다: "Keep the arrays inline: TOASTing them would destroy the locality
we just bought"(`engine/sql/bootstrap.sql:208-209`).

`STORAGE MAIN`의 정확한 의미 (PostgreSQL 문서 기준):
- **압축은 허용**된다.
- **행 밖 저장(out-of-line)은 최후의 수단으로만** 일어난다 — 압축 후에도 튜플이
  페이지에 들어가지 않을 때만.

4.2KB는 8KB 페이지에 들어가므로, 이 설계에서 `nbr`/`eid`가 TOAST 테이블로 나가는
일은 **정상 동작에서 발생하지 않는다.**

> 부트스트랩 주석의 "4KB payload stays under the 2KB*4 toast threshold per column"
> (`engine/sql/bootstrap.sql:209`)은 기제를 조금 뭉뚱그린 표현이다.
> `TOAST_TUPLE_THRESHOLD`는 컬럼당이 아니라 **튜플당** 약 2,000바이트이고,
> 4.2KB 튜플은 그 임계를 **넘는다**. 넘은 뒤 toaster가 도는데, `MAIN` 설정 때문에
> 압축만 시도하고 밖으로 빼지 않을 뿐이다. 결과는 주석이 말한 대로지만 이유는 다르다.

**함의**: `og_adj`에는 TOAST 테이블이 존재하되(모든 varlena 컬럼 보유 테이블이 그렇다)
정상 경로에서는 비어 있어야 한다. 확인:
```sql
SELECT reltoastrelid::regclass AS toast_table,
       pg_relation_size(reltoastrelid) AS toast_bytes
  FROM pg_class WHERE oid = 'og_data.og_adj'::regclass;
```
`toast_bytes`가 0이 아니면 어딘가에서 세그먼트가 4KB를 넘고 있다는 뜻이다.

---

## 사실 — 세그먼트 분할 (split)

"분할"은 없다. **꼬리 세그먼트가 꽉 차면 새 세그먼트를 만든다.**

```rust
pub fn append(src: i64, etype: i32, dir: char, nbr: i64, eid: i64) {
    let updated = ... "UPDATE og_data.og_adj a
        SET nbr = a.nbr || $4::int8, eid = a.eid || $5::int8, n = a.n + 1
      WHERE a.src = $1 AND a.etype = $2 AND a.dir = $3::text::\"char\"
        AND a.seq = (SELECT max(seq) FROM og_data.og_adj
                      WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\")
        AND a.n < $6
      RETURNING a.seq" ...;

    if updated.is_none() {
        ... "INSERT INTO og_data.og_adj (...) VALUES ($1, $2, $3::text::\"char\",
             COALESCE((SELECT max(seq) + 1 FROM og_data.og_adj
                        WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\"), 0),
             1, ARRAY[$4::int8], ARRAY[$5::int8])" ...;
    }
}
```
(`engine/src/storage/adjacency.rs:19-44`)

읽어야 할 것:

1. **꼬리에만 붙인다.** `seq = max(seq)` 조건 때문에 중간 세그먼트에는 절대 채워 넣지 않는다.
   그래서 중간에서 원소가 빠지면 그 구멍은 `og_reorganize()` 전까지 남는다.
2. **UPDATE가 0행이면 INSERT로 폴백한다.** 두 경우가 이 경로를 탄다 —
   ① 이 `(src, etype, dir)`의 첫 이웃(`max(seq)`가 NULL → `COALESCE(..., 0)`),
   ② 꼬리가 꽉 찬 경우(`a.n < 256` 실패).
3. **매 append마다 `max(seq)` 상관 서브쿼리가 돈다.** PK가
   `(src, etype, dir, seq)`이므로 역방향 인덱스 스캔 1건이지만, 이웃 하나당 한 번씩이다.
4. **`||` 배열 연결은 튜플 전체를 다시 쓴다.** PostgreSQL의 UPDATE는 항상 새 행 버전을
   만들므로, k번째 이웃을 붙이는 것은 약 `(100 + 16k)`바이트를 새로 쓰고
   `(100 + 16(k−1))`바이트를 죽은 튜플로 남기는 것이다.

   256개짜리 세그먼트 하나를 한 개씩 붙여 채우면:
   ```
   Σ(k=1..256) (100 + 16k) ≈ 25,600 + 526,336 ≈ 552 KB
   ```
   **최종 살아 있는 4.2KB를 만들기 위해 약 552KB를 쓰고 548KB를 죽인다** — 약 130배.
   WAL도 같은 배수로 늘어난다. → [`10_improvements_data.md`](10_improvements_data.md) `PERF-02`

---

## 사실 — 세그먼트 축소 (remove)

```rust
pub fn remove(src: i64, etype: i32, dir: char, eid: i64) {
    // 1) splice out at the matching index, keeping both arrays aligned
    "UPDATE og_data.og_adj a
        SET nbr = a.nbr[1 : i.idx - 1] || a.nbr[i.idx + 1 : array_length(a.nbr, 1)],
            eid = a.eid[1 : i.idx - 1] || a.eid[i.idx + 1 : array_length(a.eid, 1)],
            n   = a.n - 1
       FROM (SELECT src, etype, dir, seq, array_position(eid, $4::int8) AS idx
               FROM og_data.og_adj
              WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\"
                AND $4::int8 = ANY(eid)
              LIMIT 1) i
      WHERE a.src = i.src AND a.etype = i.etype AND a.dir = i.dir AND a.seq = i.seq"

    // 2) reclaim emptied segments so scans stay dense
    "DELETE FROM og_data.og_adj
      WHERE src = $1 AND etype = $2 AND dir = $3::text::\"char\" AND n = 0"
}
```
(`engine/src/storage/adjacency.rs:48-72`)

- 삭제도 **튜플 전체 재작성**이다.
- `$4 = ANY(eid)`는 인덱스를 못 탄다 — 해당 `(src, etype, dir)`의 세그먼트들을
  힙에서 읽어 배열을 스캔한다. 세그먼트가 몇 개뿐이면 싼 연산이다.
- **삭제할 때마다 두 번째 DELETE 문이 무조건 실행된다.** 비어 있는 세그먼트가 없어도
  인덱스 스캔 한 번을 한다.
- 중간 세그먼트가 줄어들어도 **뒤 세그먼트를 당겨오지 않는다.** 조각화가 누적된다.

---

## 사실 — 세그먼트 병합 (merge)은 `og_reorganize()`뿐

```sql
-- 대상 선정: 세그먼트가 2개 이상이고, 총 원소 수가 (세그먼트수-1)*256 이하인 것
--            = 하나 줄일 수 있는 것
HAVING count(*) > 1 AND sum(a.n) <= count(*) * $2 - $2
```
(`engine/src/storage/stats.rs:126-135`)

재작성은 노드×타입×방향 단위로 한 문장이다 —
`unnest` → `row_number()/256`로 재청킹 → 기존 삭제 → 재삽입
(`engine/src/storage/stats.rs:145-162`).

**성질**
- 대상은 **해당 그래프에 속한 노드의 세그먼트만**이다 (`og_node` → `type` 조인).
  → 그래프가 드롭된 뒤 남은 고아 세그먼트는 이 함수로 정리되지 않는다.
- **트랜잭션 하나 안에서 모든 대상을 순회한다.** 함수가 `#[pg_extern]`(비 `stable`)이고
  Rust 루프가 `Spi::run_with_args`를 반복 호출하므로, 100만 개 대상이면 100만 개의
  UPDATE/DELETE/INSERT가 한 트랜잭션에 쌓인다. "without blocking readers"라는 주석
  (`engine/src/storage/stats.rs:119-120`)은 MVCC 덕분에 **읽기**가 안 막힌다는 뜻이지,
  트랜잭션이 짧다는 뜻이 아니다.
- 조각화 지표는 `og_graph_stats()`의 `adjacency.packing_ratio`다
  (`engine/src/storage/stats.rs:79`). `1.0`이 완전 밀집이다.

---

## 결정 — 양방향 저장

엣지 하나를 만들 때 세그먼트 항목이 **두 개** 생긴다.

```rust
adjacency::append(src, tid, 'o', dst, eid);
adjacency::append(dst, tid, 'i', src, eid);
```
(`engine/src/storage/mod.rs:444-446`, 주석: "Both adjacency directions, same transaction — spec 001 FR-012")

**얻는 것**
- `<-[:K]-` 역방향 확장이 정방향과 **완전히 같은 비용**이다. `og_edge_dst_idx`를
  타는 인덱스 조회가 아니라 같은 배열 읽기다.
- 양방향 최단 경로가 성립한다. `og_csr_build`가 한 번의 스캔으로 정방향과
  역방향 CSR을 **둘 다** 만들 수 있는 이유가 이것이다 —
  "`og_adj` already stores an 'i' segment for every 'o' segment, so the reverse
  CSR costs no extra I/O"(`engine/src/storage/traverse.rs:242-243`).

**비용**
- 인접 저장 공간이 **2 × |E|** 원소다.
- 엣지 생성 시 append가 2회 → 위의 쓰기 증폭이 2배.
- 엣지 삭제 시 remove가 2회, 각각 두 문장 → 엣지당 4개 문장(`engine/src/storage/mod.rs:516-517`).

**정합성 위험**: 두 방향은 코드로만 묶여 있다. 한쪽만 성공하는 일은 트랜잭션이
막아주지만, 벌크 로드가 한쪽만 만들면 조용히 어긋난다.
`og_check_integrity()`의 검사 2가 정확히 이것을 잡는다
(`engine/src/storage/stats.rs:202-222`).

---

## 사실 — 접근 패턴 `(src, dir, etype)`

PK 인덱스는 `(src, etype, dir, seq)`이고, **`og_adj`에 다른 인덱스는 없다.**
컬럼 순서와 질의 순서가 다르다는 점에 주의.

### 컴파일된 Cypher 홉

```sql
CROSS JOIN LATERAL (
  SELECT u.nbr, u.eid
    FROM og_data.og_adj adj1, LATERAL unnest(adj1.nbr, adj1.eid) AS u(nbr, eid)
   WHERE adj1.src = a0.id AND adj1.dir = 'o'::"char" AND adj1.etype = ANY($types)
) u1
```
(`engine/src/cypher/compile.rs:900-904`)

| 조건 | PK 인덱스에서의 역할 |
|---|---|
| `src = ?` | 1번 컬럼, 등호 → 경계 조건 |
| `etype = ANY(...)` | 2번 컬럼, ScalarArrayOp → 경계 조건 |
| `dir = 'o'` | 3번 컬럼, 등호 → 경계 조건 |

**세 조건 모두 인덱스 경계 조건이 된다.** 이것이 이 컬럼 순서를 고른 이유다.

관계 타입을 지정하지 않은 `-[]->`는 `etype` 조건이 없어(`compile.rs:892-895`)
`src`만 경계가 되고 `dir`은 인덱스 내 필터가 된다. 어차피 그 노드의 세그먼트를
전부 봐야 하므로 손해가 아니다.

### 방향이 `Both`인 경우
```sql
adj1.dir IN ('o','i')
```
(`engine/src/cypher/compile.rs:897`) — 두 방향을 다 읽으므로 각 엣지가 두 번 관측된다.
그래서 컴파일러가 같은 패턴에서 `{u}.eid <> {other}` 중복 방지 술어를 건다
(`engine/src/cypher/compile.rs:908-914`).

### 공개 함수 경로
```sql
CREATE FUNCTION og_expand(src int8, etypes int4[], dir "char")
RETURNS TABLE (nbr int8, eid int8)
LANGUAGE sql STABLE PARALLEL SAFE ROWS 50 AS $$
    SELECT u.nbr, u.eid
      FROM og_data.og_adj a, LATERAL unnest(a.nbr, a.eid) AS u(nbr, eid)
     WHERE a.src = og_expand.src
       AND a.dir = og_expand.dir
       AND (og_expand.etypes IS NULL OR a.etype = ANY (og_expand.etypes))
$$;
```
(`engine/sql/access.sql:14-22`)

`LANGUAGE sql`이므로 **인라인된다** — 플래너가 `og_adj` 스캔 자체를 본다
(`engine/sql/access.sql:4-8`). 이것이 PL/pgSQL이나 C SRF와의 결정적 차이다.

`ROWS 50`은 손으로 준 추정치다. `og_expand_batch`는 `ROWS 500`(`access.sql:31`),
`og_vlp`는 `ROWS 100`(`access.sql:140`), `og_reach`는 pgrx 기본 1000을 명시적으로
100으로 낮춰 맞췄다(`engine/sql/access.sql:192-197`) — 같은 질문에 답하는 두 함수가
한 자릿수 다른 비용으로 평가되면 플래너가 다른 조인 순서를 고르고, 비교가
"추정치를 비교하는 일"이 되기 때문이다.

> `(og_expand.etypes IS NULL OR a.etype = ANY(...))` 형태는 파라미터가 NULL일 때와
> 아닐 때 **같은 계획**을 쓴다. NULL이 아닐 때도 `etype`이 경계 조건이 되는지는
> **미확인**이다. 확인 방법: `EXPLAIN (VERBOSE) SELECT * FROM og_expand(<id>, ARRAY[<t>], 'o')`
> 의 `Index Cond` 줄에 `etype`이 있는지 볼 것.

### 인덱스를 타지 못하는 접근

| 질의 | 어디 | 왜 |
|---|---|---|
| `DELETE FROM og_data.og_adj WHERE etype = $1` | `engine/src/catalog/types.rs:706` | `etype`은 2번 컬럼. **순차 스캔** → `DATA-03` |
| `SELECT src, dir, nbr FROM og_adj WHERE dir IN ('o','i') AND ...` | `engine/src/storage/traverse.rs:244-247` | 그래프 전체 컴파일. 설계상 전체 스캔 |
| `og_reorganize` 대상 선정 | `engine/src/storage/stats.rs:126-135` | 전체 스캔 + 집계. 설계상 그렇다 |
| `og_graph_stats`의 세그먼트 집계 | `engine/src/storage/stats.rs:49-52` | 전체 스캔 |
| `SELECT DISTINCT e FROM og_adj a, LATERAL unnest(a.eid) e WHERE a.src = $1` | `engine/src/storage/mod.rs:360-362` | `src` 경계 — 인덱스 탄다 |

---

## 사실 — fillfactor 80의 효과

```sql
) WITH (fillfactor = 80);
```

`fillfactor`는 **INSERT가 페이지를 얼마나 채울지**를 정한다. 남은 20%는 같은 페이지
안에서의 UPDATE(HOT update)를 위한 여유다.

이 테이블에서의 실제 효과:

- **꽉 찬 세그먼트(4.2KB)**: 페이지당 하나뿐이므로 20% 여유(≈1.6KB)로는 다음 버전
  (4.2KB)을 담을 수 없다. **HOT update가 불가능하다.** 그래서 append 한 번마다
  새 페이지로 이동하고 죽은 튜플이 남는다.
- **작은 세그먼트(초기 성장 구간)**: 이때는 여유가 도움이 된다. 예컨대 원소 10개짜리
  세그먼트(약 380바이트)는 한 페이지에 여러 개 들어가고 성장도 in-page로 흡수된다.
- 결과적으로 fillfactor 80은 **초기 성장 구간에서만 값을 한다.**

**의미**: `og_adj`는 VACUUM에 크게 의존한다. autovacuum 임계값
(`autovacuum_vacuum_scale_factor` 기본 0.2)은 이 접근 패턴에 비해 느슨하다.
운영 권고는 [`08_data_lifecycle.md`](08_data_lifecycle.md)에 있다.

---

## 사실 — 통계와 선택도

`og_adj`에 대해 확장이 `ANALYZE`를 부르는 곳은 **없다**
(근거: `engine/src/` 전체에서 `ANALYZE` 문자열의 유일한 매치는
`engine/src/cypher/mod.rs:682`의 `EXPLAIN` 옵션).

통계가 없으면:
- `unnest(a.nbr, a.eid)`의 행 수 추정이 배열의 실제 길이를 반영하지 못한다.
  세그먼트가 평균 200개 이웃을 담고 있어도 플래너는 그걸 모른다.
- `dir` / `etype`의 선택도가 기본 상수로 추정된다.
- 깊은 순회 전환 판단이 `pg_class.reltuples`를 쓰는데
  (`engine/src/cypher/compile.rs:46-53`), 그 값 역시 `ANALYZE` / `VACUUM`이
  채운다. 값이 없으면 코드는 **"깊이 ≥ 4"라는 단순 규칙으로 폴백**한다:
  ```rust
  let (nodes, edges) = match est {
      Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => (n as f64, e as f64),
      _ => return max >= DEEP,   // DEEP = 4
  };
  ```
  (`engine/src/cypher/compile.rs:51-54`)

→ [`10_improvements_data.md`](10_improvements_data.md) `PERF-09`

---

## 금지 / 필수

**금지**
- `og_adj`에 SQL로 직접 append/remove 하는 것. `n`과 두 배열을 동시에 맞춰야 한다.
- 한 방향만 만드는 것. `'o'`와 `'i'`가 짝이어야 한다.
- 중간 `seq` 세그먼트를 지워 `seq`에 구멍을 내는 것. `append`가 `max(seq)`로
  꼬리를 찾으므로 동작은 하지만, `og_reorganize`가 재번호를 매길 때까지
  `og_expand`가 읽는 순서와 `seq`의 의미가 어긋난다.

**필수**
- 벌크 로드는 **세그먼트를 완성된 형태로 한 번에 INSERT** 할 것. append 루프는
  130배 쓰기 증폭을 낳는다. 참고 구현이 이미 저장소 안에 있다:
  ```sql
  INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
  SELECT src, {rid}, 'o', chunk, count(*)::int4, array_agg(dst), array_agg(id)
    FROM (SELECT src, dst, id,
                 ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
            FROM og_data.og_edge WHERE type_id = {rid}) x
   GROUP BY src, chunk;
  ```
  (`bench/harness.py:343-348`. 역방향은 `dst` 기준으로 같은 문장을 한 번 더.)
- 벌크 로드 뒤 반드시 `ANALYZE og_data.og_adj;`.
- 대량 삭제 뒤 `og_reorganize(graph)` 후 `VACUUM og_data.og_adj;`.

---

<!-- affects: data, backend, performance -->
<!-- requires-update: docs/06_data/09_query_access_paths.md, docs/06_data/10_improvements_data.md -->
