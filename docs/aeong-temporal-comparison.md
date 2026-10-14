# AeonG 직접 돌려 보기 — 시간 그래프 DB 튜토리얼과 Ontological 비교

> **이 문서가 답하는 질문**
> - Gemini 노트북 보고서(`bench/온톨로지 데이터베이스의 시간과 스키마 관리 - Gemini Notebook.pdf`)가 소개한 시간 그래프 모델의 핵심은 무엇인가?
> - 그중 "OLTP 시점 복원의 정답"으로 추천된 **AeonG**(PVLDB 2024)는 실제로 어떻게 설치하고, 어떻게 과거를 묻는가?
> - 같은 이야기를 AeonG와 Ontological에서 돌리면 답이 어떻게 다른가?
> - **우리가 지는 곳은 어디인가?**

재현 파일:
- AeonG 빌드: [`bench/aeong/Dockerfile`](../bench/aeong/Dockerfile)
- AeonG 시나리오: [`bench/aeong/aeong_scenario.py`](../bench/aeong/aeong_scenario.py)
- AeonG 결함 최소 재현: [`bench/aeong/aeong_probes.sh`](../bench/aeong/aeong_probes.sh)
- Ontological 시나리오: [`bench/aeong/ontological_scenario.sql`](../bench/aeong/ontological_scenario.sql)
- 실행 결과 원본: `bench/results/aeong_*.out`

관련 문서: [바이템포럴 튜토리얼](tutorials/temporal-tutorial.md), [시간 기능 매뉴얼](guides/temporal.md), [ADR-027](99_decisions/ADR-027-bitemporal-current-plus-history.md)

---

## 0. 요약

AeonG는 저장소 `vldb` 브랜치 커밋 `b780a0c`를 소스에서 빌드해 Apple M-시리즈(arm64)에서 돌렸습니다. Ontological은 현재 작업 트리를 PG16 확장으로 빌드해 돌렸습니다. 두 시스템 모두 입사 → 연봉 인상 → 정정 → 퇴사로 이어지는 같은 이야기로 시험했습니다.

| 질문 | AeonG | Ontological |
|---|---|---|
| "T 시점에 DB는 무엇을 알고 있었나" (트랜잭션 시간) | `MATCH … TT AS t` — **노드 속성만 바뀐 경우에는 정답** | `og.system_at` — 정답 |
| "세상에서 그때 사실은 무엇이었나" (유효 시간) | **없음.** 문법 파일에 `VT` 절이 있지만 파서가 거절함 | `og.valid_at` — 정답 |
| 소급 입력, 오기 정정 | 표현 불가 (입력 시각 = 유효 시각) | `og.valid_from`, `'-infinity'` 정정 |
| 붙어 있던 **관계를 지운 뒤** 그 노드의 과거 조회 | **틀린 답.** 모든 과거 시점에 현재값을 돌려주고, 지운 관계는 사라짐 | 정답 |
| **노드를 지운 뒤** 그 노드의 과거 조회 | **빈 결과.** 이력 전체가 안 보임 | 정답 |
| 구간 이력 (`FROM t1 TO t2`) | Cypher 문법이 있음. 다만 존재한 적 없는 `NULL` 버전이 섞임 | Cypher 문법은 없음. SQL `og_timeline()`으로 조회 |
| 가변 길이 경로 · 라벨 없는 패턴의 시점 질의 | 문법상 허용됨. 관계가 지워진 뒤에는 빈 결과 | **명시적 오류** (`0A000 OG_TEMPORAL_UNSUPPORTED`) |
| 저장 구조 | 현재 = 인메모리. 과거 델타 = GC가 RocksDB로 이전 | 현재 = 힙 테이블. 과거 = `h_*` 테이블(트리거로 기록) |

**한 줄 결론.** AeonG가 논문에서 보인 강점은 저장 효율과 지연시간입니다. 기능 범위는 트랜잭션 시간 한 축뿐입니다. 그마저 관계나 노드가 삭제되는 순간 **조용히 틀린 답**을 냅니다. 보고서가 권한 "엔터프라이즈 OLTP 시점 복원"에 그대로 쓰기는 어렵습니다. 우리의 약점은 다른 곳에 있습니다.
- 시점 질의 Cypher 문법이 없습니다. 세션 설정으로 시점을 지정합니다.
- 시점 질의에서 가변 길이 경로를 지원하지 않습니다.
- 성능을 측정한 적이 없습니다.

