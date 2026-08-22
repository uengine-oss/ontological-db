# 설치

> **이 문서가 답하는 질문**
> - 이 확장을 설치하려면 정확히 무엇이 필요한가?
> - 어떤 PostgreSQL 버전을 지원한다고 **선언**되어 있고, 어떤 버전이 실제로 **검증**되었는가?
> - pgvector는 왜 필수인가? `CASCADE`를 빼면 어떻게 되는가?
> - Docker 없이 로컬에 설치하려면?

---

## 사실 (Facts)

### 확장 메타데이터

`engine/ontological.control` 전문 (7줄, `engine/ontological.control:1-7`):

```
comment = 'Ontological — Cypher-native ontology graph database for the AI agent era'
default_version = '0.1.0'
module_pathname = '$libdir/ontological'
relocatable = false
superuser = false
trusted = false
requires = 'vector'
```

여기서 운영에 직결되는 항목:

| 키 | 값 | 운영상 의미 |
|---|---|---|
| `default_version` | `0.1.0` | `CREATE EXTENSION ontological`은 항상 0.1.0을 만든다. 업그레이드 스크립트가 없으므로 **다른 버전은 존재하지 않는다** |
| `relocatable` | `false` | `ALTER EXTENSION ontological SET SCHEMA …` 불가. 스키마는 `og_catalog` / `og_data`로 고정 (`engine/sql/bootstrap.sql:13-14`) |
| `superuser` | `false` | 확장 스크립트가 **호출자 권한으로** 실행된다. 즉 스키마 생성 권한이 있는 역할이면 슈퍼유저가 아니어도 설치를 시도할 수 있다. 다만 이 저장소의 어떤 스크립트도 비슈퍼유저 설치를 실행하지 않으므로 **비슈퍼유저 설치 경로는 미검증** |
| `trusted` | `false` | `superuser = false`이므로 `trusted`는 효과가 없다 |
| `requires` | `vector` | pgvector가 **먼저** 설치되어 있어야 한다 |

### pgvector 의존성

`requires = 'vector'` 때문에 pgvector 확장이 없으면 설치가 실패한다.
저장소의 모든 설치 명령이 예외 없이 `CASCADE`를 붙이는 이유가 이것이다:

- `start.sh:50` — `CREATE EXTENSION ontological CASCADE`
- `README.md:178` — `CREATE EXTENSION ontological CASCADE`
- `tests/run.sh:19` — `CREATE EXTENSION ontological CASCADE`
- `tests/typeql/run.py:110` 부근 — `CREATE EXTENSION ontological CASCADE`

pgvector가 실제로 쓰이는 곳은 벡터 검색 경로다 —
`og_add_embedding`이 `vector(N)` 컬럼과 HNSW 인덱스를 만든다
(`engine/src/vector/mod.rs:33`, 오류 문자열 `"failed to build HNSW index: {e}"`).

### 지원 PostgreSQL 버전 — 선언과 검증의 구분

`engine/Cargo.toml:9-19`가 선언하는 feature:

```toml
[features]
default = ["pg16"]
pg13 = ["pgrx/pg13", "pgrx-tests/pg13" ]
pg14 = ["pgrx/pg14", "pgrx-tests/pg14" ]
pg15 = ["pgrx/pg15", "pgrx-tests/pg15" ]
pg16 = ["pgrx/pg16", "pgrx-tests/pg16" ]
pg17 = ["pgrx/pg17", "pgrx-tests/pg17" ]
pg18 = ["pgrx/pg18", "pgrx-tests/pg18" ]
pg19 = ["pgrx/pg19", "pgrx-tests/pg19" ]
```

| | 상태 | 근거 |
|---|---|---|
| **선언된 버전** | PostgreSQL 13 ~ 19 | `engine/Cargo.toml:11-17` |
| **기본 빌드 대상** | PostgreSQL 16 | `engine/Cargo.toml:10` |
| **개발 이미지가 초기화하는 버전** | PostgreSQL 16 **뿐** | `docker/Dockerfile.dev:23` — `cargo pgrx init --pg16 …` |
| **`start.sh`가 기동하는 버전** | PostgreSQL 16 **뿐** | `start.sh:37` — `cargo pgrx start pg16` |
| **벤치마크가 실행된 버전** | PostgreSQL 16.14 (Debian) | `bench/results/bench-50000-20260817T033001Z.json` 의 `environment.postgres` |
| **CI에서 교차 검증되는 버전** | **없음** | `.github/` 디렉터리가 존재하지 않음 |

