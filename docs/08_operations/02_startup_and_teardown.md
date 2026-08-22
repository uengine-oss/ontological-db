# 기동과 정지

> **이 문서가 답하는 질문**
> - `./start.sh` 한 줄이 실제로 무엇을 실행하는가?
> - 각 단계를 손으로 실행하려면 어떤 명령을 쓰는가?
> - 어떤 환경변수가 있고, 기본값과 의미는 무엇인가?
> - 무엇이 컨테이너 안에서 돌고 무엇이 호스트에서 도는가?
> - 안전하게 멈추려면?

---

## 사실 (Facts) — 프로세스 배치

`start.sh`가 만드는 최종 상태는 **컨테이너 2계층 + 호스트 1계층**이다.
이 구분을 놓치면 로그를 엉뚱한 곳에서 찾게 된다.

| 프로세스 | 어디서 도는가 | 리스닝 | 로그 |
|---|---|---|---|
| PostgreSQL 16 (pgrx 관리) | **컨테이너 안** | `28816` (컨테이너 → 호스트 동일 포트로 publish, `start.sh:27`) | pgrx 데이터 디렉터리 내 로그 — 위치는 **미확인** (저장소에 명시 없음) |
| `ontological-bolt` | **컨테이너 안** | 컨테이너 `7687` → 호스트 `28687` (`start.sh:27`) | **컨테이너 안**의 `/tmp/ontological-bolt.log` (`start.sh:63`) |
| Ontological Studio (Node.js) | **호스트** | `7474` (`start.sh:7`) | **호스트**의 `/tmp/ontological-studio.log` (`start.sh:77`) |

Studio는 호스트에서 `127.0.0.1:28816`으로 붙는다 (`start.sh:76`) — 즉 컨테이너가 publish한
포트를 거쳐서 들어간다. 컨테이너 내부 네트워크를 쓰지 않는다.

---

## 환경변수 전체 표

### `start.sh`가 읽는 변수

| 이름 | 기본값 | 의미 | 근거 |
|---|---|---|---|
| `OG_CONTAINER` | `ontological-dev` | 사용/생성할 Docker 컨테이너 이름 | `start.sh:6` |
| `OG_PORT` | `7474` | Studio가 호스트에서 리스닝할 HTTP 포트 | `start.sh:7` |
| `OG_PGPORT` | `28816` | PostgreSQL 포트. 컨테이너와 호스트 양쪽에 같은 번호로 매핑됨 | `start.sh:8`, `start.sh:27` |
| `OG_BOLTPORT` | `28687` | Bolt 게이트웨이의 **호스트** 포트. 컨테이너 안에서는 항상 `7687` | `start.sh:9`, `start.sh:27` |
| `OG_DB` | `og` | 데이터베이스 이름 | `start.sh:10` |
| `OG_BOLT` | `1` | `1`이 아니면 Bolt 게이트웨이 기동을 통째로 건너뛴다 | `start.sh:56` |

### `start.sh`가 자식 프로세스에 **설정하는** 변수

| 프로세스 | 변수 | 값 | 근거 |
|---|---|---|---|
| `ontological-bolt` | `OG_BOLT_PGPORT` | `$OG_PGPORT` (=28816) | `start.sh:62` |
| | `OG_BOLT_PGDATABASE` | `$OG_DB` (=og) | `start.sh:62` |
| | `OG_BOLT_ADVERTISED` | `localhost:$OG_BOLTPORT` | `start.sh:62` |
| Studio | `PGHOST` | `127.0.0.1` | `start.sh:76` |
| | `PGPORT` | `$OG_PGPORT` | `start.sh:76` |
| | `PGDATABASE` | `$OG_DB` | `start.sh:76` |
| | `PGUSER` | `dev` | `start.sh:76` |
| | `PORT` | `$OG_PORT` | `start.sh:76` |

`start.sh`가 설정하지 **않는** Bolt 변수는 각자의 기본값을 쓴다
(`bolt/src/main.rs:37-44`): `OG_BOLT_LISTEN=0.0.0.0:7687`, `OG_BOLT_PGHOST=localhost`,
`OG_BOLT_GRAPH=default`.

Studio의 `PGPASSWORD`와 `OG_BENCH_DIR`도 설정하지 않는다
(`portal/server/index.js:19,26`) — 각각 `undefined`와
`portal/server/../../bench/results` 가 된다.

전체 변수 목록은 [03_configuration.md](03_configuration.md)에 정리되어 있다.

---

## `start.sh` 단계별 해부

`start.sh:3`은 `set -euo pipefail`로 시작한다 — 어떤 단계든 실패하면 즉시 중단된다.
단, 아래에서 짚는 몇몇 명령은 `|| true` 또는 `2>&1 >/dev/null`로 그 보호를 스스로 해제한다.

