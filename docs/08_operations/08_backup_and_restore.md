# 백업과 복원

> **이 문서가 답하는 질문**
> - `pg_dump` 하나로 그래프가 온전히 보존되는가? 그 근거는 무엇인가?
> - 무엇이 백업되고 무엇이 백업되지 않는가?
> - 복원 순서는? 확장 버전이 다르면 어떻게 되는가?
> - 백엔드-로컬 CSR은 백업 대상인가?
> - 백업이 제대로 됐는지 어떻게 검증하는가?

---

## 결론

**표준 `pg_dump` / `pg_restore`로 충분하다.** 이 확장은 그것을 위해 명시적으로 설계되었고,
회귀 스위트가 매번 그 사실을 검증한다.

근거 세 가지:

1. `engine/sql/bootstrap.sql:9-10` — "Constitution IX: every structure below is an ordinary
   heap relation, so it inherits MVCC / WAL / vacuum / **pg_dump** for free."
2. `engine/sql/bootstrap.sql:392-448` — 확장 소유 릴레이션 **38개**가
   `pg_extension_config_dump()`로 등록되어 있다.
3. `tests/run.sh:36-67` — 백업 왕복이 **별도 게이트**로 존재한다.

---

## 사실 (Facts) — 왜 등록이 필요한가

`engine/sql/bootstrap.sql:393-401`의 주석이 위험을 정확히 진술한다:

> Tables created by a `CREATE EXTENSION` script belong to the extension, and
> pg_dump emits only `CREATE EXTENSION` for them: their CONTENTS are skipped.
> Every relation below holds user data, so each one has to be registered as
> configuration data or a dump would **silently restore an empty graph**.

즉 등록이 없으면 `pg_dump`는 성공하고, `pg_restore`도 성공하고, **그래프만 비어 있다.**
오류가 나지 않기 때문에 이것이 별도 테스트 게이트를 가진 이유다 (`tests/run.sh:37-39`).

### 등록된 릴레이션 — 전체 목록 (실측 38개)

**`og_catalog` — 카탈로그 테이블 15개** (`engine/sql/bootstrap.sql:403-422`)

| # | 릴레이션 | 필터 |
|---|---|---|
| 1 | `og_catalog.graph` | 없음 |
| 2 | `og_catalog.type` | 없음 |
| 3 | `og_catalog.type_parent` | 없음 |
| 4 | `og_catalog.type_label` | 없음 |
| 5 | `og_catalog.property` | 없음 |
| 6 | `og_catalog.role` | 없음 |
| 7 | `og_catalog.og_constraint` | 없음 |
| 8 | `og_catalog.rule` | 없음 |
| 9 | `og_catalog.schema_version` | 없음 |
| 10 | `og_catalog.embedding` | 없음 |
| 11 | `og_catalog.compat_index` | 없음 |
| 12 | `og_catalog.prefix` | 없음 |
| 13 | `og_catalog.mapping` | 없음 |
| 14 | `og_catalog.agent_role` | 없음 |
| 15 | `og_catalog.typeql_function` | 없음 |
| 16 | `og_catalog.setting` | **있음** ↓ |

`og_catalog.setting`만 필터가 걸려 있다 (`engine/sql/bootstrap.sql:420-422`):

```sql
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                       ''inference_max_depth'', ''schema_version'')');
```

주석 그대로: "The extension script seeds these keys, so restoring them again would collide."
→ **씨앗 키 4개는 백업되지 않는다. 나머지 설정(`genai.*`, `history.*` 등)은 백업된다.**

**`og_data` — 데이터 테이블 11개** (`engine/sql/bootstrap.sql:424-434`)

`og_node`, `og_edge`, `og_adj`, `og_id_alloc`, `og_role_player`, `og_history`,
`og_source`, `og_audit`, `og_embedding_state`, `og_iri`, `og_triple_overflow`

**시퀀스 11개** (`engine/sql/bootstrap.sql:436-448`)

`og_catalog.graph_id_seq`, `type_id_seq`, `property_id_seq`, `role_id_seq`,
`constraint_id_seq`, `rule_id_seq`, `schema_version_seq`, `embedding_id_seq`,
`og_data.og_history_hist_id_seq`, `og_data.og_audit_audit_id_seq`,
`og_data.og_triple_overflow_id_seq`

주석이 이유를 밝힌다 (`engine/sql/bootstrap.sql:436-437`):
"Sequences carry the allocation watermarks; losing them would hand out identifiers
that already exist."

### 등록이 필요 없는 것 — 타입별 저장 테이블

`engine/sql/bootstrap.sql:399-401`:

