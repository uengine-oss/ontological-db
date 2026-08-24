# 유지보수

> **이 문서가 답하는 질문**
> - 인접 세그먼트는 언제, 어떻게 재정리하는가?
> - 무결성 검사가 위반을 보고하면 무엇을 하는가?
> - `og_relabel`은 언제 필요한가? 부작용은?
> - VACUUM / ANALYZE 전략은? 어떤 테이블이 특별한가?
> - 백엔드-로컬 CSR은 언제 다시 만들어야 하는가?
> - 무한히 자라는 테이블은 무엇이고 어떻게 관리하는가?

---

## 정기 점검 주기 (권장)

> 아래 주기는 저장소에 명시된 값이 아니라, 각 함수의 비용과 성격에서 도출한 **판단**이다.
> 근거가 되는 사실은 각 절에 적혀 있다.

| 작업 | 주기 | 비용 | 절 |
|---|---|---|---|
| `og_check_integrity()` | 일 1회 이상 | 읽기 전용, 검사당 `LIMIT 100` | §2 |
| `og_graph_stats()` 의 `packing_ratio` 확인 | 일 1회 | 읽기 전용 집계 | §1 |
| `og_reorganize(graph)` | `packing_ratio`가 낮아졌을 때만 | **쓰기**, 대상 수에 비례 | §1 |
| `ANALYZE og_data.og_adj` | 대량 적재 직후 **반드시** | 가벼움 | §4 |
| `VACUUM (ANALYZE) og_data.og_adj` | 대량 삭제/갱신 후 | 중간 | §4 |
| `og_data.og_audit` 정리 | 보존 정책에 따라 | 삭제량에 비례 | §6 |
| `og_stale_embeddings()` 확인 | 임베딩을 쓰면 일 1회 | 읽기 전용 | §7 |
| `og_relabel(graph_id)` | **정기 작업 아님** — 진단/복구 전용 | 타입 수에 비례, 뷰 전부 드롭 | §3 |
| `og_csr_build(...)` | 연결 수립 시 또는 위상 변경 후 | dense 119ms / sparse 229ms, 백엔드마다 | §5 |

---

## 1. 인접 세그먼트 재정리 — `og_reorganize`

### 왜 필요한가

`og_data.og_adj`의 한 행은 한 노드·한 관계 타입·한 방향에 대한 이웃을 최대 256개 담는다
(`engine/src/storage/adjacency.rs:13-15`). 엣지 추가는 꼬리 세그먼트에 `UPDATE`로 붙이고,
가득 차면 새 세그먼트를 만든다 (`engine/src/storage/adjacency.rs:19-46`).
엣지 삭제는 배열에서 원소를 빼고, 비어버린 세그먼트를 삭제한다 (`:66-71`).

이 두 동작이 반복되면 **세그먼트가 여러 개인데 각각 절반만 찬 상태**가 된다.
순회는 그만큼 더 많은 힙 튜플을 읽는다.

### 시그니처와 동작

```sql
SELECT og_reorganize('default');   -- → 재패킹한 (src, etype, dir) 그룹 수
```

(`engine/src/storage/stats.rs:121-166`, spec 001 FR-018)

**대상 선정** (`engine/src/storage/stats.rs:127-132`):

```sql
SELECT a.src, a.etype, a.dir::text
  FROM og_data.og_adj a
  JOIN og_data.og_node nd ON nd.id = a.src
  JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = $1
 GROUP BY a.src, a.etype, a.dir
HAVING count(*) > 1 AND sum(a.n) <= count(*) * $2 - $2
```

`$2`는 `CHUNK` = 256. 조건을 말로 풀면:
**세그먼트가 2개 이상이면서, 전체 이웃 수가 지금보다 세그먼트 하나 적게도 들어가는 그룹.**
즉 이미 잘 채워진 그룹은 건드리지 않는다.

