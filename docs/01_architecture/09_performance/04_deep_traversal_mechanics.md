# 깊은 순회의 기계장치 — 세 경로, 전환 판정, 정확성의 경계

> **이 문서가 답하는 질문**
> - 가변 길이 매치는 몇 가지 경로로 실행될 수 있고, 각각 무엇을 보장하는가?
> - 컴파일러는 정확히 어느 코드 라인에서 어떤 조건으로 경로를 고르는가?
> - 각 경로가 답하는 질문은 **정말 같은 질문인가**?
> - 문서(`docs/deep-traversal.md`)와 코드가 어긋난 곳은 어디인가?

---

## 1. 사실 — 네 가지 실행 경로

| 경로 | 구현 | 행 수 | 지키는 것 | Cypher가 자동으로 고르는가 |
|---|---|---|---|---|
| `og_vlp` | [`access.sql:138-156`](../../../engine/sql/access.sql) — `WITH RECURSIVE`, 트레일, `int8[] path` | `Σ dⁱ` | 경로 다중도, MVCC, RLS, 미커밋 쓰기 | **예** (기본) |
| `og_reach_sql` | [`access.sql:169-187`](../../../engine/sql/access.sql) — 같은 CTE, path 없음, `UNION` | `O(k·|V|)` | MVCC, RLS, 미커밋 쓰기 | **아니오 — 컴파일러가 방출하지 않는다** |
| `og_reach` | [`traverse.rs:80-161`](../../../engine/src/storage/traverse.rs) — Rust BFS, 방문집합, SPI | `≤ |V|` | MVCC, RLS, 미커밋 쓰기 | **예** (조건부) |
| `og_csr_reach` | [`traverse.rs:359-401`](../../../engine/src/storage/traverse.rs) — 백엔드-로컬 CSR | `≤ |V|` | **아무것도** — 동결 스냅샷, RLS 미적용 | **아니오 — 명시적 호출만** |

## 2. 사실 — 전환 판정: 코드 라인으로 특정

가변 길이 홉을 컴파일할 때 실제로 실행되는 조건은
[`compile.rs:865`](../../../engine/src/cypher/compile.rs) 한 줄이다.

```rust
let f = if rel.var.is_none() && self.reachability_only && prefer_reachability(max) {
    // ... "variable-length hop compiled as reachability (og_reach)"
    "og_reach"
} else {
    "og_vlp"
};
```

세 개의 논리곱이고, 각각의 근거는 다음과 같다.

### 조건 A — `rel.var.is_none()` (관계 변수 미바인딩)

`-[e:K*1..3]->` 처럼 관계 변수를 묶으면 그 변수가 **곧 경로**이므로 열거를 피할 수 없다.
`RETURN` 이 무엇이든 무관하게 이 홉에서 거부된다 ([`compile.rs:865`](../../../engine/src/cypher/compile.rs)).

### 조건 B — `self.reachability_only` (다중도가 관측 불가)

두 곳에서 정해진다.

1. 질의 전체 판정 — `Compiler::multiplicity_blind`
   ([`compile.rs:339-349`](../../../engine/src/cypher/compile.rs)), `compile_read` 진입 시 1회
   ([`compile.rs:352`](../../../engine/src/cypher/compile.rs)):

   | 라인 | 규칙 |
   |---|---|
   | [`:340-342`](../../../engine/src/cypher/compile.rs) | `WITH` 가 **한 번이라도** 있으면 거부. 이 패스는 `WITH` 안을 들여다보지 않는다 |
   | [`:343`](../../../engine/src/cypher/compile.rs) | 마지막 절이 `RETURN` 이 아니면 거부 |
   | [`:344-346`](../../../engine/src/cypher/compile.rs) | `RETURN DISTINCT …` 이면 **허용** — 중복 행이 살아남을 수 없다 |
   | [`:347-348`](../../../engine/src/cypher/compile.rs) | 그 외에는 프로젝션(+`ORDER BY`)에 집계가 **하나라도** 있고, **모든** 항이 `blind_expr` 여야 허용 |

   `blind_expr` ([`compile.rs:82-100`](../../../engine/src/cypher/compile.rs)):
   `min`/`max` 는 무조건 허용, `count`/`collect` 는 `DISTINCT` 이면서 인자가 `*` 가 아닐 때만 허용,
   `sum`/`avg`/`stdev`/사용자 정의는 거부.

