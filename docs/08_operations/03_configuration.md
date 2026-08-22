# 설정

> **이 문서가 답하는 질문**
> - 확장 자체의 설정은 어디에 있고 어떻게 바꾸는가?
> - 씨앗으로 들어 있는 설정 키 중 실제로 **읽히는** 것은 무엇인가?
> - PostgreSQL 파라미터는 무엇을 어떻게 맞춰야 하는가?
> - Studio와 Bolt 게이트웨이의 환경변수는?

---

## 1. 확장 설정 — `og_catalog.setting`

### 저장 위치와 형태

```sql
CREATE TABLE og_catalog.setting (
    key   text PRIMARY KEY,
    value text NOT NULL
);
```

(`engine/sql/bootstrap.sql:252-255`)

키·값 모두 `text`다. 타입 검증은 읽는 쪽에서 한다.

### 쓰기 — `og_set_setting`

```sql
SELECT og_set_setting('genai.enabled', 'on');
```

구현은 upsert 한 줄이다 (`engine/src/compat/genai.rs:56-63`):

```sql
INSERT INTO og_catalog.setting (key, value) VALUES ($1, $2)
ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value
```

실패 시 오류 문자열: `failed to set '{key}': {e}` (`engine/src/compat/genai.rs:62`).

읽기는 표준 SELECT로 한다:

```sql
SELECT key, value FROM og_catalog.setting ORDER BY key;
```

### 씨앗 키 4개 — 실측 평가

`engine/sql/bootstrap.sql:256-260`이 심는 값:

```sql
INSERT INTO og_catalog.setting (key, value) VALUES
    ('chunk_size',        '256'),
    ('supernode_threshold','4096'),
    ('inference_max_depth','16'),
    ('schema_version',    '1');
```

**이 네 키를 코드가 실제로 읽는지 저장소 전체를 grep한 결과:**

| 키 | 씨앗값 | 코드가 읽는가 | 근거 |
|---|---|---|---|
| `chunk_size` | `256` | **아니다** | 인접 세그먼트 크기는 컴파일 타임 상수 `pub const CHUNK: i32 = 256;` (`engine/src/storage/adjacency.rs:15`). `og_graph_stats`도 이 상수를 읽는다 (`engine/src/storage/stats.rs:68`) |
| `supernode_threshold` | `4096` | **아니다** | `engine/src/`, `engine/sql/` 어디에서도 참조되지 않는다 |
| `inference_max_depth` | `16` | **아니다** | 위와 동일 |
| `schema_version` | `1` | **아니다** (이 키로는) | 스키마 버전은 별도 테이블 `og_catalog.schema_version`에서 관리된다 (`engine/src/agent/mod.rs:25`, `engine/src/catalog/labeling.rs:172-179`) |

> **금지 (Forbidden)**: `og_set_setting('chunk_size', '512')` 같은 호출로
> 동작이 바뀔 것이라 기대하지 말 것. **아무 효과도 없다.**
> 세그먼트 크기를 바꾸려면 `engine/src/storage/adjacency.rs:15`를 고치고 재빌드해야 한다.
> → [10_improvements_ops.md](10_improvements_ops.md) `OPS-10`

이 네 키는 백업 대상에서도 제외되어 있다 (`engine/sql/bootstrap.sql:420-422`),
확장 스크립트가 다시 심기 때문이다.

### 실제로 읽히는 설정 키 — `genai.*`

읽히는 유일한 설정군은 `genai.vector.encode`(= `og_genai_encode`)의 것이다.
읽는 함수는 `setting()` (`engine/src/compat/genai.rs:43-51`)이며,
빈 문자열은 미설정으로 취급한다.

| 키 | 기본값 | 의미 | 근거 |
|---|---|---|---|
| `genai.enabled` | (없음 = 비활성) | `'on'`이 아니면 `og_genai_encode`가 거부한다 | `engine/src/compat/genai.rs:101-107` |
| `genai.endpoint` | (없음 = 오류) | 임베딩 HTTP 엔드포인트 URL. **인자로 받지 않는다** — 질의 권한이 곧 fetch 권한이 되지 않도록 하기 위함 | `engine/src/compat/genai.rs:108-113`, 모듈 주석 `:21-24` |
| `genai.provider` | `ollama` | `ollama` \| `openai` \| `azureopenai` 만 허용 | `engine/src/compat/genai.rs:116-126` |
| `genai.model` | (없음 = 오류) | 모델 이름. 호출 시 `configuration->>'model'`로 덮어쓸 수 있다 | `engine/src/compat/genai.rs:128-133` |
| `genai.dimensions` | (없음 = 자르지 않음) | 반환 벡터를 이 길이로 절단 후 재정규화 | `engine/src/compat/genai.rs:160-169` |
| `genai.timeout_ms` | `5000` | HTTP 타임아웃(ms). 상수 `DEFAULT_TIMEOUT_MS` | `engine/src/compat/genai.rs:41,135-137` |
| `genai.token` | (없음) | 설정되면 `Authorization: Bearer <token>` 헤더로 전송 | `engine/src/compat/genai.rs:140-142` |