### 단계 0 — 변수 결정 (`start.sh:5-12`)

```bash
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTAINER="${OG_CONTAINER:-ontological-dev}"
PORT="${OG_PORT:-7474}"
PGPORT="${OG_PGPORT:-28816}"
BOLTPORT="${OG_BOLTPORT:-28687}"
DB="${OG_DB:-og}"
```

### 단계 1 — 컨테이너 (`start.sh:14-31`)

로직: **실행 중인가?** → 아니면 **존재하는가?** → 아니면 **만든다**.

```bash
docker ps --format '{{.Names}}' | grep -qx "$CONTAINER"      # 실행 중?
docker ps -a --format '{{.Names}}' | grep -qx "$CONTAINER"   # 존재?
docker start "$CONTAINER"                                    # 있으면 시작
```

새로 만드는 경우 (`start.sh:20-29`) — 수동 등가 명령:

```bash
docker volume create ontological-target
docker volume create ontological-cargo

docker run -d --name ontological-dev \
    -v "$PWD":/work \
    -v ontological-target:/work/engine/target \
    -v ontological-cargo:/home/dev/.cargo/registry \
    -p 28816:28816 -p 28687:7687 \
    -w /work ontological-dev:latest sleep infinity

docker exec ontological-dev bash -lc \
    'sudo chown -R dev /work/engine/target /home/dev/.cargo/registry'
```

볼륨 2개의 역할:

| 볼륨 | 마운트 지점 | 이유 |
|---|---|---|
| `ontological-target` | `/work/engine/target` | 빌드 산출물을 호스트 바인드 마운트 밖으로 뺀다. 재빌드 비용과 파일시스템 성능 문제를 동시에 피한다 |
| `ontological-cargo` | `/home/dev/.cargo/registry` | crates.io 레지스트리 캐시를 컨테이너 재생성 사이에 보존한다 |

두 볼륨은 root 소유로 만들어지므로 `chown -R dev`가 필요하다 (`start.sh:29`).

> **중요**: PostgreSQL 데이터 디렉터리를 위한 볼륨은 **없다**.
> 데이터는 컨테이너의 쓰기 가능 레이어에 있는 pgrx 데이터 디렉터리에 남는다.
> `docker rm`으로 컨테이너를 지우면 **그래프가 함께 사라진다.**
> → [08_backup_and_restore.md](08_backup_and_restore.md), `OPS-06`

### 단계 2 — PostgreSQL (`start.sh:33-42`)

```bash
if ! docker exec "$CONTAINER" bash -lc "pg_isready -h localhost -p $PGPORT -q"; then
    docker exec "$CONTAINER" bash -lc "cd /work/engine && cargo pgrx start pg16" >/dev/null 2>&1 || true
    for _ in $(seq 1 30); do
        docker exec "$CONTAINER" bash -lc "pg_isready -h localhost -p $PGPORT -q" && break
        sleep 1
    done
fi
```

- `cd /work/engine`이 필요한 이유는 주석에 있다 (`start.sh:36`):
  "cargo pgrx needs the crate's directory — /work has no Cargo.toml."
- `|| true` + 출력 버림 때문에 **기동 실패가 표면에 드러나지 않는다.**
  30초 폴링이 모두 실패해도 스크립트는 다음 단계로 넘어간다.

수동 등가 명령 (오류를 볼 수 있는 형태):

```bash
docker exec ontological-dev bash -lc 'cd /work/engine && cargo pgrx start pg16'
docker exec ontological-dev bash -lc 'pg_isready -h localhost -p 28816'
```

### 단계 3 — 데이터베이스 + 확장 + 데모 (`start.sh:44-52`)

```bash
if ! docker exec "$CONTAINER" bash -lc \
     "psql -h localhost -p $PGPORT -lqt | cut -d'|' -f1 | grep -qw $DB"; then
    docker exec "$CONTAINER" bash -lc "
        createdb -h localhost -p $PGPORT $DB
        psql -h localhost -p $PGPORT -d $DB -q -c 'CREATE EXTENSION ontological CASCADE'
        psql -h localhost -p $PGPORT -d $DB -q -f /work/examples/demo.sql" >/dev/null 2>&1
fi
```

**조건은 "데이터베이스가 존재하는가" 하나뿐이다.** 확장 설치 여부도, 데모 데이터의
존재 여부도 확인하지 않는다. 이 블록 전체의 출력이 버려지므로 세 명령 중 무엇이 실패해도
조용하다. [01_install.md](01_install.md) A-3의 경고가 여기서 나온다.

수동 등가 명령:

