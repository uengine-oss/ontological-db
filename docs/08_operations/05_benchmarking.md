# 벤치마크 운영

> **이 문서가 답하는 질문**
> - 벤치마크를 어떤 명령으로 돌리는가? 옵션은 정확히 무엇이 있는가?
> - "정확성 게이트"가 무엇이고, 통과하지 못하면 어떤 결과가 나오는가?
> - 결과 JSON은 어떤 스키마인가? 무엇을 읽어야 하는가?
> - Studio의 리포트 페이지는 어떻게 이 파일들과 연결되는가?
> - 회귀 게이트를 CI처럼 쓰려면?

---

## 사실 (Facts) — 하네스의 설계 원칙

`bench/harness.py:1-27`의 docstring과 `bench/README.md:7-44`가 규칙을 명시한다.
운영자가 결과를 읽을 때 반드시 알아야 할 것들:

1. **답을 먼저 맞춰보고 그다음에 시간을 보고한다.** 시스템들이 다른 답을 내면
   해당 질의의 타이밍은 **void**로 표시된다 (`bench/harness.py:1101-1102`).
   `bench/README.md:15-20`은 이 게이트가 실제로 버그를 잡은 적이 있다고 기록한다.
2. **하나의 열린 연결에서 시간을 잰다.** 질의마다 프로세스를 띄우면 약 12ms가 붙어
   모든 시스템이 똑같아 보인다 (`bench/README.md:22-26`).
3. **프로토콜 바닥값도 함께 게시한다.** 사소한 질의 하나의 비용이며,
   결과 파일의 `protocol_floor_ms`에 시스템별로 기록된다 (`bench/README.md:28-31`).
4. **논리 페이지 접근 수를 지연시간 옆에 적는다.** 지연시간은 캐시 상태에 따라 움직이지만
   페이지 접근은 저장 레이아웃의 직접 함수다 (`bench/README.md:37-39`) —
   결과의 `buffers` 필드.
5. **Ontological 행이 두 개다.** `ontological`은 사용자용 Cypher 경로,
   `ontological_raw`는 저장 접근 경로. 둘의 차이가 질의 엔진 오버헤드이며,
   숨기지 않기 위해 둘 다 게시한다 (`bench/README.md:41-44`).
6. **구조 무결성이 결과의 일부다.** 실행 끝에 `og_check_integrity()`를 세고
   `integrity_violations`로 기록한다 (`bench/harness.py:1173-1175`).

---

## 실행

### 기본

```bash
python3 bench/harness.py                                   # 기본 20,000 노드
python3 bench/harness.py --scale 50000 --degree 20 --runs 8
python3 bench/harness.py --systems ontological,cte         # AGE 건너뜀
python3 bench/harness.py --compare-baseline bench/results/baseline.json
```

(`bench/README.md:48-53`)

### CLI 옵션 — 전부 (`bench/harness.py:1247-1266`)

| 옵션 | 기본값 | 의미 |
|---|---|---|
| `--scale` | `20000` | 노드 수. `--shape grid`일 때는 정사각 격자에 맞춰 반올림된다 (`bench/harness.py:1002-1004`) |
| `--degree` | `8` | 평균 out-degree |
| `--runs` | `10` | 반복 횟수. 깊은 홉(4 이상)에서는 `min(runs, 3)`으로 줄인다 (`:1138`) |
| `--systems` | `ontological,ontological_raw,age,age_explicit,cte,neo4j,typedb` | 쉼표 구분 |
| `--hops` | `2,3` | 측정할 순회 깊이. 예: `2,3,4,5,6,8` |
| `--shape` | `random` | `random` \| `chain` \| `grid`. chain/grid는 지름이 커서 "깊이"라는 질문이 의미를 갖는 유일한 형태 |
| `--workload` | `classic` | `classic`: 게시된 1/2/3홉, 경로당 1행. `reach`: 시작 노드를 뺀 도달 노드 — 모든 시스템이 동일하게 진술할 수 있는 유일한 질문 |
| `--query-timeout` | `120` | 문장당 초 단위 캡. 초과한 시스템은 그렇게 기록되고 **그보다 깊은 것은 묻지 않는다** |
| `--compare-baseline` | `None` | 회귀 게이트. 지정 시 비교 결과가 종료 코드가 된다 |