설정 예 (모듈 주석 `engine/src/compat/genai.rs:29-35`에 있는 그대로):

```sql
SELECT og_set_setting('genai.enabled',  'on');
SELECT og_set_setting('genai.endpoint', 'http://localhost:11434/api/embed');
SELECT og_set_setting('genai.provider', 'ollama');
SELECT og_set_setting('genai.model',    'qwen3-embedding:latest');
SELECT og_set_setting('genai.dimensions', '1024');
```

> **운영 관점 경고**: 이 함수는 **PostgreSQL 백엔드가 직접 외부 HTTP 요청을 보낸다.**
> `engine/Cargo.toml:25-29`의 주석이 명시하듯 블로킹 `ureq` 클라이언트이므로,
> 요청이 진행되는 동안 그 백엔드는 다른 질의를 처리하지 않는다.
> 기본이 비활성인 것은 의도된 설계다. 켤 때는 `genai.timeout_ms`를 함께 짧게 잡을 것.
> `genai.dimensions`를 2000 이하로 두는 것도 실무적으로 필요하다 — pgvector의 HNSW
> 인덱스가 2000차원에서 멈추기 때문이다 (`engine/src/compat/genai.rs:155-159` 주석).

### 히스토리 설정 키

`og_enable_history(graph, type_name)`은 트리거를 만들면서
`history.<graph>.<type_name>` 키를 `'on'`으로 기록한다
(`engine/src/agent/mod.rs:462-467`). 이 키는 **기록용**이며, 조회 경로가 이 값을 보고
분기하지는 않는다 — `og_as_of`는 `og_data.og_history`에 행이 있는지로 판단한다
(`engine/src/agent/mod.rs:503-516`).

```sql
-- 어떤 타입에 히스토리가 켜져 있는가
SELECT key, value FROM og_catalog.setting WHERE key LIKE 'history.%' ORDER BY key;
```

---

## 2. 에이전트 역할 — 세션 리소스 제한

`og_create_role` / `og_apply_role`은 세션에 리소스 제한을 거는 경로다
(`engine/src/agent/mod.rs:405-441`).

```sql
SELECT og_create_role('analyst', '{
  "statement_timeout_ms": 5000,
  "work_mem_kb": 65536,
  "read_only": true,
  "max_rows": 10000
}'::jsonb);

SELECT og_apply_role('analyst');
```

`og_apply_role`이 실제로 실행하는 `SET` (`engine/src/agent/mod.rs:426-439`):

| limits 키 | 실행되는 문장 | 실효성 |
|---|---|---|
| `statement_timeout_ms` | `SET statement_timeout = {ms}` | **실효** — PostgreSQL 표준 GUC |
| `work_mem_kb` | `SET work_mem = {mem}` | **실효** — PostgreSQL 표준 GUC |
| `read_only` (true) | `SET default_transaction_read_only = on` | **실효** — 표준 GUC |
| `max_rows` | `SET og.max_rows = {rows}` | **무효** — `og.max_rows`를 읽는 코드가 저장소에 없다 |

`og.max_rows` 확인: `engine/src/agent/mod.rs:438`이 유일한 참조이며,
`current_setting('og.max_rows')`를 호출하는 곳은 없다. → `OPS-11`

또한 네 개의 `Spi::run(...)`이 모두 `.ok()`로 끝나므로 **`SET` 실패는 조용히 무시된다**
(`engine/src/agent/mod.rs:427,430,434,438`).

존재하지 않는 역할을 적용하면: `no agent role named '{name}'`
(`engine/src/agent/mod.rs:424`).

---

## 3. PostgreSQL 파라미터

> 아래 표에서 **사실**은 저장소에서 확인된 값이고, **권장**은 그 사실로부터 도출한 판단이다.
> 저장소에는 `postgresql.conf` 템플릿이 없으므로 권장값에 근거 파일이 붙지 않는다.

### 사실 — 스토리지 레이아웃이 만드는 제약

