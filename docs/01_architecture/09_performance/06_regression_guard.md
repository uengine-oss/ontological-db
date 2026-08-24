# 회귀 방어 — 있는 장치와 그 공백

> **이 문서가 답하는 질문**
> - 성능 회귀를 실제로 막고 있는 장치는 무엇인가?
> - 정확성 게이트는 무엇을 검사하고, 무엇을 검사하지 않는가?
> - 회귀 비교(`--compare-baseline`)는 무엇을 놓치는가?
> - 지금 어떤 회귀가 **아무 경보 없이** 통과할 수 있는가?

---

## 1. 사실 — 지금 있는 장치

| 장치 | 위치 | 무엇을 보장하는가 |
|---|---|---|
| **정확성 게이트** | [`harness.py:1058-1096`](../../../bench/harness.py) | 모든 시스템의 답을 먼저 비교하고, 다르면 그 질의의 타이밍을 VOID로 표시 |
| **프로토콜 바닥값** | [`harness.py:1097-1108`](../../../bench/harness.py) | 클라이언트 경로의 고정 비용을 결과 옆에 기록 |
| **논리 페이지 접근** | [`harness.py:183-206`](../../../bench/harness.py) | 캐시 상태와 무관한 저장 구조의 직접 지표 |
| **비대칭 워밍업** | [`harness.py:135-147`](../../../bench/harness.py) | 드라이버 워밍업(bolt 50회)과 psql(무영향)을 구분해 처리 |
| **문당 타임아웃 + 포기 규칙** | [`harness.py:148-153, 1132-1152`](../../../bench/harness.py) | "안 끝났다"를 결측이 아니라 측정으로 기록 |
| **백엔드 크래시 감지** | [`harness.py:107-132, 1071-1082`](../../../bench/harness.py) | 한 시스템의 크래시가 다른 시스템의 수치를 오염시키지 않게 함 |
| **회귀 비교** | [`harness.py:1221-1241`](../../../bench/harness.py) | 베이스라인 대비 20% 이상 느려지면 종료 코드 1 |
| **구조 무결성** | [`harness.py:1155-1157`](../../../bench/harness.py) → `og_check_integrity()` | 벤치 끝에 위반 수를 결과에 기록 |
| **깊은 순회 답 기록** | [`bench/csr/deep.py`](../../../bench/csr/deep.py) | 변형별 답을 JSON에 남기고 불일치 시 종료 코드 ≠ 0 |
| **의미 조건 회귀 파일** | [`engine/tests/sql/05_reachability.sql`](../../../engine/tests/sql/05_reachability.sql) | 재작성이 언제 일어나고 언제 일어나면 안 되는지 6가지 경우 |
| **A/B 스크립트** | [`bench/csr/cypher_ab.sql`](../../../bench/csr/cypher_ab.sql) | 하나의 바이너리·하나의 연결에서 재작성 유무를 나란히 비교 |

**이 정도의 자기 검증은 드물다.** 정확성 게이트가 있는 그래프 벤치마크는 흔하지 않고,
페이지 수를 지연과 함께 싣는 것도 그렇다. 아래는 그 위에서의 공백이다.

## 2. 공백 — SQL 회귀 스위트가 값을 검사하지 않는다 (심각)

[`tests/run.sh:14-36`](../../../tests/run.sh) 은 각 `.sql` 파일을 돌리고 **`ERROR` 줄의 개수만 센다.**

```bash
out="$(psql … -f "$f" 2>&1)"
expected="$(grep -c 'EXPECT_ERROR' "$f" || true)"
actual="$(printf '%s' "$out" | grep -c '^ERROR\|^psql.*ERROR' || true)"
if [ "$actual" -le "$expected" ]; then echo "ok" …
```

기대 출력과의 비교가 없다. `engine/tests/pg_regress/expected/` 에 있는 것은 `setup.out` 하나뿐이다.

결과적으로 [`engine/tests/sql/05_reachability.sql`](../../../engine/tests/sql/05_reachability.sql) 의
다음 단언들은 **`f` 를 출력해도 테스트가 통과한다**:

```sql
SELECT og_cypher_sql('r', $$…count(DISTINCT y)$$) LIKE '%og_reach(%' AS count_distinct_uses_reach;
SELECT og_cypher_sql('r', $$…count(y)$$)          LIKE '%og_vlp(%'   AS plain_count_keeps_vlp;
SELECT og_cypher_sql('r', $$…-[e:E*1..12]->…$$)   LIKE '%og_vlp(%'   AS rel_variable_keeps_vlp;
SELECT og_cypher_sql('r', $$…*1..2…$$)            LIKE '%og_vlp(%'   AS shallow_keeps_vlp;
```