> **금지**: 위 표에 없는 옵션은 존재하지 않는다. `--output`, `--label`, `--json` 같은 것을
> 지어내지 말 것. (`--label`은 `bench/csr/deep.py`에는 있고 `harness.py`에는 없다.)

### 등록된 시스템 (`bench/harness.py` `SYSTEMS`)

`ontological`, `ontological_raw`, `age`, `age_explicit`, `cte`, `pggraph`, `neo4j`, `typedb`.

> **`pggraph`는 `--systems` 기본값에 포함되어 있지 않다.** 측정하려면 명시해야 한다:
> `--systems ontological,pggraph`. 알 수 없는 이름은
> `! unknown system '<name>'`을 stderr로 내고 건너뛴다 (`bench/harness.py:1023-1024`).

### 환경변수

| 변수 | 기본값 | 근거 |
|---|---|---|
| `OG_PSQL` | `psql` | `bench/harness.py:45` |
| `PGHOST` | `localhost` | `:46` |
| `PGPORT` | `28816` | `:47` |
| `NEO4J_URI` | `bolt://localhost:27687` | `bench/harness.py:25`, `bench/README.md:68` |
| `NEO4J_USER` / `NEO4J_PASSWORD` | — | 동일 |
| `TYPEDB_ADDR` | `localhost:21729` | `bench/harness.py:26` |
| `TYPEDB_USER` / `TYPEDB_PASSWORD` | — | 동일 |

### 외부 시스템 준비 (`bench/README.md:61-66`)

```bash
docker run -d --name bench-neo4j  -p 27687:7687 \
  -e NEO4J_AUTH=neo4j/benchpass123 neo4j:5
docker run -d --name bench-typedb -p 21729:1729 typedb/typedb:latest
pip install neo4j typedb-driver
```

- Apache AGE가 설치되어 있지 않으면 `age` 시스템은 **실패가 아니라 안내와 함께 건너뛴다**
  (`bench/README.md:55-56`, `bench/harness.py:1026-1029`).
- `neo4j` / `typedb`도 서버나 드라이버가 없으면 같은 방식으로 건너뛴다.

---

## 정확성 게이트 — 무엇이 일어나는가

타이밍 측정 **전에** 시작 노드 하나로 모든 질의를 각 시스템에 한 번씩 물어본다
(`bench/harness.py:1057-1102`).

- 모든 시스템이 같은 답을 내면 `correctness[label].agree = true`
- 하나라도 다르면 화면에 다음을 출력한다:
  ```
  ! <label>: systems disagree {…} — timings for this query are VOID
  ```
  그리고 `agree = false`로 기록된다. **하네스는 계속 실행되지만 그 질의의 숫자는 무효다.**
- 요약 끝에서도 다시 알린다 (`bench/harness.py:1214`):
  ```
  !! systems disagreed on ['reach4hop'] — those timings are void
  ```
  모두 일치하면:
  ```
  all systems returned identical answers
  ```

**서버를 죽인 시스템은 결과다, 사고가 아니다** (`bench/harness.py:96-104`).
`Crashed` 예외가 잡히면 그 시스템은 더 깊은 질문을 받지 않고,
`wait_for_server()`가 postmaster 복귀를 기다린 뒤 다음 시스템으로 넘어간다 —
크래시 하나가 다른 시스템의 숫자를 조용히 무효화하는 것을 막기 위해서다.
크래시 판정 문자열은 `CRASH_SIGNS` (`bench/harness.py:108-115`):
`server closed the connection`, `terminated by signal`, `in recovery mode`,
`not yet accepting connections`, `crash of another server process`,
`connection to server was lost`.