> **결정(Decision)**: 이 문서는 **PostgreSQL 16만 검증된 대상**으로 취급한다.
> pg13·pg14·pg15·pg17·pg18·pg19는 "빌드 feature가 존재한다"는 사실만 참이며,
> 그 위에서 회귀 스위트가 통과한다는 근거는 저장소에 없다.
> → [10_improvements_ops.md](10_improvements_ops.md) `OPS-03`

### 확장이 만드는 것

`CREATE EXTENSION ontological`은 두 개의 SQL 파일을 순서대로 실행한다
(`engine/src/lib.rs:23-24`):

```rust
extension_sql_file!("../sql/bootstrap.sql", name = "bootstrap", bootstrap);
extension_sql_file!("../sql/access.sql", name = "access", finalize);
```

결과물:

- 스키마 2개 — `og_catalog`, `og_data` (`engine/sql/bootstrap.sql:13-14`)
- 카탈로그·저장 테이블 (`og_catalog.graph`, `og_catalog.type`, `og_data.og_node`,
  `og_data.og_edge`, `og_data.og_adj`, …)
- 인라인 가능한 `LANGUAGE sql` 접근 함수와 뷰 (`engine/sql/access.sql`)
- Rust `#[pg_extern]` 함수 전체

타입별 저장 테이블(`og_data.n_<type_id>`, `og_data.e_<type_id>`)은
확장 스크립트가 아니라 **런타임에** `og_create_type` 등이 만든다
(`engine/src/catalog/types.rs:69,73`). 이 구분은 백업에서 중요하다 —
[08_backup_and_restore.md](08_backup_and_restore.md) 참조.

---

## 경로 A — Docker (저장소가 실제로 쓰는 경로)

### A-1. 이미지 빌드

`docker/Dockerfile.dev` 전체가 하는 일 (`docker/Dockerfile.dev:1-25`):

1. `FROM pgvector/pgvector:pg16` — PostgreSQL 16 + pgvector가 이미 들어 있는 베이스
2. 빌드 도구 설치: `build-essential postgresql-server-dev-16 curl ca-certificates git
   pkg-config libssl-dev libclang-dev clang libreadline-dev zlib1g-dev flex bison jq procps sudo`
3. **비루트 사용자 `dev` 생성** — 주석 그대로 "pgrx refuses to run postgres as root".
   `dev`에게 NOPASSWD sudo를 주고, `/usr/share/postgresql/16/extension` 과
   `/usr/lib/postgresql/16/lib` 의 소유권을 넘긴다 (확장 설치 대상 디렉터리)
4. rustup stable(minimal 프로파일) 설치
5. `cargo install cargo-pgrx --locked` — **버전 미지정**
6. `cargo pgrx init --pg16 /usr/lib/postgresql/16/bin/pg_config`
7. `WORKDIR /work`

```bash
docker build -f docker/Dockerfile.dev -t ontological-dev .
```

> `start.sh:28`은 이미지 태그를 `ontological-dev:latest`로 참조한다.
> 위 명령의 `-t ontological-dev`는 `:latest`와 동일하므로 그대로 맞는다.

> **주의**: 5단계의 `cargo install cargo-pgrx --locked`에는 버전 핀이 없는데,
> `engine/Cargo.toml:22`는 `pgrx = "=0.19.2"`로 **정확히 고정**되어 있다.
> cargo-pgrx가 0.19.2보다 앞서 나가면 이미지 재빌드 시점에 따라 빌드가 깨진다.
> → `OPS-04`

### A-2. 컨테이너 기동 + 확장 설치 (README 경로)

`README.md:169-180` 에 있는, 저장소가 문서화한 유일한 완전 설치 절차:

```bash
docker build -f docker/Dockerfile.dev -t ontological-dev .
docker run -d --name og -v "$PWD":/work -p 28816:28816 -w /work ontological-dev sleep infinity

docker exec og bash -lc 'cd /work/engine && \
  cargo pgrx install --features pg16 --no-default-features \
    --pg-config /usr/lib/postgresql/16/bin/pg_config --sudo && \
  cargo pgrx start pg16 && \
  createdb -h localhost -p 28816 og && \
  psql -h localhost -p 28816 -d og -c "CREATE EXTENSION ontological CASCADE" && \
  psql -h localhost -p 28816 -d og -f /work/examples/demo.sql'
```