> Per-type tables (`og_data.n_*`, `og_data.e_*`) are created at run time, not by
> this script, so they are ordinary user tables and pg_dump already covers them.

즉 `og_data.n_<type_id>` / `og_data.e_<type_id>`
(`engine/src/catalog/types.rs:69,73`)는 확장 소유가 아니라 **평범한 사용자 테이블**이라
자동으로 덤프된다. 프로퍼티 값(실컬럼)과 벡터 컬럼이 여기 있다.

타입 이름의 별칭 뷰(`og_data."Film"` 등, `engine/src/catalog/types.rs:89-97`)도 마찬가지로
런타임 생성물이므로 덤프에 포함된다.

### 백업 대상이 **아닌** 것

| 대상 | 이유 |
|---|---|
| **백엔드-로컬 CSR** (`og_csr_build`) | 프로세스 메모리에만 존재한다 (`engine/src/storage/traverse.rs:20-23`). 연결이 끊기면 사라진다. 복원 후 필요하면 `og_csr_build()`를 다시 부르면 된다 — 원본은 `og_data.og_adj`이고 그것은 백업된다 |
| Cypher 컴파일 캐시 / 생성된 유니온 뷰 | 스키마 변경 시 드롭·재생성되는 파생물 (`engine/src/catalog/labeling.rs:172-175`) |
| 씨앗 설정 키 4개 | 확장 스크립트가 다시 심는다 (위 필터) |
| Studio / Bolt 프로세스 상태 | 상태를 갖지 않는다 (`bolt/README.md:12`) |
| PostgreSQL 데이터 디렉터리 자체 | 컨테이너 레이어에 있다 — [02_startup_and_teardown.md](02_startup_and_teardown.md) 단계 1 참조 |

---

## 백업 절차

### 논리 백업 (권장)

```bash
# 컨테이너 안에서 (psql/pg_dump 버전이 서버와 일치)
docker exec ontological-dev bash -lc \
  'pg_dump -h localhost -p 28816 -d og -Fc -f /tmp/og-$(date -u +%Y%m%dT%H%M%SZ).dump'

docker cp ontological-dev:/tmp/og-<timestamp>.dump ./backups/
```

포맷 선택:

| 포맷 | 명령 | 복원 |
|---|---|---|
| custom (권장) | `pg_dump -Fc -f x.dump` | `pg_restore -d <db> x.dump` — 병렬 복원, 선택 복원 가능 |
| plain SQL | `pg_dump -f x.sql` | `psql -d <db> -f x.sql` — `tests/run.sh:51,54`가 쓰는 방식 |

`tests/run.sh:51`이 검증하는 형태는 plain이다:

```bash
pg_dump -h "$HOST" -p "$PORT" -d "$DB" -f /tmp/og_roundtrip.dump
```

### 백업 직전 점검 (필수)

```sql
-- 1. 무결성 — 깨진 상태를 백업하면 깨진 상태가 복원된다
SELECT count(*) FROM og_check_integrity();

-- 2. 나중에 대조할 카운트를 기록해 둔다
SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view) AS node_edge;

-- 3. 확장 버전을 기록해 둔다 — 복원 시 대조용
SELECT ontological_version();
```

3번이 특히 중요하다. **덤프 파일에는 `CREATE EXTENSION ontological` 한 줄이 들어가고,
그것이 어떤 스키마를 만들지는 복원 시점에 설치되어 있는 `.so`가 결정한다.**

---

## 복원 절차

### 표준 경로

```bash
# 1. 대상 서버에 확장이 설치되어 있는지 확인 (스키마를 만드는 주체)
docker exec ontological-dev bash -lc \
  "psql -h localhost -p 28816 -d postgres -tAc \
   \"SELECT default_version FROM pg_available_extensions WHERE name='ontological'\""

# 2. 빈 데이터베이스
docker exec ontological-dev bash -lc 'createdb -h localhost -p 28816 og_restored'

# 3. 복원
docker exec ontological-dev bash -lc \
  'pg_restore -h localhost -p 28816 -d og_restored /tmp/og-<timestamp>.dump'
#   plain SQL 이면:
#   psql -h localhost -p 28816 -d og_restored -f /tmp/og-<timestamp>.dump
```

> **`CREATE EXTENSION`을 미리 실행하지 말 것.** 덤프 안에 이미 들어 있다.
> 미리 만들면 복원 중 충돌한다.

> **pgvector가 대상 서버에 있어야 한다.** `requires = 'vector'`
> (`engine/ontological.control:7`). 없으면 복원이 `CREATE EXTENSION ontological`에서 멈춘다.

### 복원 검증 — `tests/run.sh`가 하는 것과 동일