---

## 1. 보고서(PDF) 핵심 정리

### 1.1 시간은 두 개다

| 축 | 의미 | 누가 정하나 | 보고서 표기 |
|---|---|---|---|
| **유효 시간** (valid time) | 현실에서 그 사실이 성립한 기간 | 비즈니스 (소급·미래 지정 가능) | `effective_from` / `effective_to` |
| **트랜잭션 시간** (transaction / system time) | DB가 그 사실을 기록하고 있던 기간 | 시스템 시계 (단조 증가, 수정 불가) | `tx_from` / `tx_to` |

두 축을 다 가진 모델을 **바이템포럴**이라고 합니다. 이 모델이어야 "그 시점에 시스템이 알고 있던 유효 상태"를 재현할 수 있습니다(감사 추적).

### 1.2 구간과 무결성

- **구간**은 반개구간 `[start, end)`입니다. 열린 끝은 `+∞`, `9999-12-31` 또는 `NULL`로 표현합니다.
- **시점(AS OF) 질의**는 `from ≤ t < to`를 묻는 포인트 스태빙입니다.
- **범위 질의**는 `from < T_end ∧ to > T_start`를 묻는 오버랩 검사입니다.
- **무결성 제약**: 관계의 수명은 양 끝 노드 수명의 교집합 안에 있어야 합니다. `Ω(e) ⊆ Ω(u) ∩ Ω(v)`

### 1.3 모델링 방식 (보고서 분류)

- **시간을 붙이는 단위**:
  - 노드 주석형
  - 엣지 주석형
  - 노드-엣지 주석형
  - 그래프 스냅샷형
- **설계 패턴**:
  - ① 속성에 구간을 두는 방식. 관계 폭발이 생깁니다.
  - ② 불변 Identity 노드와 Version 노드를 나누는 방식. 조회할 때마다 1홉이 더 듭니다.
  - ③ 같은 노드 쌍 사이에 구간별 엣지를 복제하는 방식. 프록시 노드를 씁니다.

### 1.4 성능 쟁점

- **인터벌 스태빙**: B-Tree를 `from`에만 걸면 후행 필터가 생깁니다. 해법은 R-Tree나 GiST 같은 2차원 색인입니다.
- **슈퍼노드 퇴화**: 만료된 엣지가 쌓입니다. 해법은 hot/cold 분리(AeonG 방식)나 Time-Tree 파티셔닝입니다.
- **시간 준수 경로**: A→B→C 경로는 엣지 시점이 단조 증가할 때만 성립합니다(ChronoGraph의 EAT 경로).

### 1.5 보고서가 AeonG에 대해 말한 것 — 검증 결과

| 보고서 서술 | 실제 (소스·실행으로 확인) |
|---|---|
| `FOR TT AS OF t`, `FOR TT FROM t1 TO t2` 구문 | 실제 문법은 `MATCH <패턴> [WHERE …] TT AS t` / `TT FROM t1 TO t2`입니다. `FOR`와 `OF`가 없습니다 (`src/query/frontend/opencypher/grammar/Cypher.g4:77-83`) |
| 유효 시간과 트랜잭션 시간을 결합 | 트랜잭션 시간만 동작합니다. `vt` 규칙은 문법 파일에만 있고, AST 방문자는 `tt()`만 읽습니다 (`cypher_main_visitor.cpp:1057`). 실제로 `VT AS 0`을 보내면 `mismatched input 'VT'` 오류가 납니다 |
| 현재 = 인메모리 포인터 체인, 과거 = GC가 RocksDB로 이전 | 맞습니다. `Storage::CollectGarbage()`가 `history_deltas/` RocksDB로 옮깁니다. `FREE MEMORY` 뒤에 `history_deltas/000003.log`이 생긴 것을 확인했습니다 |
| ACID 보장, 실시간 시점 복원 | 삭제가 섞이면 틀린 답이 나옵니다 (§3.4) |