2. 패턴 단위 판정 — `MATCH p = …` 로 경로 변수를 묶으면 그 패턴 안에서만 꺼진다
   ([`compile.rs:657-660`](../../../engine/src/cypher/compile.rs)), 패턴이 끝나면 복원된다
   ([`compile.rs:694`](../../../engine/src/cypher/compile.rs)).

### 조건 C — `prefer_reachability(max)` (손익분기)

[`compile.rs:34-78`](../../../engine/src/cypher/compile.rs).

```rust
const WALKS: f64 = 512.0;   // compile.rs:42
const DEEP: u32 = 4;        // compile.rs:44

let est = crate::spiu::two::<f32, f32>(
    "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),
            (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)", &[]);
let (nodes, edges) = match est {
    Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => (n as f64, e as f64),
    _ => return max >= DEEP,            // compile.rs:53 — 통계가 없으면 깊이만 본다
};
let degree = (edges / nodes).max(1.0);  // compile.rs:58

let mut walks = 0.0f64; let mut level = 1.0f64;
for _ in 0..max {                       // compile.rs:70
    level *= degree; walks += level;
    if walks > WALKS || !walks.is_finite() { return true; }
}
false
```

- 통계 출처는 `pg_class.reltuples` — **카탈로그 조회 1회**, 스캔 없음.
  `ANALYZE` 되지 않은 테이블은 PostgreSQL 14+ 에서 `reltuples = -1` 이므로
  `n > 0.0 && e > 0.0` 가 거짓이 되어 `max >= 4` 로 되돌아간다.
- **평균 차수는 그래프 전체 기준이다.** 관계 타입별도 아니고, 그래프별도 아니다
  (`og_node`/`og_edge` 는 데이터베이스 전체의 레지스트리다).
  하나의 데이터베이스에 여러 그래프가 있거나 관계 타입 간 차수 편차가 크면 이 추정은 틀린다.
- 임계값 512는 **측정으로 맞춘 값이지 유도된 값이 아니다** — 주석이 그렇게 말한다
  ([`compile.rs:37-41`](../../../engine/src/cypher/compile.rs)).
  두 오류의 비용이 비대칭이라는 것이 근거다: 열거하지 말았어야 할 때 열거하면 시간이 터지고,
  도달성으로 갔어야 하지 않을 때 가면 1 ms 미만을 손해 본다.

## 3. 사실 — 문서와 코드가 어긋난 두 곳

[`docs/deep-traversal.md`](../../deep-traversal.md) 의 "When the compiler rewrites, and when it must not" 절은
현재 코드와 두 군데에서 다르다. **코드가 최신이다.**

| 문서의 서술 | 코드 |
|---|---|
| *"gated on the crossover being real: `Σ degreeⁱ > |V|`"* | `|V|` 를 쓰지 않는다. 고정 임계값 `WALKS = 512` ([`compile.rs:42`](../../../engine/src/cypher/compile.rs)) |
| *"Depth ≥ 12 skips the estimate"* | **그런 분기가 없다.** `DEEP = 4` 는 *통계가 없을 때의* 대체 조건일 뿐 ([`compile.rs:44,53`](../../../engine/src/cypher/compile.rs)) |

같은 문서의 뒤쪽 절("The cost rule was wrong, and the lattice proved it")은 첫 번째 항목을 스스로 정정하고 있으므로,
문서가 자기 자신과도 어긋나 있는 상태다. → [`PERF-19`](07_improvements_performance.md).

## 4. 사실 — 세 경로가 **같은 질문에 답하지 않는** 경우

### 4.1 `minhop > 1` 에서 답이 갈린다 (미해결로 판단)

`og_vlp` 는 길이가 `minhop..maxhop` 인 **트레일**로 닿는 노드를 낸다.
`og_reach` 는 **BFS 최단 거리**가 `minhop..maxhop` 인 노드를 낸다
([`traverse.rs:143-153`](../../../engine/src/storage/traverse.rs)):