**재패킹** (`engine/src/storage/stats.rs:145-162`): 그룹마다 한 문장이다 —
`unnest`로 펼쳐 `row_number()`로 256개씩 다시 묶고, 기존 행을 `DELETE`한 뒤 `INSERT`한다.

### 운영상 정확히 알아야 할 것

- **호출자의 트랜잭션 안에서 전부 실행된다.** 함수 안에서 커밋하지 않는다.
  대상 그룹이 많으면 락과 dead tuple이 트랜잭션 끝까지 누적된다.
- **읽기는 막지 않는다** (MVCC). 다만 같은 노드의 인접을 동시에 쓰는 트랜잭션은 대기한다.
- 반환값은 **재패킹한 그룹 수**이며, 대상이 없으면 `0`이다.
- 그래프 이름이 존재하지 않으면 `graph '{name}' does not exist`
  (`engine/src/catalog/types.rs`의 `graph_id`).

### 실행 절차

```sql
-- 1. 실행 전 상태 기록
SELECT jsonb_pretty(og_graph_stats('default') -> 'adjacency');

-- 2. 대상이 몇 개인지 미리 세어본다 (og_reorganize 와 동일한 조건)
SELECT count(*) AS candidates
  FROM (SELECT a.src, a.etype, a.dir
          FROM og_data.og_adj a
          JOIN og_data.og_node nd ON nd.id = a.src
          JOIN og_catalog.type t ON t.type_id = nd.type_id AND t.graph_id = og_graph_id
         GROUP BY a.src, a.etype, a.dir
        HAVING count(*) > 1 AND sum(a.n) <= count(*) * 256 - 256) x;
```

> 위 질의의 `og_graph_id` 자리에는 실제 값을 넣는다:
> `SELECT graph_id FROM og_catalog.graph WHERE name = 'default'`.

```sql
-- 3. 실행
SELECT og_reorganize('default');

-- 4. 실행 후 상태와 비교
SELECT jsonb_pretty(og_graph_stats('default') -> 'adjacency');

-- 5. dead tuple 을 회수
VACUUM (ANALYZE) og_data.og_adj;
```

> **필수**: `og_reorganize` 직후에는 반드시 `VACUUM`을 돌릴 것.
> 재패킹은 DELETE + INSERT이므로 원본 행이 전부 dead tuple로 남는다.

### 언제 돌리는가 — 판단 기준

| 신호 | 조치 |
|---|---|
| `packing_ratio`가 최근 값 대비 뚜렷이 낮아짐 | 후보 수를 세어보고 `og_reorganize` |
| 대량 엣지 삭제 직후 | `og_reorganize` → `VACUUM (ANALYZE)` |
| `chunked_supernodes`만 늘고 `packing_ratio`는 유지 | **불필요.** 수퍼노드가 청크를 쓰는 것은 정상 설계 (spec 001 FR-014/FR-020) |
| 순회는 느려졌는데 `packing_ratio`가 1.0에 가까움 | 재정리 대상이 아니다. 캐시·통계·질의 형태를 볼 것 |

---

## 2. 무결성 검사와 대응 — `og_check_integrity`

```sql
SELECT kind, count(*) AS n FROM og_check_integrity() GROUP BY kind ORDER BY n DESC;
SELECT * FROM og_check_integrity() LIMIT 50;
```

**빈 결과가 통과 조건이다** (`engine/src/storage/stats.rs:170-171`).

### 위반 종류별 의미와 조치

| `kind` | 의미 | 조치 |
|---|---|---|
| `dangling_adjacency` | `og_adj`가 존재하지 않는 엣지 id를 참조 | 해당 엣지가 삭제되었는데 인접이 남은 상태. `entity_id`(엣지 id)로 `og_data.og_edge`를 확인하고, 없다면 그 id를 참조하는 세그먼트를 손봐야 한다 |
| `missing_adjacency` | 엣지가 양 끝점 중 한쪽에서 도달 불가 | 인접의 한 방향이 유실된 상태. 순회 결과가 방향에 따라 달라진다 |
| `segment_length_mismatch` | `n ≠ array_length(nbr,1)` 또는 `nbr`/`eid` 길이 불일치 | 세그먼트 자체가 손상. 가장 심각하다 |
| `orphan_node` | 노드가 알 수 없는 `type_id`를 참조 | 타입이 카탈로그에서 사라졌거나 카탈로그가 손상 |