[`docs/deep-traversal.md`](../../deep-traversal.md) 은
*"The regression suite asserts every one of these cases, in both directions"* 라고 쓰지만,
**파일이 그 경우들을 서술할 뿐 러너가 단언하지는 않는다.**
전환 판정이 조용히 망가져도(예: `WALKS` 상수를 잘못 고쳐도) 스위트는 초록불이다.
→ [`PERF-19`](07_improvements_performance.md).

## 3. 공백 — 베이스라인이 낡았고 좁다

[`bench/results/baseline.json`](../../../bench/results/baseline.json):

- 생성 시각 **2026-08-06T04:24:05Z**. 그 이후에 깊은 순회 재작성이 들어갔다.
- 담고 있는 질의는 `1hop / 2hop / 3hop / prop_scan` **뿐**이다.
  2026-08-17에 추가된 `reach*` 워크로드에는 **베이스라인이 없다.**
  즉 이 프로젝트에서 가장 크게 바뀐 경로가 회귀 게이트 밖에 있다.
- 담고 있는 시스템은 `ontological, ontological_raw, age, cte, neo4j`.
  `pggraph`, `age_explicit`, `typedb` 는 없다.
- `correctness` 항목이 `1hop / 2hop / prop_scan` 뿐이다 — 현재 하네스 코드
  ([`harness.py:1059-1064`](../../../bench/harness.py))가 만드는 목록과 다르므로
  **이 파일은 현재 하네스로 재생성되지 않는다.**

## 4. 공백 — 회귀 비교가 놓치는 것

[`harness.py:1221-1241`](../../../bench/harness.py):

```python
for name, cur_sys in current["systems"].items():
    for q, m in cur_sys.get("queries", {}).items():
        b = base…get(q, {})
        if "median_ms" not in m or "median_ms" not in b or b["median_ms"] <= 0:
            continue                       # ← 조용히 건너뛴다
        ratio = m["median_ms"] / b["median_ms"]
        if ratio > 1.20: failures.append(…)
```

| # | 놓치는 것 | 왜 문제인가 |
|---|---|---|
| G1 | **`buffers` 를 비교하지 않는다** | 페이지 수는 이 프로젝트의 저장 구조 주장 그 자체다. 지연이 캐시 덕에 유지되어도 페이지 수는 배로 늘 수 있다 |
| G2 | **질의가 베이스라인에 없으면 그냥 건너뛴다** | 질의 이름을 바꾸거나 워크로드를 바꾸면 게이트가 조용히 무력해진다 (지금 `reach*` 가 정확히 그 상태다) |
| G3 | **하한(floor)이 없다** | 0.05 → 0.061 ms 도 22% 회귀로 실패한다. 서브밀리초 셀에서는 노이즈 |
| G4 | **개선을 반영하지 않는다** | 베이스라인이 자동으로 조여지지 않으므로, 한 번 좋아진 뒤 다시 나빠져도 20% 안이면 통과 |
| G5 | **로드 처리량을 보지 않는다** | 같은 데이터에서 124,580 대 161,852 e/s (30% 차이)가 아무 경보 없이 지나갔다 |
| G6 | **정확성 결과를 보지 않는다** | `compare()` 는 `correctness` 를 읽지 않는다. 답이 틀려도 빨라졌으면 통과 |
| G7 | **`integrity_violations` 를 보지 않는다** | 기록만 되고 게이트가 되지 않는다 |

## 5. 공백 — 측정 방식 자체의 비대칭

[`02_measured_baselines.md` §8](02_measured_baselines.md) 에서 자세히 다룬 내용의 요약:

- 지연은 **웜 세션의 중앙값**, 페이지 수는 **새로 띄운 psql 프로세스의 첫 호출 1회**다
  ([`harness.py:281-287`](../../../bench/harness.py)).
- 이 비대칭은 Rust 쪽 컴파일 경로를 가진 `ontological` 행에만 불리하게 작용한다.
- 증거: `ontological` 의 `prop_scan` 페이지 수가 5,000 / 50,000 / 250,000 노드에서
  1,170 / 1,173 / 1,177 로 **데이터 크기와 무관하게 일정**하다.

그리고 [`02_measured_baselines.md` §9](02_measured_baselines.md) 의 미해결 관측:
`bench-5000-20260817T030411Z.json` 에서 `ontological` 과 `ontological_raw` 의 페이지 수가
네 깊이 모두에서 사실상 동일하다. 두 행이 서로 다른 것을 재고 있다는 전제가 그 런에서는 성립하지 않는다.
**원인 미확인이며, 이것을 잡아 줄 장치가 하네스에 없다.**

## 6. 공백 — 정확성 게이트가 검사하지 않은 것

### 6.1 공개된 3홉 수치는 답이 검증되지 않은 런에서 나왔다

2026-08-06 런들의 `correctness` 항목은 전부 `1hop / 2hop / prop_scan` 뿐이다(§3).
`README.md` 와 `docs/benchmark.md` 가 싣는 3홉 수치
(Ontological 33.86 ms / AGE 22,412 ms / Neo4j 2.99 ms)는 그 런들에서 나왔다.
`docs/benchmark.md` 스스로도 검증된 답으로 1홉과 2홉만 인용한다
(*"1 hop = 8, 2 hops = 75 … 15 and 359"*).