| 사실 | 근거 |
|---|---|
| `og_data.og_adj`는 `fillfactor = 80`으로 만들어진다 | `engine/sql/bootstrap.sql:206` |
| `nbr`/`eid` 배열은 `STORAGE MAIN`으로 강제되어 TOAST되지 않는다. 세그먼트당 최대 256 이웃 × 8B × 2 = 4KB로 8KB 힙 페이지 안에 들어간다 | `engine/sql/bootstrap.sql:210-211`, `engine/src/storage/adjacency.rs:13-15` |
| `og_reach`와 `og_csr_*`는 `PARALLEL RESTRICTED`로 선언된다 — 병렬 워커에서 실행되지 않는다 | `engine/src/storage/traverse.rs:80,359,442`, `docs/deep-traversal.md:263-267` |
| 깊은 순회 재작성 여부는 **플래너 통계**로 결정된다. 통계가 없으면 깊이만 보고 판단한다 | `docs/deep-traversal.md:220-226` — "An unanalysed database has no statistics to answer with and falls back to depth alone" |
| Cypher 컴파일 결과는 평범한 SQL이며 플래너가 순회 스캔을 직접 본다 | `engine/src/storage/mod.rs:7-9` 주석 |

### 권장 (판단)

| 파라미터 | 권장 | 이유 |
|---|---|---|
| `shared_buffers` | 인접 세그먼트 전체가 들어갈 만큼 | `og_adj`가 순회의 핫 릴레이션이고 배열이 인라인이므로 캐시 적중이 곧 성능이다. 필요 크기는 `pg_total_relation_size('og_data.og_adj')`로 실측할 것 — [06_monitoring.md](06_monitoring.md) |
| `work_mem` | 기본보다 넉넉히 | `og_reach`의 방문집합과 집계가 여기서 나온다. 세션 단위로는 `og_apply_role`의 `work_mem_kb`로도 걸 수 있다 |
| `statement_timeout` | 반드시 설정 | 가변 길이 매치가 재작성되지 못하는 형태(경로 변수 바인딩 등, `docs/deep-traversal.md:197-215`)면 `degree^k` 행을 열거한다. 벤치 하네스도 같은 이유로 기본 120초 캡을 둔다 (`bench/harness.py:1263-1265`) |
| `autovacuum` | 켠 채로 두되 `og_adj`는 별도 관찰 | 세그먼트가 `UPDATE`로 자라고 비면 삭제된다 (`engine/src/storage/adjacency.rs:65-71`) — [07_maintenance.md](07_maintenance.md) |
| `max_parallel_workers_per_gather` | 깊은 순회 성능 기대치에 포함하지 말 것 | 순회 함수들이 `PARALLEL RESTRICTED`라 리더에서만 돈다 |
| `pg_stat_statements` | 필요하면 **직접 추가** | `docker/Dockerfile.dev`가 `shared_preload_libraries`를 설정하지 않으므로 기본 이미지에는 활성화되어 있지 않다 |

pgrx 관리 인스턴스의 설정 파일을 고치려면 `cargo pgrx`가 만든 데이터 디렉터리의
`postgresql.conf`를 직접 수정한 뒤 재기동해야 한다. 저장소에는 그 경로가 명시되어 있지
않으므로 **미확인**이며, 다음으로 실측할 수 있다:

```bash
docker exec ontological-dev bash -lc \
  "psql -h localhost -p 28816 -d postgres -tAc 'SHOW config_file'"
docker exec ontological-dev bash -lc \
  "psql -h localhost -p 28816 -d postgres -tAc 'SHOW data_directory'"
```

세션 단위 설정은 `postgresql.conf` 없이도 즉시 확인·변경할 수 있다:

```sql
SHOW statement_timeout;
SHOW work_mem;
SHOW shared_buffers;
SET statement_timeout = '30s';
```

---

## 4. Studio 환경변수

`portal/server/index.js:17-29`가 읽는 전부:

| 변수 | 기본값 | 의미 | 근거 |
|---|---|---|---|
| `PORT` | `7474` | HTTP 리스닝 포트 | `portal/server/index.js:17` |
| `PGHOST` | `localhost` | PostgreSQL 호스트 | `:22` |
| `PGPORT` | `28816` | PostgreSQL 포트 | `:23` |
| `PGDATABASE` | `og` | 데이터베이스 | `:24` |
| `PGUSER` | `dev` | 역할 | `:25` |
| `PGPASSWORD` | `undefined` | 비밀번호. 미설정이면 `pg`가 무인증/`.pgpass` 등에 의존 | `:26` |
| `OG_BENCH_DIR` | `<repo>/bench/results` | 벤치마크 리포트가 읽을 디렉터리 | `:19` |

풀 설정은 환경변수가 아니라 **하드코딩**되어 있다 (`portal/server/index.js:27-28`):

```js
max: 8,
idleTimeoutMillis: 30_000,
```