---

## 2. 튜토리얼 A — AeonG 설치

### 2.1 공식 이미지가 안 되는 이유

README가 권하는 `hououou/aeong:v1`은 4.9 GB짜리 **CentOS 7 / amd64 빌드 환경**입니다. 바이너리는 들어 있지 않습니다. 이미지 안의 `/home/memgraph`는 AeonG가 아닌 원본 Memgraph 소스입니다.
- Apple Silicon에서 이 이미지를 돌리면 QEMU 에뮬레이션 아래에서 CentOS 7의 `yum`·`curl`·`sbcl`이 vsyscall 페이지 접근(`0xffffffffff600400`)으로 죽습니다.
- AeonG 저장소에서는 `libs/antlr4.patch`, `rocksdb.patch`, `librdtsc.patch`가 `.gitignore`로 빠져 있어서 `./init`가 실패합니다.

### 2.2 네이티브 빌드 (권장)

[`bench/aeong/Dockerfile`](../bench/aeong/Dockerfile)은 Ubuntu 22.04에서 빌드합니다. arm64와 amd64 모두 됩니다.
- **Boost**: 1.78을 소스에서 빌드합니다. 22.04에 들어 있는 것은 1.74입니다.
- **gflags**: Memgraph 포크를 쓰고, toolchain v4와 같은 커밋 `b37ceb0`에 고정합니다. `DEFINE_HIDDEN_*` 매크로가 있어야 하기 때문입니다.
- **빠진 패치**: 라이브러리 버전 고정이 똑같은 **Memgraph v2.3.1**에서 가져옵니다.
- **빌드 옵션**: `-DMG_ARCH=ARM64`를 줍니다. Apple Silicon(16코어)에서 저장소 클론부터 `make memgraph mgconsole`까지 7분 20초 걸렸습니다. apt·Boost·gflags 단계는 캐시된 상태에서 잰 값입니다.

```bash
docker build -t aeong -f bench/aeong/Dockerfile bench/aeong
docker run -d --name aeong -p 17687:7687 aeong                         # TT = 논리 타임스탬프
# docker run -d --name aeong -p 17687:7687 aeong --real_time_flag=true # TT = epoch 밀리초
echo "RETURN 1 AS x;" | docker exec -i aeong mgconsole --host 127.0.0.1 --output-format csv
```

> **드라이버 주의.** 이 빌드(Memgraph 2.3 기반)는 Python `neo4j` 4.4 드라이버의 Bolt 4.3 HELLO를 받으면 `Something went wrong during session execution!`를 내고 끊습니다. 그래서 이 문서의 모든 실행은 함께 빌드한 `mgconsole`로 했습니다.

### 2.3 AeonG 고유 설정

| 플래그 | 기본값 (소스) | 의미 |
|---|---|---|
| `--real_time_flag` | `false` | `false`면 TT가 **트랜잭션 커밋 카운터**입니다. `true`면 커밋 시각(epoch ms)입니다 |
| `--storage-gc-cycle-sec` | 30 | GC 주기입니다. GC가 돌 때 만료된 델타가 RocksDB로 이전됩니다 |
| `--retention_on_startup` / `--retention_period_sec` | `false` / 15 | 오래된 이력 삭제입니다. 기본으로는 꺼져 있습니다 |
| `--anchor_num` | 11 | 델타 체인 중간에 전체 버전(앵커)을 저장하는 간격입니다 |

기본(논리) 모드에서는 사용자가 "어제 오후 3시"를 물을 방법이 없습니다. TT가 커밋 카운터이기 때문입니다. 쓰기 한 번에 카운터가 2씩 오릅니다(시작과 커밋). 그 값을 알려 주는 함수도 없습니다. 이 문서에서 벽시계 기준 질의는 `--real_time_flag=true`로, 결함 재현은 기본 모드로 했습니다.

---

## 3. 튜토리얼 B — AeonG로 과거 묻기

### 3.1 문법

```cypher
MATCH (p:Person {name:'ann'}) TT AS 1791261693460 RETURN p.salary          -- 시점
MATCH (p:Person {name:'ann'}) TT FROM 0 TO 9999999999999 RETURN p.salary   -- 구간: 겹치는 모든 버전
```