```bash
docker exec ontological-dev bash -lc 'createdb -h localhost -p 28816 og'
docker exec ontological-dev bash -lc \
  'psql -h localhost -p 28816 -d og -c "CREATE EXTENSION ontological CASCADE"'
docker exec ontological-dev bash -lc \
  'psql -h localhost -p 28816 -d og -f /work/examples/demo.sql'
```

### 단계 4 — Bolt 게이트웨이 (`start.sh:54-65`)

`start.sh:55`의 주석이 설계 의도를 명시한다:
"Optional by design: nothing on the PostgreSQL path depends on it running."

```bash
if [ "${OG_BOLT:-1}" = "1" ]; then
    if ! docker exec "$CONTAINER" bash -lc "pgrep -f ontological-bolt >/dev/null"; then
        docker exec "$CONTAINER" bash -lc "cd /work/bolt && cargo build --release -q" >/dev/null 2>&1 || true
        docker exec -d "$CONTAINER" bash -lc \
            "OG_BOLT_PGPORT=$PGPORT OG_BOLT_PGDATABASE=$DB OG_BOLT_ADVERTISED=localhost:$BOLTPORT \
             /work/bolt/target/release/ontological-bolt > /tmp/ontological-bolt.log 2>&1"
    fi
fi
```

- 존재 판정은 `pgrep -f ontological-bolt` — **프로세스 이름 매칭**이다.
- 빌드 실패는 `|| true`로 무시되고, 그다음 `docker exec -d`가 존재하지 않는 바이너리를
  실행하려다 로그 파일에 오류만 남긴 채 조용히 끝난다.
- `docker exec -d`로 띄우므로 **프로세스 감독자가 없다.** 죽으면 다시 뜨지 않는다. `OPS-07`

Bolt를 끄고 기동하려면:

```bash
OG_BOLT=0 ./start.sh
```

수동 등가 명령:

```bash
docker exec ontological-dev bash -lc 'cd /work/bolt && cargo build --release'
docker exec -d ontological-dev bash -lc \
  'OG_BOLT_PGPORT=28816 OG_BOLT_PGDATABASE=og OG_BOLT_ADVERTISED=localhost:28687 \
   /work/bolt/target/release/ontological-bolt > /tmp/ontological-bolt.log 2>&1'
```

### 단계 5 — Studio (`start.sh:67-77`)

```bash
if [ ! -d "$ROOT/portal/node_modules" ]; then
    (cd "$ROOT/portal" && npm install --no-audit --no-fund >/dev/null)
fi

pkill -f "portal/server/index.js" 2>/dev/null || true
sleep 1
PGHOST=127.0.0.1 PGPORT="$PGPORT" PGDATABASE="$DB" PGUSER=dev PORT="$PORT" \
    nohup node "$ROOT/portal/server/index.js" > /tmp/ontological-studio.log 2>&1 &
```

- Studio는 **호스트에서** 실행된다. 호스트에 Node.js가 필요하다.
- 기존 Studio를 `pkill`로 죽인다 — `SIGTERM`이다. `portal/server/index.js:374`는
  `SIGINT` 핸들러만 등록하므로 커넥션 풀은 정리되지 않고 프로세스가 끝난다. `OPS-09`
- 의존성은 `pg` 하나뿐이다 (`portal/package.json`).

수동 등가 명령:

```bash
cd portal && npm install
PGHOST=127.0.0.1 PGPORT=28816 PGDATABASE=og PGUSER=dev PORT=7474 npm start
```

(`npm start`는 `node server/index.js` — `portal/package.json` scripts)

### 단계 6 — 헬스 폴링 (`start.sh:79-90`)

```bash
for _ in $(seq 1 20); do
    if curl -sf -m 1 "http://localhost:$PORT/api/health" >/dev/null 2>&1; then
        ... exit 0
    fi
    sleep 0.5
done
printf '✗ studio did not come up — see /tmp/ontological-studio.log\n'
tail -20 /tmp/ontological-studio.log
exit 1
```

최대 10초(20 × 0.5s) 기다린다. 성공하면 다음을 출력한다:

```
✓ Ontological Studio  http://localhost:7474
  postgres  localhost:28816/og
  log       /tmp/ontological-studio.log
```

`/api/health`가 무엇을 검사하는지는 `portal/server/index.js:151-165`:
`ontological_version()`, `current_database()`, `version()`, 그리고
`og_catalog.graph` 전체 목록. 확장이 없으면 503과 함께 PostgreSQL 오류가 그대로 나온다.

> **이 헬스체크는 Studio의 것이지 데이터베이스의 것이 아니다.**
> 확장 자체에는 헬스체크 함수가 없다. `OPS-08`

---

## 기동 확인 체크리스트