```rust
for nbr in seg.into_iter().flatten() {
    if visited.insert(nbr) {          // 처음 본 노드만
        next.push(nbr);
        if depth >= minhop { out.push((nbr, depth)); }
    } else if nbr == src && !start_done && depth >= minhop { … }
}
```

깊이 `d < minhop` 에서 처음 도달한 노드는 `visited` 에 들어가고 **영원히 출력되지 않는다.**
그러나 그 노드에 길이 `minhop` 이상의 트레일이 존재할 수 있다.

최소 반례:

```
a → b,  b → c,  a → c          -- c 는 거리 1 이면서 길이 2 트레일로도 닿는다
MATCH (a)-[:K*2..2]->(x) RETURN count(DISTINCT x)
```

- `og_vlp(a, …, 2, 2)` → `a→b→c` 로 `c` 를 낸다 → 1
- `og_reach(a, …, 2, 2)` → `c` 는 깊이 1에서 방문됨 → 0

컴파일러는 `min` 을 그대로 넘긴다 ([`compile.rs:874-876`](../../../engine/src/cypher/compile.rs)):

```rust
"{joiner} {f}({from_alias}.id, {etype_pred}, {dir_lit}::\"char\", {min}, {max}) {w}{on}"
```

그리고 `prefer_reachability` 는 `max` 만 본다. 따라서 `*2..2` / `*3..5` / `*4` 처럼
`minhop > 1` 인 다중도-불감 질의는 **경로에 따라 다른 답을 낸다.**

- 회귀 스위트 [`engine/tests/sql/05_reachability.sql`](../../../engine/tests/sql/05_reachability.sql) 은
  `minhop = 1` 만 검사한다.
- 벤치 하네스의 `n_hop` / `reach_hop` 도 `*1..k` 만 쓴다
  ([`bench/harness.py:363-373`](../../../bench/harness.py)).
- **따라서 정확성 게이트가 이 경우를 한 번도 통과시킨 적이 없다.**

**신뢰도**: 코드를 읽어 도출했고 데이터베이스에서 실행해 확인하지는 않았다.
확인 방법과 제안은 [`PERF-20`](07_improvements_performance.md).

### 4.2 `og_reach_sql` 은 진짜 방문집합이 아니다

`UNION` 은 `(node, depth)` 쌍을 중복 제거하므로, 깊이 2와 3에서 모두 닿는 노드는 두 번 생산된다.
비용은 `O(k·|V|)` 이고, 순환이 있는 랜덤 그래프에서는 매 깊이마다 전 노드를 다시 낸다.
그 대신 마지막에 `GROUP BY node` 로 `min(depth)` 를 취하므로
**출력 집합은 `og_reach` 와 같다**([`access.sql:184-186`](../../../engine/sql/access.sql)) —
`minhop > 1` 일 때도 그렇다(§4.1과 달리 여기서는 모든 깊이가 생산되기 때문).

측정으로 확인된 결과: dense 픽스처 깊이 6에서 `og_reach_sql` 426 ms 대 `og_reach` 71 ms,
반대로 chain-1M 100,000홉에서는 154 ms 대 1,016 ms
([`bench/csr/results/`](../../../bench/csr/results/)).

### 4.3 `og_csr_reach` 는 다른 스냅샷을 본다

- 빌드 시점의 커밋 상태에 **동결**된다. 이후 커밋된 엣지는 다시 빌드하기 전까지 보이지 않는다.
- **RLS를 참조하지 않는다.** 호출자가 읽을 권한이 없는 행을 지나는 경로가 결과에 나타난다.
- 이 두 가지 때문에 컴파일러는 CSR로 라우팅하지 **않는다.**
  [`docs/deep-traversal.md`](../../deep-traversal.md) 가 그 결정을 명시적으로 기록한다.

## 5. 사실 — 세 경로의 비용 (측정)

dense 픽스처(50,000 노드 / 999,784 엣지 / 평균 차수 20), 중앙값 ms.
출처: [`bench/csr/results/deep-dense-20260817T021522Z.json`](../../../bench/csr/results/deep-dense-20260817T021522Z.json),
[`deep-dense-deep-20260817T021624Z.json`](../../../bench/csr/results/deep-dense-deep-20260817T021624Z.json).