- `TT` 절은 `MATCH` 하나에 붙습니다. 패턴과 `WHERE` 다음에 씁니다.
- 쓰기 절(`CREATE`, `SET`)에는 시간을 줄 수 없습니다. 기록되는 시간은 언제나 "지금"입니다.
- `MATCH … TT AS 0 SET …`도 오류 없이 받아들여집니다. 과거에 없던 노드라 매칭되는 것이 없어서 아무 일도 일어나지 않습니다.

### 3.2 되는 경우 — 노드 속성만 바뀔 때

```text
CREATE (:P {v:1})-[:R {w:'a'}]->(:C);   MATCH (p:P) SET p.v=2;   MATCH (p:P) SET p.v=3;

  TT AS 2    p.v=           r.w=           ← 생성 전
  TT AS 3    p.v="1"        r.w="a"
  TT AS 5    p.v="2"        r.w="a"
  TT AS 7    p.v="3"        r.w="a"
```

(`bench/results/aeong_probes.out` P1.) 벽시계 모드에서도 같습니다. `TT AS <A>`는 1을, `TT AS <B>`는 2를 돌려줍니다. `TT FROM 0 TO ∞`는 3, 2, 1을 돌려줍니다.

### 3.3 이야기 하나 — ann (벽시계 모드)

| 단계 | 쿼리 | 기록 시각 |
|---|---|---|
| T1 | `CREATE (:Person {name:'ann', salary:100})-[:WORKS_AT {title:'eng'}]->(:Company {name:'acme'})` | 입사 |
| T2 | `SET p.salary = 200` | 인상 |
| T3 | `SET p.salary = 250` | **정정**. AeonG에서는 "새 값"과 구분할 수 없습니다 |
| T4 | `MATCH (:Person)-[w:WORKS_AT]->() DELETE w` | 퇴사 |

AeonG에는 유효 시간이 없습니다. 그래서 "2023-01-01부터 인상"이라는 정보는 **어디에도 기록되지 않습니다**. 정정과 인상의 구분도 사라집니다.

### 3.4 깨지는 경우

**① 관계를 지우면 그 노드의 과거가 현재값으로 덮입니다.** 위 3.2의 그래프에서 `R` 하나만 지운 뒤 같은 질문을 다시 던졌습니다.

```text
  TT AS 3    p.v="3"        r.w=           ← 정답: v=1, r.w=a
  TT AS 5    p.v="3"        r.w=           ← 정답: v=2, r.w=a
  TT AS 7    p.v="3"        r.w=           ← 정답: v=3, r.w=a
```

- 오류도 경고도 없습니다. GC(`FREE MEMORY`)로 이력을 RocksDB로 옮긴 뒤에도 같습니다(P6).
- 논리 모드와 벽시계 모드 모두에서 재현됩니다.
- ann 시나리오(`aeong_scenario_keep_*.out`)에서도 T1~T5 전 시점이 `salary 250`으로 나왔습니다.

**② 노드를 지우면 이력 전체가 안 보입니다.**

```text
  삭제 전 구간: "8" "7" "NULL" "NULL" "NULL"
  삭제 후 구간: []
```

스캔 연산자가 **지금 존재하는 정점**을 순회합니다. 그 정점마다 과거 버전을 덧붙이는 구조입니다(`ScanAllCursor::Pull` → `addHistoryVertex`, `operator.cpp:481-523`). 그래서 지워진 정점은 출발점이 될 수 없습니다. ann 시나리오에서 `DETACH DELETE`까지 실행하면 T1~T5 모든 시점 질의가 빈 결과를 냅니다(`aeong_scenario_memory.out`, `aeong_scenario_gc.out`).

**③ 구간 질의에 존재한 적 없는 버전이 섞입니다.** `TT FROM 0 TO ∞`에 실제 버전 3개와 `"NULL"` 행 3개가 함께 나옵니다. 이 행은 노드가 아닙니다. 라벨 없는 패턴에서 `labels(n)`을 부르면 `'labels' argument … must be either 'null' or 'node'`로 실패합니다. 관계가 낀 패턴에서는 같은 관계가 수십 번 반복됩니다. 행 33개 중 상당수는 `c.name = NULL`이었습니다.