```bash
# 1. 컨테이너
docker ps --filter name=ontological-dev

# 2. PostgreSQL
docker exec ontological-dev bash -lc 'pg_isready -h localhost -p 28816'

# 3. 확장
psql -h localhost -p 28816 -d og -tAc "SELECT ontological_version()"

# 4. 데이터
psql -h localhost -p 28816 -d og -tAc \
  "SELECT og_cypher('default','MATCH (w:Work) RETURN count(w) AS n')"

# 5. Studio
curl -s http://localhost:7474/api/health | head -c 400

# 6. Bolt — TCP 포트가 열려 있는지
docker exec ontological-dev bash -lc 'pgrep -af ontological-bolt'
docker exec ontological-dev bash -lc 'tail -5 /tmp/ontological-bolt.log'
```

Bolt 프로토콜 수준의 확인은 `tests/neo4j-movies/run.py`가 raw 핸드셰이크로 수행한다
(`tests/neo4j-movies/run.py:47-48`의 `BOLT_MAGIC` / `BOLT_PROPOSALS`).
자세한 것은 [04_testing_and_ci.md](04_testing_and_ci.md).

---

## 정지 (Teardown)

저장소에는 `stop.sh`가 **없다.** 아래는 `start.sh`가 만든 것을 역순으로 되돌리는 명령이며,
각 명령은 기동 단계와 1:1로 대응한다.

### 안전한 정지 (데이터 보존)

```bash
# 5. Studio (호스트)
pkill -f "portal/server/index.js"

# 4. Bolt (컨테이너 안)
docker exec ontological-dev bash -lc 'pkill -f ontological-bolt'

# 2. PostgreSQL — pgrx 가 관리하므로 pgrx 로 멈춘다
docker exec ontological-dev bash -lc 'cd /work/engine && cargo pgrx stop pg16'

# 1. 컨테이너 정지 (삭제 아님 — 데이터 유지)
docker stop ontological-dev
```

`docker start ontological-dev` 후 `./start.sh`를 다시 실행하면 같은 데이터로 돌아온다
(`start.sh:16-18`의 "존재하면 start" 경로).

### 완전 삭제 (데이터 소실)

> **경고**: PostgreSQL 데이터 디렉터리는 볼륨이 아니라 컨테이너 레이어에 있다
> (단계 1 참조). 아래 명령은 **그래프를 영구 삭제한다.**
> 먼저 [08_backup_and_restore.md](08_backup_and_restore.md)의 `pg_dump`를 수행할 것.

```bash
docker rm -f ontological-dev

# 빌드 캐시까지 지우려면 (데이터와는 무관)
docker volume rm ontological-target ontological-cargo
```

### 데이터만 초기화

컨테이너는 유지한 채 그래프만 비우려면 데이터베이스를 다시 만든다:

```bash
docker exec ontological-dev bash -lc \
  'psql -h localhost -p 28816 -d postgres -c "DROP DATABASE IF EXISTS og"'
docker exec ontological-dev bash -lc 'createdb -h localhost -p 28816 og'
docker exec ontological-dev bash -lc \
  'psql -h localhost -p 28816 -d og -c "CREATE EXTENSION ontological CASCADE"'
docker exec ontological-dev bash -lc \
  'psql -h localhost -p 28816 -d og -f /work/examples/demo.sql'
```

이것이 `tests/run.sh:17-19`가 매 테스트 파일마다 하는 일과 같은 절차다.

---

## 금지 / 필수

### 금지 (Forbidden)

- `start.sh`의 출력만 보고 확장 설치 성공을 판단하지 말 것 — 단계 3은 오류를 버린다.
- `docker rm` 전에 백업 없이 진행하지 말 것 — PGDATA가 볼륨에 없다.
- Studio를 `kill -9`로 죽이지 말 것 — 커넥션 풀 정리 기회가 사라진다.
  (`SIGTERM`도 현재는 핸들러가 없다 — `OPS-09`)
- Bolt 게이트웨이가 떠 있다는 사실을 PostgreSQL 경로의 헬스 지표로 쓰지 말 것.
  둘은 독립이다 (`start.sh:55`, `bolt/README.md:35-36`).

### 필수 (Required)

- 재기동 후에는 항상 위의 **기동 확인 체크리스트 6항목**을 돌릴 것.
- `OG_PGPORT`를 바꾸면 Studio·Bolt·테스트·벤치마크의 기본값(28816)이 모두 어긋난다.
  바꾼다면 `PGPORT`(테스트/벤치)와 Studio 환경변수를 함께 맞출 것.
- 백엔드-로컬 CSR(`og_csr_build`)은 **연결 단위**로만 존재한다. 재기동은 물론
  커넥션 하나만 끊겨도 사라진다 — [07_maintenance.md](07_maintenance.md) 참조.

---

<!-- affects: ops, backend, frontend -->
<!-- requires-update: docs/08_operations/03_configuration.md, docs/08_operations/09_troubleshooting.md -->