3홉이 게이트를 통과한 기록은 2026-08-17 런에 처음 나타난다
([`bench-50000-20260817T033525Z.json`](../../../bench/results/bench-50000-20260817T033525Z.json)):
`{'ontological': '6696', 'age': '6696', 'neo4j': '6696'}`, `agree: true`.
**결과적으로는 맞았지만, 공개 시점에는 검증되지 않은 수치였다.**

### 6.2 `minhop > 1` 이 한 번도 검사되지 않았다

- 하네스: `*1..k` 만 ([`harness.py:363-373`](../../../bench/harness.py))
- `deep.py` / `cypher_ab.sql`: `*1..k` 만
- `05_reachability.sql`: `minhop = 1` 만

[`04_deep_traversal_mechanics.md` §4.1](04_deep_traversal_mechanics.md) 에 적은 대로,
`minhop > 1` 에서 `og_vlp` 와 `og_reach` 는 **다른 집합을 낸다**(코드 분석 기준).
정확성 게이트는 이것을 볼 기회가 없었다. → [`PERF-20`](07_improvements_performance.md).

### 6.3 두 벤치가 서로 다른 접근 경로를 잰다

- `bench/harness.py` — `WHERE a.val = {start}` → `IS NOT DISTINCT FROM` (인덱스 미사용으로 추정)
- `bench/csr/cypher_ab.sql` — `(a:P {val:7})` → `p_val = 7` (인덱스 사용 가능)

같은 저장소의 두 성능 자료가 다른 것을 재고 있다. → [`PERF-01`](07_improvements_performance.md).

## 7. 공백 — 자동화가 없다

- 저장소에 CI 설정 파일이 없다 (`.github/`, `*.yml` 워크플로 없음 — `find` 로 확인).
  `--compare-baseline` 은 **사람이 기억해서 돌려야 하는 명령**이다.
- 벤치를 돌리려면 Docker + AGE + Neo4j + TypeDB + pgGraph가 필요하다.
  전부 없으면 하네스는 해당 시스템을 조용히 건너뛰고, 게이트는 남은 것만 본다.
- `tests/run.sh` 도 자동으로 돌지 않는다.

## 8. 공백 — 아예 측정 대상이 아닌 것

| 영역 | 상태 |
|---|---|
| 쓰기 경로 (`og_create_node` / `og_create_edge` / Cypher `CREATE`) | **벤치 없음** |
| 동시성 (동시 읽기, 동시 쓰기, `og_id_alloc` 경합) | **벤치 없음** — 모든 측정이 단독 실행 |
| 콜드 캐시 | 없음 |
| 스큐 그래프(허브·파워로우) | 없음. 랜덤·사슬·격자 세 모양뿐 |
| 벡터 검색 / HNSW recall | `og_vector_search_exact` 라는 기준 구현은 있으나 벤치가 없음 |
| Bolt 게이트웨이 처리량 | 없음 |
| Studio 서버 | 없음 |
| LDBC SNB, openCypher TCK, 폴트 인젝션 | [`bench/README.md`](../../../bench/README.md) "Not yet implemented" 에 명시 |

## 9. 결정 — 지금 상태에서 지켜야 할 규칙

**필수**

- ✅ 핫패스(`compile.rs`, `traverse.rs`, `adjacency.rs`, `access.sql`)를 고쳤으면
  다음 두 명령을 모두 돌리고 결과를 커밋 메시지에 남긴다.
  ```bash
  bash tests/run.sh
  python3 bench/harness.py --scale 50000 --degree 20 --compare-baseline bench/results/baseline.json
  ```
- ✅ 깊은 순회 경로를 고쳤으면 정확성 비교까지 돌린다.
  ```bash
  python3 bench/csr/deep.py --db bench_csr --depths 1,2,3,4,5,6 --starts 5 --label dense
  psql -d bench_csr -f bench/csr/cypher_ab.sql
  ```
- ✅ 베이스라인을 갱신할 때는 **의도적으로**, 별도 커밋으로, 이유를 적어서 한다
  ([`bench/README.md`](../../../bench/README.md) 의 "update it deliberately, never as a side effect").

**금지**

- ❌ `tests/run.sh` 가 통과했다는 이유만으로 의미 조건이 유지된다고 주장하는 것 (§2).
- ❌ 베이스라인에 없는 워크로드(`reach*`)의 회귀가 게이트에 걸린다고 가정하는 것 (§3).
- ❌ 지연만 보고 회귀가 없다고 판단하는 것. 페이지 수를 함께 본다 (§4 G1).

<!-- affects: ops, backend -->
<!-- requires-update: bench/README.md, docs/deep-traversal.md -->