**④ `VT` 절은 동작하지 않습니다.** `MATCH (c:C) VT AS 0 RETURN c` → `line 1:13 mismatched input 'VT'`.

### 3.5 정리

AeonG가 정답을 주는 범위는 **"노드도 관계도 삭제되지 않고 속성만 바뀐 그래프의 트랜잭션 시간"**입니다. 논문 벤치마크(T-mgBench, T-LDBC, T-gMark)의 갱신 연산도 대부분 이 형태입니다. 그래서 논문의 지연시간과 저장량 수치 자체는 이 결과와 모순되지 않습니다. 다만 퇴사, 계약 해지, 관계 종료처럼 **삭제가 곧 사건인 업무**에는 쓸 수 없습니다.

---

## 4. 튜토리얼 C — 같은 이야기를 Ontological에서

### 4.1 준비

```sql
SELECT og_create_graph('hr');
-- Person(name, salary) / Company(name) / WORKS_AT(title) 타입 생성 후
SELECT og_enable_temporal('hr','Person');
SELECT og_enable_temporal('hr','Company');
SELECT og_enable_temporal('hr','WORKS_AT');
```

`og_enable_temporal`을 부르면 저장 테이블에 두 열이 추가됩니다.
- `__valid tstzrange`: 유효 시간입니다.
- `__sys_from`: 트랜잭션 시간입니다.

이력은 `og_data.h_<tid>` 테이블에 쌓입니다. 행 단위 트리거가 기록하므로 Cypher, TypeQL, 원시 SQL 어느 경로로 쓰든 빠지지 않습니다.

### 4.2 쓰기 — 유효 시간을 직접 준다

```sql
SET og.valid_from = '2020-01-01';   -- T1: 입사
SELECT og_cypher('hr','CREATE (:Person {name:"ann", salary:100})');
...
SET og.valid_from = '2023-01-01';   -- T2: 2023년부터 200 (소급 입력)
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) SET p.salary = 200');
SET og.valid_from = '2023-01-01';   -- T3: 2023년 값 정정 → 250
SELECT og_cypher('hr','MATCH (p:Person {name:"ann"}) SET p.salary = 250');
SET og.valid_from = '2024-06-01';   -- T4: 퇴사
SELECT og_cypher('hr','MATCH (:Person {name:"ann"})-[w:WORKS_AT]->() DELETE w');
```

### 4.3 읽기 — Cypher는 그대로, 시점은 세션 설정으로

(`bench/results/aeong_ontological_scenario.out`)

| # | 설정 | 질문 | 결과 |
|---|---|---|---|
| Q1 | 없음 | 현재 | `{"salary": 250, "company": null}` |
| Q2 | `og.valid_at='2021-06-01'` | 2021년 실제 상태 | `{"title":"eng","salary":100,"company":"acme"}` |
| Q3 | `og.valid_at='2023-06-01'` | 2023년 실제 상태 | `{"salary": 250, "company": "acme"}` |
| Q4 | + `og.system_at=T2` | 정정 **전에** DB가 알던 2023년 | `{"salary": 200}` |
| Q5 | + `og.system_at=T1` | 인상 입력 **전에** DB가 알던 2023년 | `{"salary": 100}` |
| Q6 | `og.valid_at='2025-01-01'` | 퇴사 후 | 관계 없음 |
| Q9 | 노드 `DETACH DELETE` 후 `og.valid_at='2021-06-01'` | 지운 사람의 과거 | `{"salary": 100, "company": "acme"}` |
| Q9 | + `og.system_at=삭제 직전` | | `{"salary": 250}` |

**구간 이력**은 SQL 함수로 봅니다. 아래는 같은 ann을 두 시점의 믿음으로 본 결과입니다.

```text
og_timeline('hr','Person','name','ann')                         -- 지금의 믿음
 2020-01-01 | 2023-01-01 | 100 | v
 2023-01-01 | infinity   | 250 | current
og_timeline('hr','Person','name','ann', system_at => T2)        -- T2 시점의 믿음
 2020-01-01 | 2023-01-01 | 100 | v
 2023-01-01 | infinity   | 200 | u
```