**타임아웃도 결과다.** `--query-timeout`을 넘긴 시스템은 그 깊이에서
`{"timeout_s": N}`으로 기록되고, 그보다 깊은 질의는
`{"not_attempted": "exceeded …"}`가 된다 (`bench/harness.py:1129-1133`, `:1145-1148`).

---

## 결과 파일

### 이름과 위치

```
bench/results/bench-<nodes>-<YYYYMMDDTHHMMSSZ>.json
```

(`bench/harness.py:1178-1180`) — 예: `bench/results/bench-50000-20260817T033001Z.json`.

현재 커밋되어 있는 파일 (실측):

| 파일 | shape | 측정된 시스템 |
|---|---|---|
| `baseline.json` | (없음) | ontological, ontological_raw, age, cte, neo4j |
| `bench-5000-20260806T042903Z.json` | (없음) | ontological, ontological_raw, age, cte, neo4j, typedb |
| `bench-5000-20260806T043214Z.json` | (없음) | neo4j, typedb |
| `bench-5000-20260806T043920Z.json` | (없음) | age, age_explicit |
| `bench-5000-20260817T030411Z.json` | (없음) | ontological, ontological_raw, cte, age, age_explicit, pggraph, neo4j |
| `bench-50000-20260806T042833Z.json` | (없음) | ontological, ontological_raw, age, cte, neo4j |
| `bench-50000-20260806T043634Z.json` | (없음) | neo4j, typedb |
| `bench-50000-20260817T033001Z.json` | (없음) | ontological, ontological_raw, cte, age, age_explicit, pggraph, neo4j |
| `bench-50000-20260817T033525Z.json` | (없음) | ontological, age, neo4j |
| `bench-250000-20260817T051823Z.json` | `chain` | ontological, ontological_raw, cte, age, pggraph, neo4j |
| `bench-250000-20260817T052859Z.json` | `grid` | ontological, ontological_raw, cte, age, pggraph, neo4j |

> `scale.shape` 키는 하네스가 나중에 추가한 필드다. 2026-08-17T05 이후 파일에만 있다.
> 이전 파일은 전부 `random`으로 읽어야 한다.

> **`baseline.json`의 `generated_at`은 `bench-50000-20260806T042833Z.json`과 동일하다
> (`2026-08-06T04:24:05`).** 즉 baseline은 그 실행의 사본이다.

### JSON 스키마

`bench/harness.py:1007-1018`이 만드는 최상위 구조:

```jsonc
{
  "generated_at": "2026-08-17T03:19:23.870051+00:00",   // ISO8601 UTC
  "scale": {
    "nodes": 50000,
    "edges": 974936,
    "avg_degree": 20,          // len(edges)/nodes 를 반올림
    "shape": "random"          // 신형 파일에만 존재
  },
  "environment": {
    "postgres": "PostgreSQL 16.14 (Debian 16.14-1.pgdg12+1) on aarch64-…",
    "host": "localhost:28816"
  },
  "systems":  { … },           // 아래
  "correctness": { … },        // 아래
  "speedup_vs": { … },         // 아래
  "integrity_violations": 0    // og_check_integrity() 의 행 수
}
```

**`systems[<name>]`** (`bench/harness.py:1042-1049`, `:1105-1107`):

```jsonc
{
  "engine": "PostgreSQL 16.14 (Debian 16.14-1.pgdg12+1)",
  "reuses": null,              // 다른 시스템의 DB를 재사용하면 그 이름
  "load_seconds": 6.02,        // reuses 가 있으면 null — 적재하지 않았으므로
  "load_edges_per_sec": 161852,
  "protocol_floor_ms": 0.023,
  "queries": {
    "reach1hop": { "median_ms": 1.519, "p95_ms": 1.588, "min_ms": 1.471,
                   "runs": 5, "buffers": 2070 },
    …
  }
}
```

시스템이 살아 있지 않은 경우 `queries` 대신 다음 중 하나가 들어간다:

| 형태 | 의미 | 근거 |
|---|---|---|
| `{"skipped": "extension not installed"}` | 확장/서버/드라이버 없음 | `bench/harness.py:1028` |
| `{"error": "<setup 실패 메시지>"}` | 적재 실패 | `:1038` |

