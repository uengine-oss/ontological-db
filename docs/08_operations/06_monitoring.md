# 관측

> **이 문서가 답하는 질문**
> - 이 시스템에서 무엇을 봐야 하는가? 어떤 SQL로 보는가?
> - 그래프가 커지고 있는지, 어디가 뚱뚱해지는지 어떻게 아는가?
> - 인접 세그먼트가 언제 재정리를 필요로 하는가?
> - 질의 이력·오류·소요시간은 어디에 남는가?
> - PostgreSQL 표준 통계와는 어떻게 엮이는가?

---

## 사실 (Facts) — 관측 표면의 두 층

이 확장은 **모든 것을 평범한 힙 릴레이션에 둔다** (`engine/sql/bootstrap.sql:9-10`:
"every structure below is an ordinary heap relation, so it inherits MVCC / WAL /
vacuum / pg_dump for free"). 따라서 관측도 두 층으로 나뉜다.

| 층 | 도구 | 성격 |
|---|---|---|
| **그래프 층** | `og_graph_stats`, `og_degree_distribution`, `og_csr_stats`, `og_embedding_stats`, `og_check_integrity`, `og_data.og_audit`, 인트로스펙션 뷰 | 이 확장이 제공 |
| **PostgreSQL 층** | `pg_stat_user_tables`, `pg_statio_user_tables`, `pg_stat_activity`, `pg_stat_database`, `pg_total_relation_size` | 표준 |

> **미구현 사실**: 메트릭 익스포터(Prometheus 등)도, 확장 수준의 헬스체크 함수도 없다.
> Studio의 `GET /api/health` (`portal/server/index.js:151-165`)가 유일한 HTTP 헬스 엔드포인트이며,
> 그것은 Studio의 상태를 말하는 것이지 데이터베이스의 상태를 말하는 것이 아니다.
> → [10_improvements_ops.md](10_improvements_ops.md) `OPS-08`, `OPS-16`

---

## 1. 그래프 규모와 인접 패킹 — `og_graph_stats`

시그니처: `og_graph_stats(graph text) → jsonb` (`engine/src/storage/stats.rs:11-12`, `stable strict`)

```sql
SELECT jsonb_pretty(og_graph_stats('default'));
```

반환 구조 (`engine/src/storage/stats.rs:69-82`):

```jsonc
{
  "graph": "default",
  "nodes": 69,
  "edges": 104,
  "types": [ { "name": "Film", "kind": "entity", "abstract": false, "instances": 12 }, … ],
  "adjacency": {
    "segments": 138,          // og_data.og_adj 행 수 (전체 DB 기준)
    "avg_fill": 1.5,          // 세그먼트당 평균 live 이웃 수
    "chunk_size": 256,        // 컴파일 상수 CHUNK
    "packing_ratio": 0.0058,  // avg_fill / chunk_size. 1.0 = 완벽히 채워짐
    "chunked_supernodes": 4   // seq > 0 인 세그먼트 수 = 청크가 쪼개진 노드
  }
}
```

### 필드를 읽는 법

| 필드 | 의미 | 무엇을 뜻하는가 |
|---|---|---|
| `packing_ratio` | `avg_fill / 256` | **1.0에 가까울수록 좋다.** 낮으면 순회가 읽는 힙 튜플이 필요 이상으로 많다는 뜻 (`engine/src/storage/stats.rs:78-79` 주석: "1.0 = perfectly packed; lower means reorganisation will help") |
| `chunked_supernodes` | `seq > 0` 세그먼트 수 | 이웃이 256개를 넘어 청크가 여러 개로 쪼개진 노드가 있다는 신호 |
| `segments` | `og_data.og_adj` 전체 행 수 | 주의: **그래프별로 필터되지 않는다.** 이 세 값은 DB 전역 집계다 (`engine/src/storage/stats.rs:46-66`의 SQL에 `graph_id` 조건이 없다) |

> **함정**: `nodes` / `edges` / `types`는 `graph`로 필터되지만 `adjacency` 블록은 아니다.
> 한 데이터베이스에 여러 그래프가 있으면 `adjacency`는 **전체 합**이다.

### 패킹 비율 추이 관찰

```sql
-- 직접 계산하는 형태 (그래프 무관, og_graph_stats 와 같은 정의)
SELECT count(*)                                   AS segments,
       round(avg(n)::numeric, 2)                  AS avg_fill,
       round((avg(n) / 256)::numeric, 4)          AS packing_ratio,
       count(*) FILTER (WHERE seq > 0)            AS chunked_supernodes,
       pg_size_pretty(pg_total_relation_size('og_data.og_adj')) AS adj_size
  FROM og_data.og_adj;
```

`packing_ratio`가 낮아지면 [07_maintenance.md](07_maintenance.md)의 `og_reorganize`를 검토한다.

---

## 2. 차수 분포 — 수퍼노드 조기 발견

시그니처: `og_degree_distribution(graph text) → jsonb`
(`engine/src/storage/stats.rs:86-87`, spec 001 FR-020)

```sql
SELECT jsonb_pretty(og_degree_distribution('default'));
```

내부 정의 (`engine/src/storage/stats.rs:92-100`):
out-degree(`dir = 'o'`)를 노드별로 합산한 뒤 `width_bucket(deg, 0, 1024, 8)`으로
8개 버킷에 넣는다. 반환:

```jsonc
{ "graph": "default",
  "buckets": [ { "bucket": 1, "nodes": 61, "max_degree": 7 }, … ] }
```

버킷 경계는 `0..1024`를 8등분 → 폭 128. `bucket = 9`는 1024를 넘는 노드다.

### 읽기 좋은 형태

```sql
SELECT (b->>'bucket')::int                                     AS bucket,
       ((b->>'bucket')::int - 1) * 128                          AS deg_from,
       (b->>'bucket')::int * 128                                AS deg_to,
       (b->>'nodes')::bigint                                    AS nodes,
       (b->>'max_degree')::bigint                               AS max_degree
  FROM jsonb_array_elements(og_degree_distribution('default') -> 'buckets') b
 ORDER BY bucket;
```

### 상위 수퍼노드 직접 찾기

```sql
SELECT a.src,
       nv.type_name,
       sum(a.n)             AS out_degree,
       count(*)             AS segments
  FROM og_data.og_adj a
  JOIN og_node_view nv ON nv.id = a.src
 WHERE a.dir = 'o' AND nv.graph = 'default'
 GROUP BY a.src, nv.type_name
 ORDER BY out_degree DESC
 LIMIT 20;
```

### 노드 하나의 차수

```sql
SELECT og_degree_all(<node_id>::int8, 'o');            -- 나가는 전체
SELECT og_degree_all(<node_id>::int8, 'i');            -- 들어오는 전체
SELECT og_degree(<node_id>::int8, <etype_id>::int4, 'o');  -- 관계 타입 한정
```

(`engine/src/storage/adjacency.rs:77`, `:89`. `dir`은 `'o'` \| `'i'`)

관계 타입 id는 `og_type_id`로 얻는다:

```sql
SELECT og_type_id('default', 'ACTED_IN');
```

---

## 3. 백엔드-로컬 CSR — `og_csr_stats`

시그니처: `og_csr_stats() → TABLE(built_for text, nodes int8, edges int8, bytes int8)`
(`engine/src/storage/traverse.rs:322-334`)

```sql
SELECT * FROM og_csr_stats();
```

| 상태 | 결과 |
|---|---|
| 이 백엔드가 CSR을 갖고 있지 않음 | `built_for` = `NULL`, 나머지 0 (`engine/src/storage/traverse.rs:335-337`) |
| 갖고 있음 | `built_for` = 빌드 키(etypes + 방향), `nodes`/`edges`/`bytes` |

> **핵심**: CSR은 **연결(백엔드) 단위**다. `og_csr_stats()`가 보여주는 것은
> **지금 이 세션이 잡고 있는 것**이며, 다른 세션의 상태는 알 수 없다.
> 커넥션 풀 뒤에서 이 함수를 호출하면 매번 다른 백엔드를 볼 수 있다.

빌드와 해제:

```sql
-- 특정 관계 타입만, 나가는 방향 (기본 'o')
SELECT * FROM og_csr_build(ARRAY[og_type_id('default','KNOWS')]::int4[], 'o');
-- → nodes | edges | bytes | build_ms

-- 전체 타입
SELECT * FROM og_csr_build(NULL, 'b');

SELECT og_csr_drop();
```

(`engine/src/storage/traverse.rs:295-315`, `:316-320`)

`docs/deep-traversal.md:252-256`이 비용을 실측치로 적는다:
dense 픽스처에서 **119 ms / 8.4 MiB**, sparse에서 **229 ms / 9.2 MiB**,
그리고 이것은 **연결마다** 지불된다.

CSR이 없는 상태에서 `og_csr_reach` / `og_csr_hops`를 호출하면:

```
ERROR:  no compiled graph in this backend — call og_csr_build() first
```

(`engine/src/storage/traverse.rs:340`)

> **관측 시 반드시 함께 기억할 제약** (`docs/deep-traversal.md:257-267`):
> CSR의 스냅샷은 **빌드 시점에 얼어붙는다.** 이후 커밋된 엣지는 재빌드 전까지 보이지 않는다.
> RLS도 참조되지 않는다. 그리고 `og_reach`/`og_csr_*`는 `PARALLEL RESTRICTED`라
> 병렬 워커에서 실행되지 않는다.

---

## 4. 임베딩 상태

### 선언된 임베딩 목록

```sql
SELECT jsonb_pretty(og_embedding_stats('default'));
```

(`engine/src/vector/mod.rs:383-384`) 반환:

```jsonc
{ "graph": "default",
  "embeddings": [
    { "type": "Film", "property": "plot_vec", "dims": 1024,
      "metric": "cosine", "source_property": "plot" }, … ] }
```

### 소스가 바뀌어 재계산이 필요한 것

```sql
SELECT * FROM og_stale_embeddings('default');
-- entity_id | type_name | prop
```

(`engine/src/vector/mod.rs:299-300`) `source_prop`이 선언된 임베딩만 대상이며,
`og_data.og_embedding_state`와 대조해 갱신이 밀린 엔티티를 찾는다.

재계산 후에는 `og_mark_embedded(entity_id, prop)`로 표시한다.

### 모니터링 질의

```sql
-- 임베딩 대기열 크기
SELECT count(*) AS stale FROM og_stale_embeddings('default');

-- 타입별 대기 분포
SELECT type_name, prop, count(*) AS stale
  FROM og_stale_embeddings('default')
 GROUP BY type_name, prop
 ORDER BY stale DESC;
```

---

## 5. 질의 감사 로그 — `og_data.og_audit`

`og_cypher`와 `og_typeql`은 매 호출마다 이 테이블에 한 행을 남긴다
(`engine/src/cypher/mod.rs:122-135`, `engine/src/typeql/mod.rs:118`).

스키마 (`engine/sql/bootstrap.sql:380-390`):

```sql
CREATE TABLE og_data.og_audit (
    audit_id    bigserial PRIMARY KEY,
    principal   text        NOT NULL DEFAULT session_user,
    at          timestamptz NOT NULL DEFAULT now(),
    query       text,
    lang        text,                 -- 'cypher' | 'sparql' | 'sql'
    rows_out    int8,
    duration_ms double precision,
    error_code  text
);
CREATE INDEX og_audit_at_idx ON og_data.og_audit (at DESC);
```

`query`에는 `[<graph>] <query text>` 형태로 그래프 이름이 앞에 붙는다
(`engine/src/cypher/mod.rs:128`). 파싱 실패도 기록된다 — `error_code`에 오류 메시지
앞 200자가 들어간다 (`engine/src/cypher/mod.rs:96`, `:131`).

> **주의**: 감사 기록의 `Spi::run_with_args(...)`가 `.ok()`로 끝난다
> (`engine/src/cypher/mod.rs:134`). 감사 쓰기 실패는 질의를 실패시키지 않지만
> **조용히 누락된다.**

### 운영용 질의

```sql
-- 최근 실패 20건
SELECT at, principal, lang, round(duration_ms::numeric, 2) AS ms,
       left(query, 120) AS query, error_code
  FROM og_data.og_audit
 WHERE error_code IS NOT NULL
 ORDER BY at DESC
 LIMIT 20;

-- 느린 질의 상위 20건
SELECT round(duration_ms::numeric, 2) AS ms, rows_out, lang,
       left(query, 160) AS query, at
  FROM og_data.og_audit
 ORDER BY duration_ms DESC NULLS LAST
 LIMIT 20;

-- 시간당 호출량과 오류율
SELECT date_trunc('hour', at)                              AS hour,
       count(*)                                            AS calls,
       count(*) FILTER (WHERE error_code IS NOT NULL)      AS errors,
       round(avg(duration_ms)::numeric, 2)                 AS avg_ms,
       round((percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms))::numeric, 2) AS p95_ms
  FROM og_data.og_audit
 WHERE at > now() - interval '24 hours'
 GROUP BY 1
 ORDER BY 1 DESC;

-- 언어별 사용량
SELECT lang, count(*), round(avg(duration_ms)::numeric, 2) AS avg_ms
  FROM og_data.og_audit
 GROUP BY lang ORDER BY 2 DESC;

-- 감사 테이블 자체의 크기 (무한히 자란다 — 보존 정책은 운영자 몫)
SELECT pg_size_pretty(pg_total_relation_size('og_data.og_audit')) AS audit_size,
       count(*) AS rows,
       min(at) AS oldest, max(at) AS newest
  FROM og_data.og_audit;
```

> **필수**: `og_data.og_audit`에는 **자동 정리 장치가 없다.** 질의마다 한 행이 쌓이고,
> `pg_extension_config_dump`에 등록되어 있으므로 **백업에도 통째로 들어간다**
> (`engine/sql/bootstrap.sql:431`). 보존 정책은 [07_maintenance.md](07_maintenance.md) 참조.

Studio는 이 테이블의 최근 100건을 `GET /api/audit`로 노출한다
(`portal/server/index.js:283-293`).

---

## 6. 구조 무결성 — `og_check_integrity`

시그니처: `og_check_integrity() → TABLE(kind text, entity_id int8, detail text)`
(`engine/src/storage/stats.rs:172-180`, `stable`, spec 009 FR-015)

```sql
SELECT * FROM og_check_integrity();     -- 빈 결과 = 통과
SELECT count(*) FROM og_check_integrity();
```

검사 4종 (각각 최대 100행까지만 보고, `engine/src/storage/stats.rs:183-260`):

| `kind` | 검사 내용 | `detail` |
|---|---|---|
| `dangling_adjacency` | `og_adj.eid`가 존재하지 않는 엣지를 가리킴 | `adjacency references a non-existent edge` |
| `missing_adjacency` | 엣지가 양 끝점 중 한쪽에서 도달 불가 | `edge is not reachable from one of its endpoints` |
| `segment_length_mismatch` | `n`과 `array_length(nbr,1)` 또는 `nbr`/`eid` 길이 불일치 | `segment count does not match array length` |
| `orphan_node` | 노드가 알 수 없는 `type_id`를 참조 | `node references an unknown type` |

> **각 검사에 `LIMIT 100`이 걸려 있다.** 결과가 100의 배수에 가깝다면 잘렸을 수 있다.
> 위반의 **총량**을 알고 싶다면 각 SQL을 직접 돌려야 한다.

정기 관측:

```sql
SELECT kind, count(*) AS n
  FROM og_check_integrity()
 GROUP BY kind ORDER BY n DESC;
```

---

## 7. 인트로스펙션 뷰

`engine/sql/access.sql`이 만드는 뷰들. BI·ETL 도구가 그대로 읽을 수 있다 (spec 005 FR-011).

| 뷰 | 컬럼 | 근거 |
|---|---|---|
| `og_type_view` | `graph, type_id, name, kind, is_abstract, storage_table, iri, depth, lft, rgt, parents` | `access.sql:81-97` |
| `og_property_view` | `graph, type_name, property, data_type, column_name, required, is_key` | `access.sql:99-104` |
| `og_role_view` | `graph, relation, role, player_type, ordinal, card_min, card_max` | `access.sql:106-113` |
| `og_node_view` | `id, type_name, graph` | `access.sql:116-120` |
| `og_edge_view` | `id, type_name, graph, src, dst` | `access.sql:122-126` |

```sql
-- 그래프별 노드/엣지 수
SELECT graph, count(*) AS nodes FROM og_node_view GROUP BY graph ORDER BY graph;
SELECT graph, count(*) AS edges FROM og_edge_view GROUP BY graph ORDER BY graph;

-- 타입 계층과 구간 라벨 (lft/rgt 폭 = 서브트리 크기)
SELECT name, kind, is_abstract, depth, lft, rgt, (rgt - lft) AS span, parents, storage_table
  FROM og_type_view
 WHERE graph = 'default'
 ORDER BY lft;

-- 실컬럼으로 승격된 프로퍼티 목록
SELECT type_name, property, data_type, column_name, required, is_key
  FROM og_property_view
 WHERE graph = 'default'
 ORDER BY type_name, property;
```

`og_schema(graph, token_budget)`는 같은 정보를 LLM용으로 압축한 JSON으로 준다
(`engine/src/agent/mod.rs:21-22`) — 인스턴스 수 기준으로 정렬한 뒤 예산에 맞춰 자른다.

```sql
SELECT jsonb_pretty(og_schema('default'));
SELECT jsonb_pretty(og_schema('default', 2000));   -- 토큰 예산
```

---

## 8. 저장 크기와 성장 추이

```sql
-- 확장 소유 릴레이션 크기 상위
SELECT n.nspname || '.' || c.relname                       AS relation,
       pg_size_pretty(pg_total_relation_size(c.oid))       AS total,
       pg_size_pretty(pg_relation_size(c.oid))             AS heap,
       pg_size_pretty(pg_indexes_size(c.oid))              AS indexes,
       c.reltuples::bigint                                 AS est_rows
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace
 WHERE n.nspname IN ('og_catalog', 'og_data')
   AND c.relkind = 'r'
 ORDER BY pg_total_relation_size(c.oid) DESC
 LIMIT 25;
```

핵심 릴레이션 4개만:

```sql
SELECT rel, pg_size_pretty(pg_total_relation_size(rel::regclass)) AS size
  FROM unnest(ARRAY['og_data.og_adj','og_data.og_node','og_data.og_edge','og_data.og_audit']) rel;
```

타입별 저장 테이블(`og_data.n_<type_id>` / `og_data.e_<type_id>`,
`engine/src/catalog/types.rs:69,73`)은 `og_type_view.storage_table`로 이름을 알 수 있다:

```sql
SELECT name, storage_table,
       pg_size_pretty(pg_total_relation_size(storage_table::regclass)) AS size
  FROM og_type_view
 WHERE graph = 'default' AND storage_table IS NOT NULL
 ORDER BY pg_total_relation_size(storage_table::regclass) DESC;
```

---

## 9. PostgreSQL 표준 통계와의 연계

### 테이블 접근 패턴과 vacuum 상태

```sql
SELECT relname,
       n_live_tup, n_dead_tup,
       round(100.0 * n_dead_tup / NULLIF(n_live_tup + n_dead_tup, 0), 1) AS dead_pct,
       seq_scan, idx_scan,
       last_vacuum, last_autovacuum, last_analyze, last_autoanalyze
  FROM pg_stat_user_tables
 WHERE schemaname IN ('og_data', 'og_catalog')
 ORDER BY n_dead_tup DESC
 LIMIT 25;
```

`og_data.og_adj`는 이 표에서 **가장 먼저 봐야 할 행**이다.
세그먼트가 `UPDATE`로 자라고(`engine/src/storage/adjacency.rs:19-46`)
비면 삭제되므로(`:66-71`) dead tuple이 쌓인다.
`fillfactor = 80`(`engine/sql/bootstrap.sql:206`)은 HOT 갱신 여지를 남기려는 설정이다.

### 캐시 적중률 — 순회는 캐시에 살고 죽는다

```sql
SELECT relname,
       heap_blks_read, heap_blks_hit,
       round(100.0 * heap_blks_hit / NULLIF(heap_blks_hit + heap_blks_read, 0), 2) AS heap_hit_pct,
       idx_blks_read, idx_blks_hit
  FROM pg_statio_user_tables
 WHERE schemaname IN ('og_data', 'og_catalog')
 ORDER BY heap_blks_read DESC
 LIMIT 25;
```

`og_data.og_adj`의 `heap_hit_pct`가 이 시스템의 순회 성능을 가장 직접적으로 설명한다 —
배열이 `STORAGE MAIN`으로 인라인이라(`engine/sql/bootstrap.sql:210-211`)
한 노드 확장이 힙 튜플 하나 읽기로 끝나기 때문이다.
벤치마크가 지연시간 옆에 페이지 접근 수를 게시하는 이유도 같다 (`bench/README.md:37-39`).

### 실행 중인 질의

```sql
SELECT pid, state,
       now() - query_start AS running_for,
       wait_event_type, wait_event,
       left(query, 200) AS query
  FROM pg_stat_activity
 WHERE datname = current_database()
   AND state <> 'idle'
 ORDER BY query_start;
```

Cypher는 함수 호출로 들어오므로 `query` 컬럼에는 보통
`SELECT og_cypher($1,$2,$3)` 같은 문장이 보인다. **원문 Cypher는 `og_data.og_audit`에 있다.**
둘을 함께 봐야 그림이 완성된다.

폭주 질의 종료:

```sql
SELECT pg_cancel_backend(<pid>);     -- 먼저 이것
SELECT pg_terminate_backend(<pid>);  -- 안 들으면 이것
```

### 인덱스 사용

```sql
SELECT relname, indexrelname, idx_scan, idx_tup_read, idx_tup_fetch,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
  FROM pg_stat_user_indexes
 WHERE schemaname IN ('og_data', 'og_catalog')
 ORDER BY idx_scan ASC;      -- 안 쓰이는 인덱스가 위로
```

`engine/sql/bootstrap.sql`이 만드는 주요 인덱스: `og_node_type_idx`, `og_edge_type_idx`,
`og_edge_src_idx`, `og_edge_dst_idx`(`:231,239-241`), `type_label_range_idx`,
`type_label_lft_idx`(`:79-80`), `og_audit_at_idx`(`:390`).
`og_data.og_adj`는 별도 인덱스 없이 PK `(src, etype, dir, seq)`로 접근한다 (`engine/sql/bootstrap.sql:205`).

### 데이터베이스 전역

```sql
SELECT datname, numbackends, xact_commit, xact_rollback,
       blks_read, blks_hit,
       round(100.0 * blks_hit / NULLIF(blks_hit + blks_read, 0), 2) AS cache_hit_pct,
       deadlocks, temp_files, pg_size_pretty(temp_bytes) AS temp
  FROM pg_stat_database
 WHERE datname = current_database();
```

`temp_files` / `temp_bytes`가 늘고 있다면 `work_mem`이 부족한 것이다 —
[03_configuration.md](03_configuration.md) 참조.

### `pg_stat_statements`

`docker/Dockerfile.dev`가 `shared_preload_libraries`를 설정하지 않으므로
**기본 개발 이미지에는 활성화되어 있지 않다.** 쓰려면 `postgresql.conf`에
`shared_preload_libraries = 'pg_stat_statements'`를 넣고 재기동한 뒤
`CREATE EXTENSION pg_stat_statements`를 해야 한다.

---

## 10. 최소 관측 대시보드 (한 번에 붙여넣기)

```sql
\echo '=== extension ==='
SELECT ontological_version() AS version, current_database() AS db;

\echo '=== graphs ==='
SELECT graph_id, name, created_at FROM og_catalog.graph ORDER BY name;

\echo '=== graph stats (default) ==='
SELECT jsonb_pretty(og_graph_stats('default'));

\echo '=== integrity (empty = ok) ==='
SELECT kind, count(*) FROM og_check_integrity() GROUP BY kind;

\echo '=== adjacency packing ==='
SELECT count(*) AS segments,
       round(avg(n)::numeric, 2) AS avg_fill,
       round((avg(n)/256)::numeric, 4) AS packing_ratio,
       count(*) FILTER (WHERE seq > 0) AS chunked_supernodes
  FROM og_data.og_adj;

\echo '=== storage ==='
SELECT rel, pg_size_pretty(pg_total_relation_size(rel::regclass)) AS size
  FROM unnest(ARRAY['og_data.og_adj','og_data.og_node','og_data.og_edge','og_data.og_audit']) rel;

\echo '=== dead tuples ==='
SELECT relname, n_live_tup, n_dead_tup, last_autovacuum
  FROM pg_stat_user_tables WHERE schemaname IN ('og_data','og_catalog')
 ORDER BY n_dead_tup DESC LIMIT 10;

\echo '=== audit, last 24h ==='
SELECT count(*) AS calls,
       count(*) FILTER (WHERE error_code IS NOT NULL) AS errors,
       round(avg(duration_ms)::numeric, 2) AS avg_ms
  FROM og_data.og_audit WHERE at > now() - interval '24 hours';

\echo '=== this backend CSR ==='
SELECT * FROM og_csr_stats();
```

---

## 금지 / 필수

### 금지 (Forbidden)

- `og_graph_stats`의 `adjacency` 블록을 그래프별 값으로 읽지 말 것 — DB 전역이다.
- `og_csr_stats()`의 결과를 서버 전체 상태로 해석하지 말 것 — 호출한 백엔드의 상태다.
- `og_check_integrity()`의 행 수를 위반 총량으로 단정하지 말 것 — 검사마다 `LIMIT 100`.
- `pg_stat_activity`의 `query` 컬럼만 보고 어떤 Cypher가 도는지 판단하지 말 것 —
  `og_data.og_audit`를 함께 볼 것.
- 존재하지 않는 관측 함수를 안내하지 말 것. 이 문서에 나온 것이 전부다.

### 필수 (Required)

- 정기 점검 항목: `og_check_integrity()` 행 수, `packing_ratio`,
  `og_data.og_adj`의 `n_dead_tup`, `og_data.og_audit`의 크기, `og_stale_embeddings` 수.
- 벤치마크 숫자를 해석할 때는 `buffers`(논리 페이지 접근)를 지연시간과 함께 볼 것.
- `og_data.og_audit`의 보존 정책을 정할 것 — 자동 정리가 없다.

---

<!-- affects: ops, backend, data -->
<!-- requires-update: docs/08_operations/07_maintenance.md, docs/08_operations/09_troubleshooting.md -->