```bash
DB=og_restored
psql -h localhost -p 28816 -d $DB -tAc \
  "SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)"
# → 백업 직전에 기록한 값과 같아야 한다

psql -h localhost -p 28816 -d $DB -tAc \
  "SELECT og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')"
# → 비어 있지 않아야 한다 (Cypher 경로가 살아 있다는 증거)

psql -h localhost -p 28816 -d $DB -c "SELECT * FROM og_check_integrity()"
# → 빈 결과
```

`tests/run.sh:60`의 통과 조건을 그대로 옮기면:

```
before = after   AND   before ≠ ""   AND   before ≠ "0/0"   AND   query_ok ≠ ""
```

`before ≠ "0/0"` 조건이 붙은 이유가 핵심이다 —
**빈 그래프도 "일치"하기 때문에, 0/0을 통과로 세면 아무것도 검증하지 못한다.**

### 확장 버전 불일치 시

현재 `default_version`은 `0.1.0` 하나뿐이고, **업그레이드 스크립트가 없다**
(`engine/sql/`에 `bootstrap.sql`과 `access.sql`만 존재).
따라서 상황은 아래 세 가지로 정리된다.

| 상황 | 결과 | 대응 |
|---|---|---|
| 백업 서버 `.so`와 복원 서버 `.so`가 **같은 커밋** | 정상 복원 | 없음 |
| `.so`는 다르지만 `bootstrap.sql`/`access.sql`이 **동일** | 정상 복원. 함수 시그니처가 바뀌었다면 애플리케이션 쪽에서 깨질 수 있다 | 회귀 스위트로 확인 |
| `bootstrap.sql`/`access.sql`이 **다름** (테이블·컬럼 변경) | 덤프의 `COPY` 문이 새 스키마와 맞지 않아 **복원 중 오류** 또는 조용한 데이터 누락 | 아래 이관 절차 |

**스키마가 바뀐 버전으로 이관하는 절차** (업그레이드 스크립트가 없으므로 이것이 유일한 경로):

```bash
# 1. 구 버전 서버에서 논리 표현으로 뽑는다 — 물리 스키마가 아니라 그래프 자체를
psql -h old -p 28816 -d og -tAc \
  "SELECT og_dump_rdf('default', 'turtle')" > graph.ttl
#    또는 애플리케이션 수준의 Cypher/TypeQL 스크립트

# 2. 신 버전으로 빈 데이터베이스를 만든다
createdb -h new -p 28816 og
psql -h new -p 28816 -d og -c 'CREATE EXTENSION ontological CASCADE'

# 3. 타입을 선언하고 데이터를 적재한다
psql -h new -p 28816 -d og -c "SELECT og_load_rdf('default', pg_read_file('graph.ttl'), 'turtle')"
```

> `og_dump_rdf` / `og_load_rdf`의 정확한 시그니처와 커버리지는
> [`docs/api.md`](../api.md)의 "Semantic web — spec 006" 절을 확인할 것.
> spec 006은 partial 상태이며 SPARQL은 미구현이다.
> **RDF 왕복이 그래프의 모든 측면(벡터 임베딩, 히스토리, 감사 로그)을 보존하지는 않는다.**

> **필수**: 백업 파일과 함께 **그때의 커밋 해시 또는 `ontological_version()` 값**을 보관할 것.
> 확장 버전이 `0.1.0`으로 고정되어 있어 버전 번호만으로는 스키마를 식별할 수 없다.
> → [10_improvements_ops.md](10_improvements_ops.md) `OPS-02`, `OPS-17`

---

## 파일시스템/물리 백업

pgrx가 관리하는 인스턴스이므로 데이터 디렉터리 위치는 `SHOW data_directory`로 확인한다
([03_configuration.md](03_configuration.md) §3).

물리 백업(`pg_basebackup`, 파일 복사)은 표준 PostgreSQL 규칙을 그대로 따른다:

- 서버를 정지하거나 `pg_start_backup`/`pg_backup_start`를 쓸 것.
- 복원 대상은 **같은 메이저 버전**이어야 한다 (PostgreSQL 16).
- **확장 `.so` 파일도 함께 옮겨야 한다.** `$libdir/ontological.so`와
  확장 디렉터리의 `ontological.control` / `ontological--0.1.0.sql`이
  대상 서버에 없으면 데이터베이스는 열려도 함수가 없다.

> `start.sh`가 만드는 컨테이너에는 **PGDATA용 볼륨이 없다**
> (`start.sh:21-28`은 `ontological-target`와 `ontological-cargo` 두 개만 만든다).
> `docker rm` 한 번이 곧 데이터 소실이다. → `OPS-06`

---

## 정기 백업 예시