질의 셀에 들어갈 수 있는 형태:

| 형태 | 의미 |
|---|---|
| `{"median_ms": …, "p95_ms": …, "min_ms": …, "runs": …, "buffers": …}` | 정상 측정. `buffers`는 PostgreSQL 계열에만 |
| `{"timeout_s": 120}` | `--query-timeout` 초과 |
| `{"not_attempted": "exceeded 120s at reach5hop"}` | 더 얕은 깊이에서 이미 캡을 넘겨 시도하지 않음 |
| `{"crashed": "…"}` | 서버를 죽임 |
| `{"error": "…"}` | 그 외 오류 |

**`correctness[<label>]`** (`bench/harness.py:1096-1099`):

```jsonc
{
  "answers": { "ontological": "15", "cte": "15", "neo4j": "15", … },
  "agree": true
}
```

`answers`의 값은 **문자열**이다. 오류는 `"error: …"`로 시작하며,
`agree` 계산에서 제외된다 (`bench/harness.py:1095`).

**`speedup_vs[<other>][<query>]`** (`bench/harness.py:1160-1171`):
`ontological` 대비 배수 = `other.median_ms / ontological.median_ms`.
비교 대상은 `age`, `cte`, `ontological_raw`, `neo4j`, `typedb` 다섯 개로 고정되어 있다 —
**`pggraph`는 이 표에 들어가지 않는다.**

### 결과 읽는 순서 (권장)

```bash
# 1. 정확성 게이트부터
python3 -c "
import json,sys
d=json.load(open(sys.argv[1]))
bad=[k for k,v in d['correctness'].items() if not v['agree']]
print('disagreements:', bad or 'none')
print('integrity_violations:', d.get('integrity_violations'))
" bench/results/bench-50000-20260817T033001Z.json

# 2. 그다음 지연시간
python3 -c "
import json,sys
d=json.load(open(sys.argv[1]))
for name,s in d['systems'].items():
    if 'queries' not in s: print(f'{name}: {s}'); continue
    floor=s.get('protocol_floor_ms')
    print(f'{name} (floor {floor} ms)')
    for q,m in s['queries'].items():
        print('   ', q, m.get('median_ms', m))
" bench/results/bench-50000-20260817T033001Z.json
```

---

## 회귀 게이트

```bash
python3 bench/harness.py --compare-baseline bench/results/baseline.json
```

동작 (`bench/harness.py:1221-1240`):

- 임계값은 `threshold = 1.20` — **20% 이상 느려지면 실패**.
- 비교 대상은 baseline과 현재 실행 **양쪽에 `median_ms`가 있는 셀만**.
  한쪽에 없는 셀(타임아웃, skipped 등)은 조용히 건너뛴다.
- 실패 시 출력:
  ```
  REGRESSION detected:
    ontological/reach3hop: 33.86 → 41.20 ms (1.22× slower)
  ```
  그리고 프로세스가 **종료 코드 1**로 끝난다 (`bench/harness.py:1272`).
- 통과 시: `no regression against baseline`, 종료 코드 0.

> **필수**: `bench/results/baseline.json`은 **의도적으로만** 갱신한다.
> `bench/README.md:116-120`이 명시한다 — "update it deliberately, never as a side effect."
> 마지막 갱신은 2026-08-06이며, 하네스가 프로퍼티/엣지 엔드포인트 인덱스를 만들기 시작한
> 시점이다. 그 이전 baseline의 숫자는 이후 어떤 것과도 비교 가능하지 않다.

> **비교의 전제**: baseline은 `--scale 50000 --degree 20` 실행에서 나왔다
> (`bench/results/baseline.json`의 `scale`). **다른 스케일로 실행한 결과를 이 baseline과
> 비교하는 것은 무의미하다.** 게이트를 쓸 때는 baseline과 같은 `--scale`/`--degree`를 줄 것.

---

## 깊은 순회 전용 벤치 — `bench/csr/`