> **저장소에는 자동 복구 함수가 없다.** `og_check_integrity`는 **보고만** 한다.
> `og_reorganize`는 세그먼트를 다시 묶을 뿐 참조 정합성을 고치지 않는다.
>
> 위반이 나온 경우의 안전한 대응 순서:
> 1. **더 쓰지 말 것.** 원인을 모르는 채 계속 쓰면 범위가 넓어진다.
> 2. `pg_dump`로 현 상태를 보존 ([08_backup_and_restore.md](08_backup_and_restore.md)).
> 3. 위반의 총량 파악 — `og_check_integrity`의 각 검사는 `LIMIT 100`이므로
>    `engine/src/storage/stats.rs:183-260`의 SQL을 `LIMIT` 없이 직접 실행할 것.
> 4. 마지막 정상 백업으로부터 복원하는 것이 가장 확실한 경로다.

### 위반 총량을 직접 세는 질의

```sql
-- 1. dangling_adjacency
SELECT count(DISTINCT e) FROM og_data.og_adj a, LATERAL unnest(a.eid) AS e
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_edge x WHERE x.id = e);

-- 2. missing_adjacency
SELECT count(*) FROM og_data.og_edge e
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_adj a
                    WHERE a.src = e.src AND a.dir = 'o' AND e.id = ANY(a.eid))
    OR NOT EXISTS (SELECT 1 FROM og_data.og_adj a
                    WHERE a.src = e.dst AND a.dir = 'i' AND e.id = ANY(a.eid));

-- 3. segment_length_mismatch
SELECT count(*) FROM og_data.og_adj
 WHERE n <> COALESCE(array_length(nbr,1),0)
    OR COALESCE(array_length(nbr,1),0) <> COALESCE(array_length(eid,1),0);

-- 4. orphan_node
SELECT count(*) FROM og_data.og_node n
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.type t WHERE t.type_id = n.type_id);
```

(각각 `engine/src/storage/stats.rs:187-188`, `:204-211`, `:227-231`, `:246-248`에서 `LIMIT` 제거)

---

## 3. 타입 구간 라벨 재계산 — `og_relabel`

### 무엇인가

이 시스템의 서브타입 판정은 재귀 워크가 아니라 **구간(nested-set) 라벨의 범위 비교 1회**다
(`engine/src/catalog/labeling.rs:232-244`의 `og_is_subtype`).
그 라벨이 `og_catalog.type_label`에 있고, 계층이 바뀔 때마다 갱신된다.

```sql
SELECT og_relabel(<graph_id>::int4);
```

> **주의**: 인자는 **그래프 이름이 아니라 `graph_id`(int4)** 다
> (`engine/src/catalog/labeling.rs:248`). 이름으로 부르면 타입 오류가 난다.

```sql
SELECT og_relabel(graph_id) FROM og_catalog.graph WHERE name = 'default';
```

### 부작용 — 반드시 알 것

`og_relabel` → `relabel_graph` (`engine/src/catalog/labeling.rs:116-170`)의 실제 동작:

1. 타입 DAG를 적재하고 루트를 찾는다.
2. **상속 사이클을 검출한다.** 발견되면 다음 중 하나로 실패한다:
   - `inheritance cycle detected while walking the type hierarchy` (`:132`)
   - `inheritance cycle detected involving type(s) {orphans:?}` (`:142`)