**지원하지 않는 형태는 명시적으로 거절합니다.**

```text
SET og.valid_at = '2021-06-01';
MATCH (p:Person)-[:WORKS_AT*1..2]->(c:Company) RETURN c.name
→ 0A000 OG_TEMPORAL_UNSUPPORTED: variable-length traversal over a temporal relationship type
  cannot answer a point-in-time query
```

---

## 5. 기능 비교

| 항목 | AeonG (실측) | Ontological (실측) |
|---|---|---|
| 시간 축 | 트랜잭션 시간 1개 | 유효 + 시스템 (바이템포럴). 스키마 버전은 별도 축 |
| TT 단위 | 커밋 카운터(기본) 또는 epoch ms | `timestamptz` |
| 시점 지정 방식 | Cypher 절 `TT AS t` (`MATCH` 하나 단위) | 세션·트랜잭션 GUC `og.valid_at` / `og.system_at`. Bolt `tx_metadata`도 가능 |
| 한 쿼리 안에서 서로 다른 시점 | 가능 (`MATCH`마다 다른 `TT`) | 불가 (쿼리 전체가 한 시점) |
| 구간 질의 | `TT FROM a TO b` (Cypher). 가짜 `NULL` 행이 섞임 | `og_timeline()` (SQL, 키 하나 단위). Cypher 구간 문법은 없음 |
| 소급·미래 효력 쓰기 | 불가 | `og.valid_from` / `og.valid_to` |
| 오기 정정 (과거 믿음 보존) | 불가 (모든 변경이 "새 사실") | `og.valid_from='-infinity'` + `og.system_at`으로 이전 믿음 조회 |
| 관계 삭제 뒤 과거 | **틀린 답** | 정답 |
| 노드 삭제 뒤 과거 | **빈 결과** | 정답 |
| 가변 길이 경로 시점 질의 | 받아들이지만 결과는 신뢰 불가 | 거절 (`0A000`) |
| 라벨 없는 패턴 시점 질의 | 동작 (지운 노드는 제외) | 거절 (`0A000`) |
| 이력 저장 | 델타 체인 → GC가 RocksDB로 이전. 앵커 간격 조정 가능 | 행 전체 복사 (`h_*`). 압축이나 델타 없음 |
| 이력 보존 정책 | `--retention_period_sec` (기본 꺼짐) | 없음 (무기한) |
| 이력 위조 방지 | 해당 없음 (단일 프로세스) | 쓰기 권한자도 `h_*`에 DML 불가 (42501) |
| 누가·어디서 바꿨나 | 없음 | `og.source`, `actor`가 `og_timeline`에 남음 |
| 스키마 변경과 시간 | 없음 | 스키마 스냅샷 버전, `og_schema_snapshot(at => …)`. 이력 테이블이 스키마를 따라감 |
| `Ω(e) ⊆ Ω(u)∩Ω(v)` 무결성 | 강제 안 함 | 강제 안 함 (읽을 때 같은 시점으로 걸러서 숨겨질 뿐) |
| 시간 준수 경로 (EAT 등) | 없음 | 없음 |
| 구간 색인 | 앵커와 RocksDB 키 (gid, 시각) | 열린 이력 행에 부분 GiST(`__valid`). 현재 테이블 `__valid`에는 GiST 없음 |
| 성능 측정 | 논문에 있음 (LDBC 등) | **없음** (spec 013 SC-005 미측정) |

---

## 6. 보고서 체크리스트 대비 우리 위치

