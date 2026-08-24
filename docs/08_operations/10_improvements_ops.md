# 운영 개선 포인트

> **이 문서가 답하는 질문**
> - 이 시스템을 운영하려 할 때 지금 당장 걸리는 것은 무엇인가?
> - 각 항목의 근거는 어느 파일 어느 줄인가?
> - 무엇부터 손대야 하는가?

---

## 이 문서의 규칙

- **모든 항목은 실제 파일을 읽고 확인한 것이다.** 일반론은 넣지 않았다.
- "없다"는 주장은 저장소 전체 검색으로 확인한 것이며, 검색 방법을 함께 적었다.
- 심각도는 **운영 관점**이다 — 제품 기능의 완성도가 아니라
  "실 운영에서 사고가 나는가 / 진단할 수 있는가"로 매겼다.
- 제안은 **제안이다.** 이 문서는 코드를 바꾸지 않는다.

---

## 요약표

| ID | 제목 | 심각도 | 근거 |
|---|---|---|---|
| [OPS-01](#ops-01) | `start.sh`에 확장 설치 단계가 없고 오류를 전부 버린다 | **High** | `start.sh:37,45-52,60` |
| [OPS-02](#ops-02) | 확장 업그레이드 스크립트 부재 — 버전 이행 경로가 없다 | **High** | `engine/sql/` 디렉터리, `engine/ontological.control:2` |
| [OPS-03](#ops-03) | CI 설정 부재 + pg_regress 스캐폴딩이 잘못된 확장 이름을 참조 | **High** | `.github` 부재, `engine/tests/pg_regress/sql/setup.sql:3` |
| [OPS-04](#ops-04) | Dockerfile의 `cargo-pgrx` 버전 미고정 | **Med** | `docker/Dockerfile.dev:22` vs `engine/Cargo.toml:22` |
| [OPS-05](#ops-05) | 프로덕션 이미지 정의 부재 (dev 이미지만 존재) | **Med** | `docker/` 디렉터리 |
| [OPS-06](#ops-06) | PGDATA 볼륨 부재 — `docker rm` 한 번이 데이터 소실 | **High** | `start.sh:21-28` |
| [OPS-07](#ops-07) | Bolt 게이트웨이에 프로세스 감독·그레이스풀 셧다운이 없다 | **Med** | `start.sh:61-63`, `bolt/src/main.rs:36-80` |
| [OPS-08](#ops-08) | 확장 수준 헬스체크 함수 부재 | **Med** | `engine/src/` 전체, `portal/server/index.js:151` |
| [OPS-09](#ops-09) | Studio에 `SIGTERM` 핸들러가 없다 | **Low** | `portal/server/index.js:374`, `start.sh:73` |
| [OPS-10](#ops-10) | 씨앗 설정 키 4종을 읽는 코드가 없다 | **Med** | `engine/sql/bootstrap.sql:256-260` |
| [OPS-11](#ops-11) | `og.max_rows` 가드레일이 설정만 되고 읽히지 않는다 | **Med** | `engine/src/agent/mod.rs:437-439` |
| [OPS-12](#ops-12) | Studio 커넥션 풀 `max: 8` 하드코딩 + 질의 타임아웃 없음 | **Med** | `portal/server/index.js:21-29,296-308` |
| [OPS-13](#ops-13) | 회귀·벤치 게이트가 전부 수동 실행 | **High** | `.github` 부재, `bench/harness.py:1221` |
| [OPS-14](#ops-14) | `tests/run.sh`의 무결성 결과가 종료 코드에 반영되지 않는다 | **Med** | `tests/run.sh:71-75` |
| [OPS-15](#ops-15) | 게시된 벤치마크가 참조하는 결과 파일 1개가 저장소에 없다 | **Low** | `docs/benchmark.md:399` vs `bench/results/` |
| [OPS-16](#ops-16) | 메트릭 노출(Prometheus 등) 부재 | **Med** | 저장소 전체 검색 |
| [OPS-17](#ops-17) | 백업 시 확장 버전 고정 전략 부재 | **Med** | `engine/ontological.control:2`, `engine/src/lib.rs:41-43` |
| [OPS-18](#ops-18) | 확장이 PostgreSQL 로그에 수명주기 이벤트를 남기지 않는다 | **Med** | `engine/src/catalog/types.rs:96,168` |

**우선순위 제안**: OPS-01 → OPS-06 → OPS-02 → OPS-13/OPS-03 → OPS-16.

---

## 상세

### OPS-01
**`start.sh`에 확장 설치 단계가 없고 오류를 전부 버린다**

| | |
|---|---|
| **심각도** | **High** |
| **근거** | `start.sh:37` (`\|\| true` + `>/dev/null 2>&1`), `start.sh:45-52` (블록 전체 `>/dev/null 2>&1`), `start.sh:60` (`\|\| true`). `start.sh` 전체 90줄에 `cargo pgrx install`이 **없다** |
| **현상** | `start.sh:45`의 조건은 "데이터베이스가 존재하는가" 하나뿐이다. 확장이 설치되지 않은 컨테이너에서 처음 실행하면 `createdb`만 성공하고 `CREATE EXTENSION ontological CASCADE`와 `demo.sql` 적재가 조용히 실패한다. DB는 생겼으므로 **다음 실행부터는 이 블록 자체를 건너뛴다** — 빈 데이터베이스에 계속 붙게 되고, 스크립트는 `✓ Ontological Studio`를 출력하며 성공으로 끝난다. PostgreSQL 기동 실패(`:37`)와 Bolt 빌드 실패(`:60`)도 같은 방식으로 삼켜진다 |
| **제안** | ① `start.sh`에 확장 설치 여부 확인 단계를 추가한다 — `psql -tAc "SELECT 1 FROM pg_available_extensions WHERE name='ontological'"` 가 비면 `cargo pgrx install`을 수행하거나 명시적으로 실패시킨다. ② 조건을 "DB 존재"에서 "DB 존재 **그리고** 확장 존재"로 바꾼다. ③ `>/dev/null 2>&1`을 제거하거나 로그 파일로 리다이렉트하고, 실패 시 `set -e`가 동작하도록 `\|\| true`를 걷어낸다 |
| **예상 효과** | 최초 실행 실패가 즉시 드러난다. 신규 기여자·운영자가 "떴는데 비어 있다"를 진단하는 데 쓰는 시간이 사라진다 |
| **리스크** | 낮음. 다만 오류를 노출하기 시작하면 지금까지 조용히 지나가던 환경(예: pgvector 미설치)에서 `start.sh`가 실패로 끝나게 되므로, 그 실패 메시지가 조치 가능하도록 문구를 다듬어야 한다 |

---

### OPS-02
**확장 업그레이드 스크립트 부재 — 버전 이행 경로가 없다**

| | |
|---|---|
| **심각도** | **High** |
| **근거** | `engine/sql/`에는 `bootstrap.sql`과 `access.sql` 두 파일만 있다. `find . -name "ontological--*"` 결과가 **비어 있다**. `engine/ontological.control:2` — `default_version = '0.1.0'` |
| **현상** | `ALTER EXTENSION ontological UPDATE`로 갈 수 있는 버전이 존재하지 않는다. `bootstrap.sql`에 테이블이나 컬럼을 추가하는 변경을 하면, 기존 데이터베이스는 그 변경을 받을 방법이 없다. 게다가 `default_version`이 `0.1.0`으로 고정되어 있어 **어떤 스키마인지 버전 번호로 식별할 수도 없다** — `ontological_version()`은 `CARGO_PKG_VERSION`을 그대로 돌려주므로(`engine/src/lib.rs:41-43`) 항상 `0.1.0`이다 |
| **제안** | ① 스키마 변경을 동반하는 릴리스마다 `engine/sql/ontological--<from>--<to>.sql`을 작성하고 `default_version`을 올린다. ② 그 전까지는 **문서에 "업그레이드 불가, 재생성 + 데이터 이관만 가능"을 명시**한다(이 문서 묶음이 그렇게 하고 있다). ③ `og_catalog.schema_version` 테이블이 이미 존재하므로(`engine/sql/bootstrap.sql:177`), 물리 스키마 버전을 여기에 함께 기록해 런타임 검증 지점으로 쓴다 |
| **예상 효과** | 릴리스 간 이행이 가능해지고, 백업 파일이 어떤 스키마의 것인지 판별 가능해진다 |
| **리스크** | 업그레이드 스크립트는 한번 배포되면 되돌리기 어렵다. 각 스크립트에 대해 "구 스키마 → 신 스키마" 왕복 테스트를 회귀 스위트에 추가하는 비용이 함께 든다 |

---

### OPS-03
**CI 설정 부재 + pg_regress 스캐폴딩이 잘못된 확장 이름을 참조**

| | |
|---|---|
| **심각도** | **High** |
| **근거** | `.github/` 디렉터리가 **존재하지 않는다**(`ls -la .github` → No such file). `Makefile`·`.gitlab-ci.yml`·`Jenkinsfile` 도 없다. `engine/tests/pg_regress/`에는 `sql/setup.sql`과 `expected/setup.out` 두 파일뿐이고, `engine/tests/pg_regress/sql/setup.sql:3`은 `CREATE EXTENSION engine;` — **그런 확장은 없다**(이 확장의 이름은 `ontological`). `#[pg_test]` 어트리뷰트도 저장소에 하나도 없다 |
| **현상** | ① 자동 검증이 전혀 없다. ② pg_regress 경로는 pgrx가 디렉터리 이름(`engine/`)에서 생성한 템플릿 그대로 방치되어 있어, 실행하면 setup에서 실패한다. ③ `engine/Cargo.toml:11-17`이 pg13~pg19를 feature로 선언하지만 **pg16 외에는 아무 검증도 없다** — `docker/Dockerfile.dev:23`은 `--pg16`만 초기화하고 `start.sh:37`도 `pg16`만 기동한다 |
| **제안** | ① `setup.sql`을 `CREATE EXTENSION ontological CASCADE;`로 고치거나, 쓰지 않을 것이면 `engine/tests/pg_regress/`를 제거해 오해를 없앤다. ② 최소 CI 파이프라인을 추가한다 — `docker build` → `cargo pgrx install` → `cargo test` → `tests/run.sh` → `bench/harness.py --compare-baseline`. ③ 지원한다고 선언한 PostgreSQL 버전에 대해 매트릭스 잡을 돌리거나, **선언 feature를 검증된 버전으로 줄인다** |
| **예상 효과** | 회귀가 커밋 시점에 잡힌다. "지원한다"는 선언이 근거를 갖는다 |
| **리스크** | pg13~pg19 매트릭스는 빌드 시간이 길다(`lto = "fat"`, `codegen-units = 1`, `engine/Cargo.toml:44-46`). 우선 pg16 단일 잡부터 시작하는 편이 현실적이다 |

---

### OPS-04
**Dockerfile의 `cargo-pgrx` 버전 미고정**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `docker/Dockerfile.dev:22` — `RUN cargo install cargo-pgrx --locked` (버전 인자 없음). `engine/Cargo.toml:22` — `pgrx = "=0.19.2"` (정확 고정) |
| **현상** | 이미지를 재빌드하는 시점의 최신 `cargo-pgrx`가 설치된다. `cargo-pgrx`와 `pgrx` 라이브러리는 버전이 맞아야 하는데 한쪽만 고정되어 있으므로, 어느 날 재빌드하면 빌드나 `cargo pgrx install`이 깨진다. 이미지 캐시가 살아 있는 동안에는 재현되지 않아 원인 파악이 늦어진다 |
| **제안** | `docker/Dockerfile.dev:22`를 `RUN cargo install cargo-pgrx --locked --version 0.19.2`로 고정하고, `engine/Cargo.toml`의 pgrx 버전과 함께 올린다. 두 값이 어긋나지 않도록 릴리스 체크리스트에 넣는다 |
| **예상 효과** | 이미지 재빌드가 결정적(deterministic)이 된다 |
| **리스크** | 매우 낮음. pgrx 업그레이드 시 두 곳을 함께 고쳐야 한다는 규율이 추가될 뿐 |

---

### OPS-05
**프로덕션 이미지 정의 부재 (dev 이미지만 존재)**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `docker/` 디렉터리에 `Dockerfile.dev` **하나만** 있다. 그 이미지는 `build-essential`, `postgresql-server-dev-16`, `clang`, `flex`, `bison`, rustup 툴체인, cargo 레지스트리 캐시를 전부 담고 있고(`docker/Dockerfile.dev:5-23`), `dev` 사용자에게 **NOPASSWD sudo**를 부여한다(`docker/Dockerfile.dev:13`) |
| **현상** | 릴리스 배포 경로가 정의되어 있지 않다. 지금 이미지를 그대로 쓰면 빌드 툴체인 전체와 무제한 sudo가 운영 환경에 들어간다. 또한 `start.sh`가 컨테이너 안에서 `cargo build`를 수행하는 구조(`start.sh:60`)라 이미지와 소스 트리가 분리되어 있지 않다 |
| **제안** | 멀티스테이지 `docker/Dockerfile`을 추가한다 — 빌더 스테이지에서 `cargo pgrx install`을 수행하고, 런타임 스테이지는 `pgvector/pgvector:pg16` 위에 `.so` / `.control` / `--0.1.0.sql`과 `ontological-bolt` 바이너리만 복사한다. sudo와 rustup은 런타임에서 제외한다 |
| **예상 효과** | 이미지 크기와 공격 표면이 크게 줄고, 배포 산출물이 소스 트리에서 독립한다 |
| **리스크** | 개발 워크플로(`start.sh`)와 별개 경로가 되므로 둘이 어긋나지 않도록 CI에서 두 이미지를 모두 빌드해야 한다 |

---

### OPS-06
**PGDATA 볼륨 부재 — `docker rm` 한 번이 데이터 소실**

| | |
|---|---|
| **심각도** | **High** |
| **근거** | `start.sh:21-22`가 만드는 볼륨은 `ontological-target`(빌드 산출물)과 `ontological-cargo`(cargo 레지스트리) **둘뿐**이다. `start.sh:23-28`의 `docker run`에도 PostgreSQL 데이터 디렉터리에 대한 `-v`가 없다 |
| **현상** | PostgreSQL 데이터는 컨테이너의 쓰기 가능 레이어에 남는다. `docker rm -f ontological-dev` 한 번으로 그래프 전체가 사라진다. 컨테이너 재생성이 개발 중 흔한 동작이라는 점에서 위험이 크다 |
| **제안** | ① `docker volume create ontological-pgdata` 를 추가하고 pgrx 데이터 디렉터리(`SHOW data_directory`로 확인 가능)를 마운트한다. ② 그것이 어려우면(pgrx가 홈 디렉터리 아래에 데이터를 만들므로) `/home/dev/.pgrx` 전체를 볼륨으로 잡는 방법을 검토한다. ③ 최소한 `start.sh` 또는 README에 **"컨테이너 삭제 = 데이터 삭제"를 명시**한다 |
| **예상 효과** | 컨테이너 재생성이 데이터와 무관해진다 |
| **리스크** | 볼륨에 담긴 PGDATA는 이미지의 PostgreSQL 메이저 버전과 묶인다. 버전 업그레이드 시 볼륨 초기화 절차가 별도로 필요하다 |

---

### OPS-07
**Bolt 게이트웨이에 프로세스 감독·그레이스풀 셧다운이 없다**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `start.sh:61-63` — `docker exec -d`로 백그라운드 실행, 로그는 컨테이너 안 `/tmp/ontological-bolt.log`. 존재 판정은 `pgrep -f ontological-bolt` (`start.sh:57`). `bolt/src/main.rs:36-80`에는 시그널 핸들러가 **없다**(`grep -rn "signal\|SIGTERM\|SIGINT" bolt/src/` → 결과 없음). 연결마다 스레드를 띄우고(`bolt/src/main.rs:71`) 종료 경로가 정의되어 있지 않다. 빌드 실패는 `\|\| true`로 무시된다(`start.sh:60`) |
| **현상** | ① 게이트웨이가 죽으면 다시 뜨지 않고, Bolt 클라이언트만 연결 실패를 본다 — PostgreSQL 경로는 정상이므로 헬스체크로 드러나지 않는다. ② 종료 시 진행 중인 세션과 그 PostgreSQL 커넥션이 정리되지 않는다. ③ 로그가 컨테이너 안의 `/tmp`에 있어 컨테이너를 지우면 함께 사라지고, 로테이션도 없다. ④ 빌드가 실패해도 `docker exec -d`가 실행되어 로그에만 흔적이 남는다 |
| **제안** | ① `main`에 `SIGTERM`/`SIGINT` 핸들러를 붙여 리스너를 닫고 활성 세션의 커밋/롤백을 마친 뒤 종료한다. ② 컨테이너 안에서 supervisor(또는 별도 컨테이너 + `restart: unless-stopped`)로 감독한다. ③ 로그를 stdout으로 내보내 도커 로깅 드라이버가 처리하게 한다. ④ `start.sh:60`의 `\|\| true`를 제거하고 빌드 실패 시 게이트웨이 기동을 건너뛴다 |
| **예상 효과** | Bolt 경로의 가용성이 관측·복구 가능해진다 |
| **리스크** | 낮음. 그레이스풀 셧다운은 세션 상태(`bolt/src/session.rs`)를 건드리므로 트랜잭션 처리 로직에 대한 테스트가 필요하다 |

---

### OPS-08
**확장 수준 헬스체크 함수 부재**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | 공개 함수 목록(`#[pg_extern]`)에 헬스체크에 해당하는 것이 없다 — 가장 가까운 것이 `ontological_version()`(`engine/src/lib.rs:40-43`)과 `og_check_integrity()`(`engine/src/storage/stats.rs:172`)이다. HTTP 헬스 엔드포인트는 Studio의 `GET /api/health` 하나뿐이며(`portal/server/index.js:151-165`), 그것은 Studio가 PostgreSQL에 붙을 수 있는지를 말한다 |
| **현상** | 오케스트레이터(Kubernetes liveness/readiness, 로드밸런서)가 확인할 단일 지점이 없다. 운영자는 `ontological_version()` + `og_catalog.graph` 조회 + `og_check_integrity()`를 각각 조합해 스스로 만들어야 한다 |
| **제안** | `og_health()` 함수를 추가한다 — `jsonb`로 확장 버전, 그래프 수, 타입 수, 마지막 스키마 버전, 무결성 위반 여부(빠른 샘플), 인접 패킹 비율을 한 번에 반환. 비용이 큰 검사는 인자로 분리한다(`og_health(deep => false)`). Studio의 `/api/health`가 이 함수를 호출하도록 바꾸면 두 층의 정의가 하나가 된다 |
| **예상 효과** | 헬스 판정 기준이 한 곳에 모이고, Bolt/Studio/외부 모니터가 같은 답을 본다 |
| **리스크** | 헬스체크가 무거워지면 그 자체가 부하가 된다. `og_check_integrity()`는 전체 스캔에 가까우므로 기본 경로에서 제외해야 한다 |

---

### OPS-09
**Studio에 `SIGTERM` 핸들러가 없다**

| | |
|---|---|
| **심각도** | **Low** |
| **근거** | `portal/server/index.js:374-376` — `process.on('SIGINT', ...)` **하나뿐**. `start.sh:73`은 `pkill -f "portal/server/index.js"`로 죽이는데, `pkill`의 기본 시그널은 `SIGTERM`이다 |
| **현상** | 재기동 때마다 커넥션 풀(`pool.end()`)이 정리되지 않은 채 프로세스가 종료된다. PostgreSQL 쪽에 잠시 유휴 커넥션이 남고, 진행 중인 질의는 백엔드에서 계속 돈다 |
| **제안** | `SIGTERM`에도 같은 핸들러를 등록한다. 아울러 `server.close()`를 함께 호출해 새 요청을 받지 않도록 하고, 종료 타임아웃을 둔다 |
| **예상 효과** | 재기동이 깨끗해지고, 컨테이너/오케스트레이터 환경에서도 동일하게 동작한다 |
| **리스크** | 없음에 가깝다 |

---

### OPS-10
**씨앗 설정 키 4종을 읽는 코드가 없다**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `engine/sql/bootstrap.sql:256-260`이 `chunk_size`=256, `supernode_threshold`=4096, `inference_max_depth`=16, `schema_version`=1을 심는다. `grep -rn`으로 확인한 결과: `chunk_size` 문자열은 `engine/src/storage/stats.rs:77`에서 **JSON 출력 키**로만 쓰이고 값은 컴파일 상수 `crate::storage::adjacency::CHUNK`(`engine/src/storage/adjacency.rs:15`)에서 온다. `supernode_threshold`와 `inference_max_depth`는 `engine/src/`·`engine/sql/` 어디에서도 참조되지 않는다. `schema_version`은 동명의 별도 테이블(`og_catalog.schema_version`)이 실제 역할을 한다 |
| **현상** | 설정 테이블이 **튜닝 노브처럼 보이지만 아무 효과가 없다.** 운영자가 `og_set_setting('chunk_size','512')`를 실행하면 성공 응답을 받고 동작은 그대로다 — 진단하기 어려운 종류의 오해다 |
| **제안** | 셋 중 하나를 택한다. ① 코드가 이 값을 실제로 읽게 한다(`CHUNK`는 배열 크기 가정과 얽혀 있으므로 난이도가 높다). ② 씨앗에서 제거한다. ③ 남기되 `og_catalog.setting`에 `COMMENT`를 달거나 키 이름에 `reserved.` 접두사를 붙여 미사용임을 표면화한다. 최소한 [03_configuration.md](03_configuration.md)에 명시하는 것이 현재 취한 조치다 |
| **예상 효과** | 설정 표면이 정직해진다. 존재하지 않는 노브를 돌리며 낭비하는 시간이 사라진다 |
| **리스크** | ②는 이 키를 읽는 외부 도구가 있다면 깨진다(저장소 안에는 없다). 백업 필터(`engine/sql/bootstrap.sql:420-422`)도 함께 손봐야 한다 |

---

### OPS-11
**`og.max_rows` 가드레일이 설정만 되고 읽히지 않는다**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `engine/src/agent/mod.rs:437-439` — `Spi::run(&format!("SET og.max_rows = {rows}")).ok();`. `grep -rn "max_rows" engine/src engine/sql` 결과 이 두 줄이 전부이며, `current_setting('og.max_rows')`를 호출하는 코드가 **없다**. 게다가 `.ok()`로 끝나 `SET` 자체의 실패도 무시된다(같은 함수의 `statement_timeout`, `work_mem`, `default_transaction_read_only`도 동일: `:427,430,434`) |
| **현상** | `og_create_role(..., '{"max_rows": 1000}')` → `og_apply_role(...)`을 실행하면 `{"role":..., "applied":{...}}`가 반환되어 **제한이 걸린 것처럼 보인다.** 실제로는 어떤 질의도 1000행에서 잘리지 않는다. 에이전트 가드레일(spec 008 FR-024..FR-029)의 일부가 무효인 셈이다 |
| **제안** | ① `og_cypher`/`og_typeql`의 결과 반환 지점에서 `current_setting('og.max_rows', true)`를 읽어 잘라내고, 잘렸다는 사실을 결과나 NOTICE로 알린다. ② 구현 전까지는 `og_apply_role`이 지원하지 않는 키를 만나면 NOTICE로 알린다. ③ `.ok()`를 실제 오류 처리로 바꿔 `SET` 실패가 드러나게 한다 |
| **예상 효과** | 선언한 가드레일과 실제 동작이 일치한다 |
| **리스크** | 행 제한을 도입하면 기존에 전체 결과를 받던 호출자의 동작이 바뀐다. 기본값은 무제한으로 두고 명시적으로 설정한 세션에만 적용해야 한다 |

---

### OPS-12
**Studio 커넥션 풀 `max: 8` 하드코딩 + 질의 타임아웃 없음**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `portal/server/index.js:21-29` — `max: 8`, `idleTimeoutMillis: 30_000`이 리터럴로 박혀 있고 환경변수로 조정할 수 없다. `statement_timeout` / `query_timeout` / `connectionTimeoutMillis` 설정이 없다. `POST /api/sql`(`:296-308`)은 임의 SQL을 그대로 `pool.query(sql)`에 넘긴다. `POST /api/cypher`(`:182-215`)는 풀에서 클라이언트를 하나 잡고 `og_cypher` 실행 후 추가로 `og_cypher_sql`까지 호출한다 |
| **현상** | 장기 질의 8개가 동시에 걸리면 **`GET /api/health`를 포함한 모든 엔드포인트가 대기한다.** 재작성되지 못한 깊은 순회 하나가 이 상황을 만들기에 충분하다([09_troubleshooting.md](09_troubleshooting.md) §5). 브라우저에서 요청을 취소해도 백엔드 질의는 계속 돈다 |
| **제안** | ① 풀 크기와 타임아웃을 환경변수로 노출한다(`OG_STUDIO_POOL_MAX`, `OG_STUDIO_STATEMENT_TIMEOUT_MS`). ② 연결 시 `SET statement_timeout`을 걸어 상한을 강제한다. ③ `/api/health`는 풀과 분리된 전용 커넥션(또는 짧은 타임아웃)을 쓴다. ④ HTTP 요청이 끊기면 `pg_cancel_backend`로 질의를 취소한다 |
| **예상 효과** | 콘솔이 자기 자신을 잠그지 않게 되고, 헬스 신호가 부하 중에도 살아 있다 |
| **리스크** | `statement_timeout`을 걸면 정당한 장기 배치 질의가 잘린다. 기본값을 넉넉히 잡고 엔드포인트별로 다르게 두는 것이 안전하다 |

---

### OPS-13
**회귀·벤치 게이트가 전부 수동 실행**

| | |
|---|---|
| **심각도** | **High** |
| **근거** | `.github/` 부재. `bench/harness.py:1221` `compare()`는 `"""CI regression gate — spec 009 FR-019..FR-023."""`라는 docstring을 달고 있지만, 이것을 호출하는 자동화가 저장소에 없다. `tests/run.sh`, `tests/typeql/run.py`, `tests/neo4j-movies/run.py`, `examples/meeting-rooms/verify_mcp.py`도 마찬가지다 |
| **현상** | 잘 만들어진 게이트가 여러 개 있는데(정확성 게이트, 백업 왕복, 무결성, 20% 성능 회귀) 아무도 자동으로 돌리지 않는다. 회귀가 커밋 시점이 아니라 누군가 수동으로 돌릴 때 발견된다 |
| **제안** | 단계적으로 도입한다. **1단계**: `cargo test` + `tests/run.sh`(도커 서비스 위에서). **2단계**: `tests/typeql/run.py`. **3단계**: `bench/harness.py --scale 50000 --degree 20 --systems ontological,ontological_raw,cte --compare-baseline bench/results/baseline.json` — 외부 서버가 필요 없는 시스템만으로 회귀 게이트를 돌린다. **4단계**: Neo4j 컨테이너를 서비스로 띄워 `tests/neo4j-movies/run.py` |
| **예상 효과** | `bench/README.md:109-120`이 정의한 회귀 규율이 실제로 작동한다 |
| **리스크** | CI 러너의 성능 변동이 20% 임계값을 흔든다. 벤치 게이트는 전용 러너나 더 큰 임계값으로 시작하고, 정확성 게이트(`agree`)와 무결성(`integrity_violations`)을 우선 신호로 삼는 편이 안전하다 |

---

### OPS-14
**`tests/run.sh`의 무결성 결과가 종료 코드에 반영되지 않는다**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `tests/run.sh:71-75` |

```bash
psql -h "$HOST" -p "$PORT" -d "$DB" -tAc "SELECT count(*) FROM og_check_integrity()" 2>/dev/null \
  | while read -r n; do
      if [ "${n:-0}" = "0" ]; then echo "integrity                          ok"
      else echo "integrity                          FAIL ($n violations)"; fi
    done
```

| | |
|---|---|
| **현상** | `while` 루프가 파이프의 서브셸에서 돌기 때문에 `$fail` 카운터를 증가시킬 수 없다. `integrity FAIL (37 violations)`가 화면에 찍혀도 `tests/run.sh`의 마지막 줄(`tests/run.sh:79`)은 `0 failed`를 보고 **종료 코드 0**으로 끝난다. 자동화에서 이 스위트를 게이트로 쓰면 무결성 위반을 통과시킨다 |
| **제안** | 파이프를 제거하고 변수에 담는다 — `n="$(psql … -tAc "SELECT count(*) FROM og_check_integrity()")"` 후 `if [ "${n:-0}" != "0" ]; then fail=$((fail+1)); failed+=("integrity"); fi`. 또한 `2>/dev/null`로 psql 오류를 삼키는 부분도 함께 손본다(질의 자체가 실패해도 `n`이 비어 `ok`로 읽힌다) |
| **예상 효과** | 무결성이 실제 게이트가 된다 |
| **리스크** | 없음. 다만 지금까지 통과하던 환경이 실패로 바뀔 수 있으므로, 먼저 현재 상태를 측정하고 도입해야 한다 |

---

### OPS-15
**게시된 벤치마크가 참조하는 결과 파일 1개가 저장소에 없다**

| | |
|---|---|
| **심각도** | **Low** |
| **근거** | `docs/benchmark.md:396-401`의 표가 `bench-50000-20260806T052220Z`를 AGE / AGE explicit 열의 출처로 지정한다. `bench/results/`의 실제 파일 목록에 그 이름이 **없다**(같은 표의 나머지 5개는 모두 존재) |
| **현상** | 게시된 50,000노드 AGE 숫자를 저장소만으로 재현·검증할 수 없다. `bench/README.md:15-20`이 강조하는 "증거로서의 벤치마크" 원칙에 대한 국소적 예외가 생긴다 |
| **제안** | ① 해당 실행 파일을 커밋하거나, ② 다시 측정해 새 파일과 함께 문서의 표를 갱신하거나, ③ 그 열의 출처가 커밋되지 않았음을 문서에 명시한다. 아울러 `docs/benchmark.md`가 참조하는 파일이 `bench/results/`에 존재하는지 검사하는 스크립트를 CI에 추가한다(OPS-13과 함께) |
| **예상 효과** | 게시 숫자와 저장소 증거가 1:1로 대응한다 |
| **리스크** | 없음 |

---

### OPS-16
**메트릭 노출(Prometheus 등) 부재**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | 저장소 전체에서 `metrics`·`prometheus`·`/metrics`·`healthz`·`readyz` 를 검색한 결과, 프로젝트 코드(`engine/`, `bolt/`, `portal/`, `bench/`, `tests/`)에는 **일치하는 것이 없다**(`.claude/agents/*.md`의 에이전트 설명 문서에만 단어가 등장). Studio의 라우트 목록(`portal/server/index.js:140-309`)에도 메트릭 엔드포인트가 없다 |
| **현상** | 시계열 관측이 불가능하다. `og_graph_stats`·`og_degree_distribution`·`og_data.og_audit` 같은 좋은 관측 표면이 이미 있는데, 전부 **사람이 SQL로 물어봐야만** 나온다. 알림(alerting)을 걸 지점이 없다 |
| **제안** | ① 가장 저렴한 경로: `postgres_exporter`의 커스텀 질의 파일로 `og_graph_stats`, `og_check_integrity()` 카운트, `og_data.og_audit`의 최근 오류율/지연을 노출한다 — 코드 변경 없이 가능하다. ② Studio에 `GET /metrics`를 추가해 같은 값을 Prometheus 텍스트 포맷으로 내보낸다. ③ 핵심 지표 후보: 그래프별 노드/엣지 수, `packing_ratio`, `chunked_supernodes`, 무결성 위반 수, 감사 로그 기준 질의율·오류율·p95 지연, `og_stale_embeddings` 수, `og_data.og_audit` 크기 |
| **예상 효과** | 성능 저하와 무결성 문제를 사후가 아니라 추세로 발견할 수 있다 |
| **리스크** | 일부 지표(무결성 검사, 전체 카운트)는 비용이 있다. 수집 주기를 길게(예: 5분) 잡고, 비싼 검사는 별도 주기로 분리해야 한다 |

---

### OPS-17
**백업 시 확장 버전 고정 전략 부재**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `engine/ontological.control:2` — `default_version = '0.1.0'` (고정). `engine/src/lib.rs:41-43` — `ontological_version()`은 `env!("CARGO_PKG_VERSION")`을 그대로 반환하므로 `engine/Cargo.toml:3`의 `version = "0.1.0"`과 같다. 업그레이드 스크립트 부재(OPS-02). `tests/run.sh:36-67`의 백업 왕복 테스트는 **같은 서버·같은 확장**에서만 검증한다 |
| **현상** | `pg_dump` 결과물에는 `CREATE EXTENSION ontological` 한 줄이 들어가고, 그것이 만들 스키마는 **복원 시점에 설치된 `.so`가 결정한다.** 버전 번호가 항상 `0.1.0`이므로 백업 파일이 어느 스키마의 것인지 파일만 보고 알 수 없다. `bootstrap.sql`이 바뀐 뒤 옛 덤프를 복원하면 `COPY` 문이 새 스키마와 어긋나 실패하거나, 최악의 경우 일부만 들어간다 |
| **제안** | ① 백업 파이프라인이 `ontological_version()`과 **커밋 해시**를 사이드카 메타파일에 기록하도록 한다([08_backup_and_restore.md](08_backup_and_restore.md)의 스크립트 예시가 이 방식이다). ② `og_catalog.schema_version`에 물리 스키마 버전을 기록하고 복원 후 검증한다. ③ 근본 해결은 OPS-02 — 스키마 변경마다 `default_version`을 올린다 |
| **예상 효과** | 복원 전에 호환 여부를 판단할 수 있고, 불일치가 조용한 데이터 손실이 아니라 명시적 실패가 된다 |
| **리스크** | 낮음. 메타파일 관리 규율이 추가된다 |

---

### OPS-18
**확장이 PostgreSQL 로그에 수명주기 이벤트를 남기지 않는다**

| | |
|---|---|
| **심각도** | **Med** |
| **근거** | `grep -rnE "pgrx::(log\|notice\|warning\|info\|debug1)!" engine/src` 결과가 **정확히 2건**이다: `engine/src/catalog/types.rs:96` (`pgrx::log!` — 별칭 뷰 생성 실패), `engine/src/catalog/types.rs:168` (`pgrx::notice!` — 없는 라벨 힌트). 반면 `error!`는 14개 파일에 걸쳐 약 115곳에서 쓰인다 |
| **현상** | 로그에 남는 것은 **치명적 오류(ERROR) 아니면 아무것도 없다**는 이분법이다. 확장 로드, 그래프 생성, 타입 생성/삭제, `og_reorganize` 실행량, `og_relabel` 수행, 임베딩 엔드포인트 호출, CSR 빌드 같은 사건이 서버 로그에 흔적을 남기지 않는다. 사후 조사에서 "언제 무엇이 바뀌었나"를 서버 로그로 재구성할 수 없다. `og_data.og_audit`가 일부를 메우지만 그것은 **질의 단위**이고 DDL·유지보수 작업은 담지 않는다(감사 기록조차 `.ok()`로 실패를 무시한다 — `engine/src/cypher/mod.rs:134`) |
| **제안** | ① 스키마 변경 경로(`bump_schema_version` 호출 지점, `engine/src/catalog/labeling.rs:172`)에 `pgrx::log!`를 추가한다 — 이미 `og_catalog.schema_version`에 기록하고 있으므로 로그는 자연스러운 짝이다. ② `og_reorganize`가 재패킹한 그룹 수를, `og_relabel`이 재라벨링한 타입 수를 `log!`로 남긴다. ③ `og_genai_encode`의 외부 호출 실패를 `warning!`으로도 남긴다(현재는 `error!`로 질의 자체를 죽인다). ④ `warning!` 레벨을 도입해 "치명적이지 않지만 알아야 할 것"(별칭 뷰 실패, 감사 기록 실패)을 표현한다 |
| **예상 효과** | 서버 로그만으로 스키마 변경 이력과 유지보수 작업을 재구성할 수 있다. 로그 기반 알림이 가능해진다 |
| **리스크** | 로그가 과도하면 그 자체가 부하다. 질의 경로(hot path)에는 넣지 말고 DDL·유지보수 경로에 한정해야 한다 |

---

## 이 문서가 개선 대상으로 삼지 **않은** 것

혼동을 막기 위해 명시한다.

| 항목 | 왜 개선 대상이 아닌가 |
|---|---|
| CSR 스냅샷이 얼어붙는 것 | 명시된 트레이드오프다 (`docs/deep-traversal.md:257-259`). 트리거 캡처는 "이 경로를 유지한다면 다음에 할 일"로 이미 문서에 적혀 있다 |
| CSR이 RLS를 참조하지 않는 것 | 동일 (`docs/deep-traversal.md:260-261`). 그래서 Cypher 컴파일러가 CSR로 라우팅하지 않는다 |
| `genai.vector.encode`가 기본 비활성인 것 | 의도된 안전 기본값 (`engine/src/compat/genai.rs:13-25`) |
| Bolt가 TLS를 종단하지 않는 것 | 명시된 설계 — 앞단 프록시 (`bolt/README.md:68`) |
| Bolt 5.x 미지원 | 명시된 지원 매트릭스 (`bolt/README.md:60-61`) |
| 벤치마크에 동시성 워크로드가 없는 것 | 하네스의 공백으로 이미 선언되어 있다 (`bench/README.md:127`) |
| Cypher의 `UNION` 미구현 | 기능 범위이지 운영 문제가 아니다. `README.md`의 스펙 상태표와 [`docs/cypher.md`](../cypher.md) 참조 |

---

## 금지 / 필수

### 금지 (Forbidden)

- 이 표의 항목을 근거 없이 확대 해석하지 말 것. 각 항목의 **근거** 칸이 주장의 범위다.
- 이 문서를 근거로 코드를 즉시 고치지 말 것 — 제안이며, 각 항목에 리스크가 적혀 있다.

### 필수 (Required)

- 항목을 해결했다면 이 문서와 해당 운영 문서(01~09)를 함께 갱신할 것.
- 새 운영 개선 포인트를 추가할 때는 **반드시 파일:라인 근거**를 붙일 것.

---

<!-- affects: ops, backend, frontend, data -->
<!-- requires-update: docs/08_operations/00_index.md -->