3. `DELETE FROM og_catalog.type_label WHERE graph_id = $1` 후 전부 다시 INSERT.
4. `bump_schema_version(graph_id, "relabel")` 호출 (`:169`) →
   그 안에서 **`crate::cypher::views::drop_all_views()`가 실행되어 생성된 타입 뷰가 전부 드롭된다**
   (`engine/src/catalog/labeling.rs:172-175` 주석: "Generated per-type union views encode the
   descendant set, so any schema change invalidates them").

즉 `og_relabel`은 **스키마 버전을 올리고 캐시된 뷰를 무효화하는 무거운 작업**이다.
정상 경로에서는 `insert_between`이 기존 라벨을 건드리지 않고 처리하므로
(`engine/src/catalog/labeling.rs:113-115` 주석) 직접 호출할 일이 없다.

### 언제 부르는가

| 상황 | 부르는가 |
|---|---|
| 평소 타입 추가/삭제 | **아니다** — 자동으로 처리된다 |
| `og_type_view`의 `lft`/`rgt`가 명백히 어긋나 보임 | 진단 목적으로 사용 |
| `og_is_subtype` / `og_subtypes` 결과가 계층과 맞지 않음 | 복구 목적으로 사용 |
| 상속 사이클이 의심됨 | 호출해 보면 위 두 오류 중 하나로 확인된다 |

라벨 상태 확인:

```sql
SELECT name, kind, depth, lft, rgt, (rgt - lft) AS span, parents
  FROM og_type_view WHERE graph = 'default' ORDER BY lft;

-- 라벨이 아예 없는 타입 (있으면 비정상)
SELECT t.type_id, t.name
  FROM og_catalog.type t
  LEFT JOIN og_catalog.type_label l ON l.type_id = t.type_id
 WHERE l.type_id IS NULL;
```

스키마 버전 이력:

```sql
SELECT version, graph_id, description
  FROM og_catalog.schema_version ORDER BY version DESC LIMIT 20;
```

---

## 4. VACUUM / ANALYZE 전략

### 사실

| 사실 | 근거 |
|---|---|
| `og_data.og_adj`는 `fillfactor = 80` | `engine/sql/bootstrap.sql:206` |
| 배열은 `STORAGE MAIN`으로 인라인 — TOAST 없음 | `engine/sql/bootstrap.sql:210-211` |
| 엣지 추가는 꼬리 세그먼트 `UPDATE` | `engine/src/storage/adjacency.rs:19-46` |
| 엣지 삭제는 배열 갱신 + 빈 세그먼트 `DELETE` | `engine/src/storage/adjacency.rs:48-72` |
| 깊은 순회 재작성 여부가 **플래너 통계**로 결정된다. 통계가 없으면 깊이만 보고 판단한다 | `docs/deep-traversal.md:220-226` |
| 벤치 하네스도 적재 후 `ANALYZE`를 명시적으로 실행한다 | `bench/harness.py` `PgGraph.setup` 등 |

### 전략

**A. 대량 적재 직후 — `ANALYZE`는 선택이 아니다**

```sql
ANALYZE og_data.og_adj;
ANALYZE og_data.og_node;
ANALYZE og_data.og_edge;
-- 타입별 저장 테이블까지
ANALYZE og_data;   -- 스키마 전체
```

이유: 가변 길이 매치를 방문집합 BFS로 재작성할지 말지가
`Σ degreeⁱ > |V|` 비교로 결정되고, 두 항 모두 **플래너 통계에서 읽는다**.
통계가 없으면 깊이만 보고 판단하는 보수적 경로로 떨어진다 (`docs/deep-traversal.md:220-226`).
즉 **ANALYZE를 빼먹으면 질의 계획이 나빠진다.**

**B. 대량 삭제/재정리 후 — `VACUUM`**

```sql
VACUUM (ANALYZE, VERBOSE) og_data.og_adj;
```

**C. 상시 — autovacuum을 켜두되 `og_adj`를 관찰**

```sql
SELECT relname, n_live_tup, n_dead_tup,
       round(100.0*n_dead_tup/NULLIF(n_live_tup+n_dead_tup,0),1) AS dead_pct,
       last_autovacuum, last_autoanalyze
  FROM pg_stat_user_tables
 WHERE schemaname IN ('og_data','og_catalog')
 ORDER BY n_dead_tup DESC;
```

`og_data.og_adj`의 `dead_pct`가 지속적으로 높다면, 이 테이블만 autovacuum을 공격적으로
설정하는 것을 검토한다 (판단):

```sql
ALTER TABLE og_data.og_adj SET (autovacuum_vacuum_scale_factor = 0.02,
                                autovacuum_analyze_scale_factor = 0.02);
```

> `og_adj`는 순회의 핫 릴레이션이고 배열이 인라인이므로, 팽창(bloat)이 곧 순회 지연이다.
> 이 설정은 저장소에 없는 **권장값**이며, 실측 후 조정할 것.

**D. `VACUUM FULL`은 마지막 수단**

`VACUUM FULL`은 ACCESS EXCLUSIVE 락을 잡는다 — 그동안 그래프 전체가 멈춘다.
먼저 `og_reorganize` + 일반 `VACUUM`을 시도할 것.

---

## 5. 백엔드-로컬 CSR 재빌드 시점

### 사실 (`docs/deep-traversal.md:237-267`, `engine/src/storage/traverse.rs:16-24`)

- `og_csr_build`는 `og_data.og_adj`를 **이 백엔드의 메모리**로 컴파일한다.
- **연결 단위다.** 데이터베이스 단위도, 서버 단위도 아니다.
- 비용: dense 픽스처 **119 ms / 8.4 MiB**, sparse **229 ms / 9.2 MiB**.
- **스냅샷이 빌드 시점에 얼어붙는다.** 이후 커밋된 엣지는 재빌드 전까지 보이지 않는다.
  트리거 캡처가 없다.
- **RLS가 참조되지 않는다.** 호출자가 읽을 수 없는 행을 지나는 경로도 결과에 나온다.
- `PARALLEL RESTRICTED` — 병렬 워커에서 실행되지 않는다.
- **Cypher 컴파일러는 CSR로 라우팅하지 않는다.** `og_reach`로 간다.
  CSR은 노출·측정·문서화되어 있을 뿐 자동 대체되지 않는다 (`docs/deep-traversal.md:269-272`).

### 재빌드가 필요한 시점

| 사건 | 재빌드 필요 |
|---|---|
| 새 연결 수립 | **필요** — 아무것도 없는 상태로 시작한다 |
| 엣지 생성/삭제가 커밋됨 | **필요** — 스냅샷이 오래됐다 |
| 타입/방향/관계 타입 필터를 바꿔 묻고 싶음 | **필요** — `built_for` 키가 다르다 |
| 같은 백엔드에서 같은 질문 반복 | 불필요 |

### 운영 패턴

```sql
-- 이 백엔드가 무엇을 갖고 있는지
SELECT * FROM og_csr_stats();

-- 없거나 오래됐으면 다시 만든다
SELECT * FROM og_csr_build(NULL, 'o');
-- → nodes | edges | bytes | build_ms

-- 쓰고 나서 메모리를 돌려주려면
SELECT og_csr_drop();
```

> **커넥션 풀 환경 주의**: 풀 뒤에서는 어느 백엔드가 응답할지 알 수 없다.
> "빌드한 뒤 질의"가 같은 백엔드에서 일어나도록 **하나의 세션을 명시적으로 고정**하거나,
> 매 요청마다 빌드 비용(119~229ms)을 지불할 각오를 해야 한다.
> `docs/deep-traversal.md:252-256`이 이 효과를 pgGraph의 콜드 컬럼과 같은 것이라 설명한다.

---

## 6. 무한히 자라는 테이블

### `og_data.og_audit`

`og_cypher` / `og_typeql` 호출마다 한 행 (`engine/src/cypher/mod.rs:122-135`).
**자동 정리 장치가 없다.** 백업 대상에도 포함된다 (`engine/sql/bootstrap.sql:431`).

```sql
-- 현황
SELECT count(*) AS rows, min(at) AS oldest, max(at) AS newest,
       pg_size_pretty(pg_total_relation_size('og_data.og_audit')) AS size
  FROM og_data.og_audit;

-- 보존 정책 예: 30일. 배치로 나눠 지우면 락 보유 시간이 짧다.
DELETE FROM og_data.og_audit
 WHERE audit_id IN (SELECT audit_id FROM og_data.og_audit
                     WHERE at < now() - interval '30 days'
                     LIMIT 50000);

-- 반복 후
VACUUM (ANALYZE) og_data.og_audit;
```

`og_audit_at_idx`가 `(at DESC)`로 있으므로 (`engine/sql/bootstrap.sql:390`)
시간 조건 삭제는 인덱스를 탄다.

### `og_data.og_history`

`og_enable_history(graph, type_name)`을 켠 타입에서만 자란다
(`engine/src/agent/mod.rs:448-468`). 기본은 꺼져 있다 — "Off by default: it costs writes."

```sql
-- 어떤 타입에 켜져 있는가
SELECT key, value FROM og_catalog.setting WHERE key LIKE 'history.%' ORDER BY key;

-- 규모
SELECT count(*) AS rows, min(recorded_at) AS oldest,
       pg_size_pretty(pg_total_relation_size('og_data.og_history')) AS size
  FROM og_data.og_history;

-- 엔티티별 상위
SELECT entity_id, count(*) AS versions
  FROM og_data.og_history GROUP BY entity_id ORDER BY versions DESC LIMIT 20;
```

> **금지**: `og_data.og_history`를 함부로 지우지 말 것.
> `og_as_of(id, ts)`는 **히스토리 행의 존재 여부로** 추적 대상인지 판단하고,
> 없으면 오류를 낸다 (`engine/src/agent/mod.rs:503-516`):
> `no history is retained for entity {id}. enable it with og_enable_history(graph, type) —
> returning the current value instead would be a lie`.
> 즉 히스토리를 지우면 시점 조회가 "값이 틀린" 게 아니라 "실패"로 바뀐다.

히스토리 조회:

```sql
SELECT * FROM og_history(<entity_id>::int8);   -- recorded_at | op | payload
SELECT og_as_of(<entity_id>::int8, '2026-08-01T00:00:00Z'::timestamptz);
```

---

## 7. 임베딩 유지보수

```sql
-- 재계산이 밀린 것
SELECT count(*) FROM og_stale_embeddings('default');
SELECT type_name, prop, count(*) FROM og_stale_embeddings('default')
 GROUP BY type_name, prop ORDER BY 3 DESC;

-- 선언 현황
SELECT jsonb_pretty(og_embedding_stats('default'));
```

재계산 후에는 `og_mark_embedded(entity_id, prop)`로 상태를 갱신한다
(`engine/src/vector/mod.rs:358-359` 부근).

HNSW 인덱스는 `og_add_embedding`이 만든다. 대량 적재 후 인덱스 품질이 걱정되면
PostgreSQL 표준 `REINDEX`를 쓴다 — 대상 인덱스 이름은 여기서 확인한다:

```sql
SELECT indexrelname, relname, pg_size_pretty(pg_relation_size(indexrelid)) AS size
  FROM pg_stat_user_indexes
 WHERE schemaname = 'og_data'
 ORDER BY pg_relation_size(indexrelid) DESC;
```

---

## 8. 확장 재설치 (코드 변경 반영)

```bash
# 1. 빌드 + 설치
docker exec ontological-dev bash -lc 'cd /work/engine && \
  cargo pgrx install --features pg16 --no-default-features \
    --pg-config /usr/lib/postgresql/16/bin/pg_config --sudo'

# 2. 이미 열려 있는 백엔드가 옛 .so 를 잡고 있을 수 있으므로 재기동
docker exec ontological-dev bash -lc 'cd /work/engine && cargo pgrx stop pg16 && cargo pgrx start pg16'

# 3. 회귀 스위트
docker exec ontological-dev bash -lc 'cd /work && ./tests/run.sh'
```

> **금지**: `ALTER EXTENSION ontological UPDATE`.
> 업그레이드 스크립트(`ontological--0.1.0--*.sql`)가 저장소에 없다.
> SQL 스키마(`bootstrap.sql` / `access.sql`)가 바뀌는 변경은 **재설치가 아니라
> 새 데이터베이스 + 데이터 이관**으로만 반영된다 —
> [08_backup_and_restore.md](08_backup_and_restore.md) 및 `OPS-02` 참조.

---

## 9. 유지보수 스크립트 (붙여넣기용)

```sql
-- ============================================================
-- Ontological — daily maintenance check
-- ============================================================
\timing on
\pset pager off

\echo '--- 1. integrity (empty = pass) ---'
SELECT kind, count(*) FROM og_check_integrity() GROUP BY kind;

\echo '--- 2. adjacency packing ---'
SELECT count(*) AS segments,
       round(avg(n)::numeric, 2) AS avg_fill,
       round((avg(n)/256)::numeric, 4) AS packing_ratio,
       count(*) FILTER (WHERE seq > 0) AS chunked_supernodes
  FROM og_data.og_adj;

\echo '--- 3. dead tuples ---'
SELECT relname, n_live_tup, n_dead_tup,
       round(100.0*n_dead_tup/NULLIF(n_live_tup+n_dead_tup,0),1) AS dead_pct,
       last_autovacuum, last_autoanalyze
  FROM pg_stat_user_tables
 WHERE schemaname IN ('og_data','og_catalog') AND n_dead_tup > 0
 ORDER BY n_dead_tup DESC LIMIT 10;

\echo '--- 4. growing tables ---'
SELECT 'og_audit' AS t, count(*) AS rows,
       pg_size_pretty(pg_total_relation_size('og_data.og_audit')) AS size
  FROM og_data.og_audit
UNION ALL
SELECT 'og_history', count(*),
       pg_size_pretty(pg_total_relation_size('og_data.og_history'))
  FROM og_data.og_history;

\echo '--- 5. stale embeddings ---'
SELECT count(*) AS stale FROM og_stale_embeddings('default');

\echo '--- 6. schema version history ---'
SELECT version, graph_id, description
  FROM og_catalog.schema_version ORDER BY version DESC LIMIT 5;
```

---

## 금지 / 필수

### 금지 (Forbidden)

- `og_reorganize`를 정기 크론에 무조건 걸지 말 것 — 대상이 없으면 헛돌고,
  많으면 한 트랜잭션에 큰 쓰기를 만든다. `packing_ratio`를 보고 결정할 것.
- `og_relabel`을 정기 작업으로 돌리지 말 것 — 생성된 뷰를 전부 드롭하고 스키마 버전을 올린다.
- `og_data.og_history`를 임의로 삭제하지 말 것 — `og_as_of`가 실패로 바뀐다.
- 무결성 위반을 발견한 뒤 원인 파악 없이 계속 쓰지 말 것.
- `VACUUM FULL`을 1차 대응으로 쓰지 말 것 — ACCESS EXCLUSIVE 락.
- `ALTER EXTENSION ontological UPDATE`를 시도하지 말 것.

### 필수 (Required)

- **대량 적재 후 `ANALYZE`.** 순회 재작성 판단이 플래너 통계에 의존한다.
- `og_reorganize` 후 `VACUUM (ANALYZE) og_data.og_adj`.
- `og_data.og_audit`의 보존 정책을 정할 것.
- CSR을 쓰는 워크로드라면 **세션 고정 또는 빌드 비용 감수** 중 하나를 명시적으로 선택할 것.
- 유지보수 전후로 `og_check_integrity()`를 돌려 비교할 것.

---

<!-- affects: ops, backend, data -->
<!-- requires-update: docs/08_operations/08_backup_and_restore.md, docs/08_operations/09_troubleshooting.md -->