> **운영 관점**: 커넥션 8개가 상한이고 질의 타임아웃 설정이 없다.
> 장기 질의 8개가 동시에 걸리면 `/api/health`를 포함한 모든 엔드포인트가 대기한다.
> `POST /api/sql`은 임의 SQL을 그대로 풀에 넘긴다 (`portal/server/index.js:296-308`).
> → `OPS-12`

정적 파일은 `portal/web/`에서 서빙되며, 경로 탈출은 `full.startsWith(WEB_DIR)` 검사로
막는다 (`portal/server/index.js:359-360`).

`OG_BENCH_DIR`은 `/^bench-(\d+)-(\d{8}T\d{6}Z)\.json$/` 패턴에 맞는 파일만 읽는다
(`portal/server/index.js:91`) — `baseline.json`은 리포트에 포함되지 않는다.

---

## 5. Bolt 게이트웨이 환경변수

설정 수단은 **환경변수뿐이다** (`bolt/src/main.rs:9` 주석: "Configuration is environment only").

| 변수 | 기본값 | 의미 | 근거 |
|---|---|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` | Bolt 연결을 받을 주소 | `bolt/src/main.rs:37` |
| `OG_BOLT_PGHOST` | `localhost` | PostgreSQL 호스트 | `bolt/src/main.rs:39` |
| `OG_BOLT_PGPORT` | `5432` | PostgreSQL 포트. **파싱 실패 시 조용히 5432로 되돌아간다** (`.parse().unwrap_or(5432)`) | `bolt/src/main.rs:40` |
| `OG_BOLT_PGDATABASE` | `og` | 데이터베이스 | `bolt/src/main.rs:41` |
| `OG_BOLT_GRAPH` | `default` | 세션이 database를 지정하지 않았을 때 쓸 그래프 | `bolt/src/main.rs:42` |
| `OG_BOLT_ADVERTISED` | `OG_BOLT_LISTEN`과 동일 | 라우팅 테이블에서 알릴 주소 | `bolt/src/main.rs:43` |

> **함정**: `OG_BOLT_PGPORT`의 기본값 `5432`는 이 프로젝트의 실제 포트 `28816`과 다르다.
> 환경변수 없이 바이너리만 실행하면 **엉뚱한 서버에 붙거나 붙지 못한다.**
> `start.sh:62`가 이 값을 명시적으로 넘기는 이유다.

인증은 게이트웨이가 하지 않는다. `HELLO`의 user/password는 **PostgreSQL 역할과 그 비밀번호**
이며, 게이트웨이는 인증된 사용자 외의 누구로도 접속하지 않는다
(`bolt/README.md:43`, `bolt/README.md:49-51`). 따라서 RLS·권한·감사는 모두 PostgreSQL의 것이다.

TLS는 종단하지 않는다 — 앞단에 TLS 프록시를 두라고 명시되어 있다 (`bolt/README.md:68`).

기동 시 stderr로 한 줄을 남긴다 (`bolt/src/main.rs:53-57`):

```
ontological-bolt: listening on 0.0.0.0:7687, forwarding to postgres://localhost:28816/og (default graph 'default')
```

바인드 실패는 exit code 1 (`bolt/src/main.rs:49-51`):

```
ontological-bolt: cannot bind {listen}: {e}
```

---

## 금지 / 필수

### 금지 (Forbidden)

- 씨앗 설정 키 4종(`chunk_size`, `supernode_threshold`, `inference_max_depth`,
  `schema_version`)을 튜닝 노브로 취급하지 말 것 — 읽히지 않는다.
- `og_apply_role`의 `max_rows`를 행 수 제한으로 신뢰하지 말 것 — 읽히지 않는다.
- `genai.endpoint`를 신뢰할 수 없는 URL로 설정하지 말 것 — 백엔드가 그 URL로 직접 요청한다.
- Bolt 게이트웨이를 `OG_BOLT_PGPORT` 없이 기동하지 말 것.
- Studio를 공개망에 노출하지 말 것 — `POST /api/sql`이 임의 SQL 통로다.

### 필수 (Required)

- `genai.*`를 켤 때는 `genai.timeout_ms`를 반드시 함께 설정할 것.
- 세션 단위 가드레일이 필요하면 `og_apply_role`의 **`statement_timeout_ms` / `work_mem_kb` /
  `read_only`** 세 개만 쓸 것 (실효가 확인된 것).
- 설정 변경 후에는 `SELECT key, value FROM og_catalog.setting ORDER BY key`로 실제 반영을 확인할 것.

---

<!-- affects: ops, backend -->
<!-- requires-update: docs/08_operations/06_monitoring.md, docs/08_operations/09_troubleshooting.md -->