일반 하네스와 별개로, "k홉 안의 노드"를 네 가지 방법으로 재는 스위트가 있다
(`bench/csr/README.md:9-14`):

| variant | 함수 | 일이 일어나는 곳 |
|---|---|---|
| `vlp` | `og_vlp()` | 재귀 CTE, trail 열거, `int8[]` 경로 |
| `reach_sql` | `og_reach_sql()` | 경로 배열 없는 같은 CTE, `UNION ALL` 대신 `UNION` |
| `reach` | `og_reach()` | Rust BFS + 방문집합, SPI로 인접 읽기 |
| `csr` | `og_csr_reach()` | 백엔드-로컬 컴파일 CSR — SPI도 힙도 플래너도 없음 |

픽스처 생성과 실행 (`bench/csr/README.md:22-43`, 그대로):

```bash
createdb bench_csr    && psql -d bench_csr    -v nodes=50000  -v degree=20 -f gen.sql
createdb bench_sparse && psql -d bench_sparse -v nodes=200000 -v degree=4  -f gen.sql

createdb bench_chain && psql -d bench_chain -v shape=chain -v nodes=1000000 -f gen_shape.sql
createdb bench_grid  && psql -d bench_grid  -v shape=grid  -v nodes=1000000 -f gen_shape.sql
python3 deep.py --db bench_chain --depths 10,100,1000,10000,100000 --label chain
python3 deep.py --db bench_grid  --depths 10,20,50,100,500,1000    --label grid

python3 deep.py --db bench_csr    --depths 1,2,3,4,5,6 --starts 5 --label dense
python3 deep.py --db bench_csr    --depths 7,8,10,16,20 --variants reach_sql,reach,csr --label dense-deep
python3 deep.py --db bench_sparse --depths 1,2,3,4,5,6 --starts 5 --label sparse
python3 deep.py --db bench_sparse --depths 8,10,12,16,20 --variants reach_sql,reach,csr --label sparse-deep

psql -d bench_csr -f cypher_ab.sql
```

`deep.py` 옵션 (`bench/csr/deep.py:109-115`):
`--db`(기본 `bench_csr`), `--depths`(기본 `1,2,3,4,5,6`), `--starts`(기본 5),
`--timeout`(기본 120), `--variants`, `--label`.

`deep.py`는 실행마다 `results/` 아래 JSON을 하나 쓰며 **각 변형이 낸 답까지 기록한다.**
불일치가 있으면 종료 코드가 0이 아니다 (`bench/csr/README.md:45-47`).

`bench/pggraph_cost.sql`은 pgGraph의 시간이 순회에 쓰이는지 행 반환에 쓰이는지를 가르는
스크립트다. 헤더가 규칙을 못 박는다 (`bench/pggraph_cost.sql:6-11`):
**어떤 pgGraph 순회 숫자든 인용하기 전에 이것을 먼저 돌릴 것 — 우리 숫자를 포함해서.**

```bash
python3 bench/harness.py --scale 50000 --degree 20 --systems pggraph --hops 5
psql -d bench_pggraph -f bench/pggraph_cost.sql
```

(`bench/pggraph_cost.sql:3-4`)

---

## Studio 리포트 페이지 연결

```
http://localhost:7474/benchmark.html
```

파이프라인:

1. `portal/server/index.js:86-138` `readBenchmarks()`가 `OG_BENCH_DIR`(기본
   `bench/results`)를 직접 읽는다.
2. `/^bench-(\d+)-(\d{8}T\d{6}Z)\.json$/`에 맞는 파일만 대상 —
   `baseline.json`은 제외된다 (`portal/server/index.js:91`).
3. 타임스탬프 문자열 오름차순으로 정렬한 뒤, **스케일별로 시스템 단위 newest-wins 병합**
   (`:93-114`). `queries`가 없는 시스템 항목(skipped/error)은 좋은 실행을 덮어쓰지 않는다
   (`:111`).