| 깊이 | `og_vlp` | `og_reach_sql` | `og_reach` | `og_csr_reach` |
|---|---|---|---|---|
| 1 | 0.165 | 0.151 | 0.083 | **0.05** |
| 3 | 6.85 | 4.07 | 1.61 | **0.60** |
| 4 | 106.72 | 49.24 | 23.62 | **3.68** |
| 6 | 49,333.99 | 426.45 | 71.42 | **4.86** |
| 8 | — | 910.60 | 74.93 | **5.92** |
| 20 | — | 3,659.57 | 69.43 | **4.88** |

- **알고리즘 교체(`og_vlp` → `og_reach`)가 깊이 6에서 691배**를 벌었고,
  **힙을 떠나는 것(`og_reach` → `og_csr_reach`)이 약 15배**를 더한다.
- 두 도달성 경로는 그래프가 덮이는 순간 평평해진다(깊이 6에서 71 ms, 깊이 20에서 69 ms).
  **그 평평함이 "20홉 이상"이 실제로 요구하는 성질이다.**

## 6. 사실 — 전환 판정이 남긴 비용

전환이 일어나도 사라지지 않는 것들:

1. **도착 노드의 재조인과 jsonb 조립.**
   `og_reach` 는 노드 id만 돌려주지만 Cypher는 그 id마다 타입 뷰를 다시 조인하고
   `count(DISTINCT b)` 를 위해 노드 전체의 jsonb를 만든다.
   50,000 노드 4홉에서 195,202 페이지 대 16,170 페이지
   ([`02_measured_baselines.md` §4](02_measured_baselines.md)).
2. **`og_reach` 는 LATERAL 조인이므로 바깥 행마다 한 번씩 호출된다**
   ([`compile.rs:874-876`](../../../engine/src/cypher/compile.rs)).
   시작 노드 집합이 크면 SPI 연결·`prepare`·`HashSet` 할당이 그만큼 반복된다.
3. **`PARALLEL RESTRICTED`.** `og_reach` 와 CSR 함수는 모두 병렬 워커에서 실행될 수 없다
   ([`traverse.rs:80,359,442`](../../../engine/src/storage/traverse.rs)).
   이들을 포함한 계획은 리더에서 실행된다.
4. **판정 결과가 `PLAN_CACHE` 에 굳는다.**
   `prefer_reachability` 는 컴파일 시점에 한 번 평가되고 결과가 SQL 텍스트에 반영되어 캐시된다
   ([`cypher/mod.rs:47-67`](../../../engine/src/cypher/mod.rs)).
   그래프가 자라도 그 백엔드는 512개 캐시가 비워지기 전까지 옛 결정을 쓴다.

## 7. 결정 — 이 설계가 명시적으로 하지 않기로 한 것

| # | 결정 | 근거 |
|---|---|---|
| D1 | Cypher가 `og_csr_reach` 로 자동 라우팅하지 않는다 | MVCC/RLS를 조용히 포기할 수 없다. CSR은 노출·측정·문서화만 |
| D2 | Cypher가 `og_reach_sql` 을 방출하지 않는다 | 프론티어가 겹치는지 여부를 공짜로 알려 주는 통계가 없다 ([`docs/deep-traversal.md`](../../deep-traversal.md)) |
| D3 | 손익분기 규칙을 `|V|` 가 아니라 고정 walk 수로 둔다 | 격자에서 `|V|` 규칙이 틀렸다는 것이 측정으로 드러났다 ([`compile.rs:60-67`](../../../engine/src/cypher/compile.rs)) |
| D4 | `WITH` 가 있으면 무조건 열거로 되돌린다 | 이 패스가 `WITH` 안을 보지 않기 때문. 안전하지만 과하게 보수적 → [`PERF-07`](07_improvements_performance.md) |

<!-- affects: backend, api -->
<!-- requires-update: docs/deep-traversal.md, docs/01_architecture/09_performance/06_regression_guard.md -->