각 인자의 의미:

| 인자 | 의미 |
|---|---|
| `cargo pgrx install` | cdylib를 빌드해 `$libdir`와 확장 디렉터리에 설치한다 |
| `--features pg16 --no-default-features` | 기본 feature와 중복 활성화를 피한다 |
| `--pg-config /usr/lib/postgresql/16/bin/pg_config` | 이미지의 apt 설치본 PostgreSQL을 대상으로 지정 |
| `--sudo` | 확장 디렉터리 쓰기에 sudo 사용. Dockerfile이 `dev`에게 NOPASSWD sudo를 준 이유 (`docker/Dockerfile.dev:13`) |
| `cargo pgrx start pg16` | pgrx가 관리하는 PostgreSQL 16 인스턴스를 기동. **포트 28816** |

> **포트 28816은 pgrx의 pg16 기본 포트다.** 저장소 전체가 이 값을 기본으로 쓴다 —
> `start.sh:8`, `tests/run.sh:7`, `bench/harness.py:47`,
> `portal/server/index.js:23`, `tests/typeql/run.py:26`.

### A-3. `start.sh`는 설치를 하지 않는다 — 반드시 알아야 할 점

`start.sh`에는 `cargo pgrx install`이 **없다**. 전체 90줄 중 빌드 단계는
Bolt 게이트웨이의 `cargo build --release` 하나뿐이다 (`start.sh:60`).

`start.sh:45-52`는 데이터베이스가 없을 때만 아래를 실행하는데, 표준 출력·오류를 모두 버린다:

```bash
docker exec "$CONTAINER" bash -lc "
    createdb -h localhost -p $PGPORT $DB
    psql -h localhost -p $PGPORT -d $DB -q -c 'CREATE EXTENSION ontological CASCADE'
    psql -h localhost -p $PGPORT -d $DB -q -f /work/examples/demo.sql" >/dev/null 2>&1
```

즉 **확장이 설치되지 않은 컨테이너에서 `start.sh`를 처음 실행하면**
`createdb`는 성공하고 `CREATE EXTENSION`은 조용히 실패한다. 데이터베이스는 생겼으므로
다음 실행부터는 `start.sh:45`의 조건이 거짓이 되어 이 블록 자체를 건너뛴다 —
결과적으로 빈 데이터베이스에 계속 붙게 된다.

> **필수 규칙**: `start.sh`를 실행하기 전에 **반드시** A-2의 `cargo pgrx install`을
> 최소 한 번 수행할 것. → `OPS-01`

설치 여부는 이렇게 확인한다:

```bash
docker exec ontological-dev bash -lc \
  "psql -h localhost -p 28816 -d postgres -tAc \
   \"SELECT name, default_version FROM pg_available_extensions WHERE name IN ('ontological','vector')\""
```

기대 출력:

```
ontological|0.1.0
vector|<pgvector 버전>
```

---

## 경로 B — 로컬 빌드 (Docker 없이)

> 저장소에 로컬 설치 스크립트는 없다. 아래는 `docker/Dockerfile.dev:1-25`가 컨테이너 안에서
> 하는 일을 호스트에서 그대로 재현한 것이며, **저장소 안에서 실행 검증된 절차는 아니다.**
> 검증된 경로가 필요하면 경로 A를 쓸 것.

### B-1. 사전 요구

| 요구 | 근거 |
|---|---|
| PostgreSQL 16 + 서버 개발 헤더 (`postgresql-server-dev-16`) | `docker/Dockerfile.dev:6` |
| pgvector (확장 `vector`) | `engine/ontological.control:7` |
| Rust stable 툴체인 | `docker/Dockerfile.dev:21` |
| `clang` / `libclang-dev` (bindgen용), `libssl-dev`, `pkg-config` | `docker/Dockerfile.dev:7-9` |
| 비루트 사용자 (pgrx가 root로 postgres를 띄우지 않음) | `docker/Dockerfile.dev:12` |

### B-2. 절차