```bash
#!/usr/bin/env bash
# Daily logical backup with a verified round trip.
set -euo pipefail

CONTAINER=ontological-dev
PGPORT=28816
DB=og
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT=/tmp/og-${STAMP}.dump

# 1. integrity gate — never archive a broken graph silently
violations=$(docker exec "$CONTAINER" bash -lc \
  "psql -h localhost -p $PGPORT -d $DB -tAc 'SELECT count(*) FROM og_check_integrity()'")
if [ "$violations" != "0" ]; then
    echo "integrity violations: $violations — aborting backup" >&2
    exit 1
fi

# 2. record what we expect to see after a restore
before=$(docker exec "$CONTAINER" bash -lc \
  "psql -h localhost -p $PGPORT -d $DB -tAc \
   \"SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)\"")
version=$(docker exec "$CONTAINER" bash -lc \
  "psql -h localhost -p $PGPORT -d $DB -tAc 'SELECT ontological_version()'")

# 3. dump
docker exec "$CONTAINER" bash -lc "pg_dump -h localhost -p $PGPORT -d $DB -Fc -f $OUT"
docker cp "$CONTAINER:$OUT" "./backups/og-${STAMP}.dump"
printf '%s\n' "counts=$before extension_version=$version stamp=$STAMP" \
  > "./backups/og-${STAMP}.meta"

echo "backup ok: ./backups/og-${STAMP}.dump ($before nodes/edges, ext $version)"
```

복원 리허설(분기 1회 권장):

```bash
docker exec ontological-dev bash -lc 'createdb -h localhost -p 28816 og_drill'
docker exec ontological-dev bash -lc 'pg_restore -h localhost -p 28816 -d og_drill /tmp/og-<stamp>.dump'
docker exec ontological-dev bash -lc \
  "psql -h localhost -p 28816 -d og_drill -tAc \
   \"SELECT (SELECT count(*) FROM og_node_view) || '/' || (SELECT count(*) FROM og_edge_view)\""
# → .meta 파일의 counts 와 일치해야 한다
docker exec ontological-dev bash -lc 'psql -h localhost -p 28816 -d postgres -c "DROP DATABASE og_drill"'
```

---

## 회귀 스위트가 백업을 검증하는 방식

`tests/run.sh:41-67`을 그대로 요약:

1. 새 DB → `CREATE EXTENSION ontological CASCADE` → `examples/demo.sql`
2. `before` = `og_node_view` / `og_edge_view` 카운트
3. `pg_dump -f /tmp/og_roundtrip.dump`
4. `${OG_TEST_DB}_restored` 생성 → `psql -f`로 복원
5. `after` = 같은 카운트, `query_ok` = `og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')`
6. 네 조건 모두 만족해야 통과 (위 "복원 검증" 절)

출력 예:

```
backup round trip                 ok (69/104 nodes/edges preserved)
```

> **확장의 SQL 스키마에 테이블을 추가했다면, `pg_extension_config_dump` 등록도 함께
> 추가해야 한다.** 잊으면 이 게이트가 잡아준다 — 그것이 이 게이트의 존재 이유다.

---

## 금지 / 필수

### 금지 (Forbidden)

- 복원 전에 `CREATE EXTENSION ontological`을 미리 실행하지 말 것 — 덤프에 들어 있다.
- 무결성 위반이 있는 상태를 백업 아카이브에 넣지 말 것.
- 백업 성공 여부를 `pg_dump`의 종료 코드만으로 판단하지 말 것 —
  등록 누락은 오류 없이 빈 그래프를 만든다.
- `docker rm ontological-dev`를 백업 없이 실행하지 말 것 — PGDATA가 볼륨에 없다.
- 백엔드-로컬 CSR을 백업하려 하지 말 것 — 프로세스 메모리이며 `og_csr_build()`로 재생성한다.
- 확장 SQL 스키마가 바뀐 버전 사이에서 `pg_dump` 파일을 그대로 옮기지 말 것.

### 필수 (Required)

- 백업 파일마다 `ontological_version()`과 **커밋 해시**를 함께 보관할 것.
- 백업 전 `og_check_integrity()`를 통과시킬 것.
- 복원 후 **노드/엣지 카운트 + Cypher 실행 + 무결성** 세 가지를 모두 확인할 것.
- 확장 스키마에 테이블을 추가하는 변경을 했다면 `pg_extension_config_dump` 등록과
  `tests/run.sh`의 백업 왕복 통과를 함께 확인할 것.

---

<!-- affects: ops, data, backend -->
<!-- requires-update: docs/08_operations/07_maintenance.md, docs/08_operations/09_troubleshooting.md -->