4. `correctness`는 병합된 `systems`에 남아 있는 시스템만 대상으로 재계산된다 (`:121-132`).
5. 반쯤 쓰인 JSON은 `try/catch`로 무시하고 페이지를 살린다 (`:100-102`).
6. `GET /api/benchmark`가 그 결과를 그대로 반환하고 `portal/web/benchmark.js`가 렌더한다.

`bench/README.md:73-76`이 이 설계의 요지를 적는다:
디렉터리를 직접 읽고 시스템별 최신 실행을 취하므로,
**하네스 실행이 끝나는 행위 자체가 게시다** — 페이지에 손으로 옮겨 적는 숫자가 없다.

> **운영상 함의**: 리포트 페이지가 이상하면 먼저 `OG_BENCH_DIR`을 의심할 것.
> 파일 이름 규칙에서 벗어난 결과 파일은 **조용히 무시된다.**

---

## 문서와 결과 파일의 대응

`docs/benchmark.md:396-401`이 게시된 표의 출처 파일을 명시한다:

| | 5,000 nodes | 50,000 nodes |
|---|---|---|
| Ontological, raw, CTE | `bench-5000-20260806T042903Z` | `bench-50000-20260806T042833Z` |
| AGE, AGE explicit | `bench-5000-20260806T043920Z` | `bench-50000-20260806T052220Z` |
| Neo4j, TypeDB | `bench-5000-20260806T043214Z` | `bench-50000-20260806T043634Z` |

> **실측 불일치**: 이 중 `bench-50000-20260806T052220Z.json`은 `bench/results/`에
> **존재하지 않는다.** 나머지 5개는 존재한다. 즉 게시된 50,000노드 AGE 열은
> 저장소만으로 재현·검증할 수 없다. → [10_improvements_ops.md](10_improvements_ops.md) `OPS-15`

측정 환경은 `docs/benchmark.md:129-141`에 있다 —
Apple silicon(arm64), macOS, Docker Desktop, PostgreSQL 16.14, AGE 1.5.0,
Neo4j 5.26.28 Community(heap 2GB / page cache 1GB), TypeDB 3.12.1,
psql 18.4 / `neo4j` 파이썬 드라이버 6.2.0 / `typedb-driver` 3.12.1.
**모두 커뮤니티·기본 설정이며 어떤 엔진도 이 워크로드에 맞춰 튜닝되지 않았다.**

---

## 금지 / 필수

### 금지 (Forbidden)

- `agree: false`인 질의의 `median_ms`를 인용하지 말 것 — 정의상 무효다.
- `protocol_floor_ms`를 보지 않고 1홉 숫자를 비교하지 말 것.
  Bolt 드라이버와 psql의 왕복 비용이 다르며, 1홉 답의 큰 부분을 차지한다.
- 다른 `--scale`/`--degree`로 낸 결과를 현 baseline과 비교하지 말 것.
- `baseline.json`을 실행의 부수효과로 갱신하지 말 것.
- `bench/results/`의 파일명을 손으로 바꾸지 말 것 — Studio가 정규식으로 걸러 무시한다.

### 필수 (Required)

- 결과를 인용할 때는 **파일 이름**을 함께 적을 것.
- pgGraph 숫자를 인용하기 전에 `bench/pggraph_cost.sql`을 돌릴 것.
- 회귀 게이트를 돌릴 때는 baseline과 동일한 스케일 인자를 줄 것:
  ```bash
  python3 bench/harness.py --scale 50000 --degree 20 \
    --compare-baseline bench/results/baseline.json
  ```

---

## 하네스가 아직 하지 않는 것 (`bench/README.md:122-131`)

- LDBC SNB (spec 009 phase 2) — 생성기가 배선되어 있지 않음
- openCypher TCK 통과율 추적 (phase 1)
- 결함 주입 및 장기 스트레스 (phase 4)
- **동시성** — 이 결과의 모든 질의는 혼자 실행되었다
- TypeDB 순회의 재귀 함수 변형

"측정했는데 뺀 것이 아니라 하네스의 공백"이라고 명시되어 있다.

---

<!-- affects: ops, backend -->
<!-- requires-update: docs/08_operations/06_monitoring.md -->