| 보고서가 요구한 것 | 상태 | 근거 |
|---|---|---|
| 유효·트랜잭션 시간 분리 | ✅ | `og.valid_*` / `og.system_at` |
| 반개구간, 열린 끝 = ∞ | ✅ | `tstzrange '[)'`, `infinity` |
| 노드-엣지 주석형 (노드와 관계 각각의 수명) | ✅ | 타입 단위 `og_enable_temporal` |
| AS OF = 포인트 스태빙 | ✅ | `__valid @> og_valid_at()` |
| 범위 질의 (Allen overlap) | △ | `og_timeline` (SQL, 키 하나 단위). 그래프 패턴 범위 질의는 없음 |
| 관계 ⊆ 노드 수명 제약 | ❌ | 쓰기 때 검사하지 않음 |
| 시간 준수 경로 | ❌ | 시점 질의에서 가변 길이 경로를 거절. `og_reach`는 현재 상태만 |
| 2차원 구간 색인 | △ | 열린 이력에만 부분 GiST |
| hot/cold 분리 | △ | 현재/이력 테이블 분리는 있음. 저장 매체 계층화는 없음 |
| 이력 보존·축출 정책 | ❌ | FR-013 미구현 |
| GQL-T (`AT SYSTEM`, `AT VALID`) 같은 언어 수준 문법 | ❌ (의도적) | ADR-027: 플랜 캐시 보호를 위해 GUC 방식 채택 |

---

## 7. 우리가 가져올 것

AeonG에서 배울 것은 **질의 모델**이 아니라 **저장 모델**입니다.
1. **Cypher 안에 시점을 쓰고 싶다는 요구는 실재합니다.** AeonG의 `MATCH … TT AS t`처럼 "이 패턴만 과거로" 묻는 기능은 GUC로 흉내 낼 수 없습니다. 예를 들어 "지금의 조직도에 작년 담당자를 붙여 보라" 같은 질문입니다. ADR-027의 플랜 캐시 논거를 지키는 길이 있습니다. 시점을 리터럴이 아닌 **파라미터로** 컴파일하면 됩니다(`MATCH … AT VALID $t`). 이 방식은 검토할 가치가 있습니다.
2. **이력 저장량.** 우리는 바뀐 열이 하나여도 행 전체를 복사합니다. AeonG의 델타 체인과 앵커 방식은 이력이 긴 타입에서 크게 차이 날 수 있습니다. 우선 측정부터 해야 합니다. spec 013 SC-005("이력 쓰기 오버헤드 ≤ 40%")가 아직 비어 있습니다.
3. **보존 정책.** AeonG에도 기본으로 꺼진 `retention_period`가 있습니다. 우리는 아예 없습니다.

가져오지 말아야 할 것도 분명합니다. **현재 정점에서 출발해 과거를 덧붙이는 스캔 구조**입니다. 삭제가 있는 순간 과거가 사라지는 결함(§3.4 ②)이 바로 이 구조에서 나옵니다. 우리 구조는 이력 테이블을 1급 관계로 `UNION`합니다(ADR-027). 이 구조 덕분에 같은 질문에 정답을 냈습니다.

---

## 8. 재현 방법

```bash
# AeonG (빈 인스턴스, 기본 논리 모드)
docker build -t aeong -f bench/aeong/Dockerfile bench/aeong
docker run -d --name aeong -p 17687:7687 aeong
bash bench/aeong/aeong_probes.sh aeong                 # §3.2, §3.4 결함 재현 → aeong_probes.out

# AeonG ann 시나리오 (벽시계 모드로 띄운 뒤)
python3 bench/aeong/aeong_scenario.py --container aeong              # 노드 삭제까지
python3 bench/aeong/aeong_scenario.py --container aeong --keep-node  # 노드는 남기고
python3 bench/aeong/aeong_scenario.py --container aeong --gc         # 질의 전 FREE MEMORY

# Ontological
docker exec og bash -lc 'createdb -h localhost -p 28816 tt_demo &&
  psql -h localhost -p 28816 -d tt_demo -c "CREATE EXTENSION ontological CASCADE" &&
  psql -h localhost -p 28816 -d tt_demo -f /work/bench/aeong/ontological_scenario.sql'
```

**환경**:
- 하드웨어: Apple M-시리즈(arm64), 16코어, Docker Desktop
- AeonG: `hououou/AeonG@b780a0c` (vldb 브랜치), Ubuntu 22.04에서 clang 13으로 빌드
- Ontological: 현재 작업 트리, PG16
- 실행일: 2026-10-06

**한계**: 성능은 비교하지 않았습니다. AeonG는 이 문서에서 기능 정확성만 확인했습니다.