```bash
# 1. cargo-pgrx. Dockerfile은 --locked 만 쓰지만, engine/Cargo.toml 이 pgrx를
#    "=0.19.2" 로 고정하므로 같은 버전을 명시하는 편이 안전하다.
cargo install cargo-pgrx --locked --version 0.19.2

# 2. pgrx 초기화 — 시스템 PostgreSQL 16 을 대상으로
cargo pgrx init --pg16 $(which pg_config)

# 3. 확장 빌드 및 설치
cd engine
cargo pgrx install --features pg16 --no-default-features \
  --pg-config $(which pg_config) --sudo

# 4. pgrx 관리 인스턴스 기동 (포트 28816)
cargo pgrx start pg16

# 5. 데이터베이스 생성과 확장 설치
createdb -h localhost -p 28816 og
psql -h localhost -p 28816 -d og -c 'CREATE EXTENSION ontological CASCADE'
psql -h localhost -p 28816 -d og -f ../examples/demo.sql
```

> `--version 0.19.2`는 `engine/Cargo.toml:22`의 핀에서 도출한 값이다.
> `docker/Dockerfile.dev:22`에는 이 인자가 없다 — 그 차이가 `OPS-04`의 내용이다.

### B-3. macOS 링크 설정

`engine/.cargo/config.toml`은 macOS에서만 적용되는 링커 플래그를 담고 있다
(`engine/.cargo/config.toml:1-3`):

```toml
[target.'cfg(target_os="macos")']
# Postgres symbols won't be available until runtime
rustflags = ["-Clink-arg=-Wl,-undefined,dynamic_lookup"]
```

리눅스에는 추가 설정이 없다.

### B-4. 빌드 프로파일

`engine/Cargo.toml:39-46`:

```toml
[profile.dev]
panic = "unwind"

[profile.release]
panic = "unwind"
opt-level = 3
lto = "fat"
codegen-units = 1
```

`lto = "fat"` + `codegen-units = 1`은 릴리스 빌드를 상당히 느리게 만든다.
반복 개발에는 `cargo pgrx install`의 기본(debug) 빌드를 쓰고,
성능 측정·배포에는 `--release`를 쓴다.

`panic = "unwind"`는 pgrx가 Rust 패닉을 PostgreSQL의 `ereport(ERROR)`로 변환하기 위해
필요하다 — `engine/src/id.rs:31`의 주석이 이 동작에 의존한다고 명시한다.

---

## 설치 검증

```bash
# 확장 버전
psql -h localhost -p 28816 -d og -tAc "SELECT ontological_version()"
# → 0.1.0   (engine/src/lib.rs:41-43, CARGO_PKG_VERSION 을 그대로 반환)

# 스키마와 테이블
psql -h localhost -p 28816 -d og -c "\dn og_catalog og_data"

# 그래프 목록
psql -h localhost -p 28816 -d og -c "SELECT graph_id, name, created_at FROM og_catalog.graph ORDER BY name"

# 데모 그래프가 살아 있는지 (Cypher 경로)
psql -h localhost -p 28816 -d og -tAc \
  "SELECT og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')"

# 구조 무결성 — 빈 결과가 정상
psql -h localhost -p 28816 -d og -c "SELECT * FROM og_check_integrity()"
```

---

## 금지 / 필수

### 금지 (Forbidden)

- `ALTER EXTENSION ontological UPDATE` — **업그레이드 스크립트가 존재하지 않는다.**
  `engine/` 아래에 `ontological--0.1.0--*.sql` 형식의 파일이 하나도 없으며,
  `engine/sql/`에는 `bootstrap.sql`과 `access.sql`만 있다. `OPS-02`
- pgvector 없이 `CREATE EXTENSION ontological` (CASCADE 생략) — `requires = 'vector'` 위반으로 실패한다.
- root로 `cargo pgrx start` — pgrx가 거부한다 (`docker/Dockerfile.dev:12` 주석).
- 확장 스키마 이동 (`SET SCHEMA`) — `relocatable = false`.

### 필수 (Required)

- `start.sh` 실행 전에 `cargo pgrx install`을 최소 한 번 수행할 것 (A-3).
- pg16 이외의 버전을 시도할 때는 그것이 **미검증 경로**임을 인지할 것.
- 프로덕션 이미지가 필요하면 직접 정의할 것 — 저장소에는 dev 이미지만 있다 (`OPS-05`).

---

<!-- affects: ops, backend -->
<!-- requires-update: docs/08_operations/02_startup_and_teardown.md -->
