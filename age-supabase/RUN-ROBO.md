# Robo Architect Lite 를 Supabase Postgres + Apache AGE 로 띄우기

Robo Architect Lite 는 Neo4j 드라이버(Bolt)로 그래프를 쓴다. 여기서는 그 그래프를
Supabase Postgres 안의 Apache AGE 1.8.0 에 두고, 사이에 Bolt 게이트웨이
(`bolt/`, `OG_BOLT_BACKEND=age`)를 세운다. Robo 쪽은 접속값만 바꾼다. Robo 저장소의
코드와 `.env` 는 건드리지 않는다.

```
Robo 백엔드(uvicorn) ── Bolt ──▶ 게이트웨이 :7697 ── PostgreSQL ──▶ Supabase Postgres + AGE
Robo 프런트엔드(vite) ─ /api ─▶ 백엔드 :8000                         DB `graph` (LC_COLLATE C), 역할 robo
```

## 준비

- Docker, 이미지 `uengine/supabase-postgres-age:15.8.1.060-age1.8.0`(`./build.sh`)
- 게이트웨이: Rust(`cargo`, 기본) 또는 이미지 `uengine/ontological-bolt:age`(`./gateway/build.sh`)
- Robo Architect Lite 체크아웃(`ROBO_DIR`, 기본 `/Users/uengine/robo-architect-lite`)에 `.venv` 와 `frontend/node_modules`

## 한 줄로

```bash
cd age-supabase
./run-robo.sh up        # DB · 게이트웨이 · Robo 백엔드 · 프런트엔드
# 브라우저: http://127.0.0.1:5173
./run-robo.sh status
./run-robo.sh down      # 멈춤. DB 볼륨은 남는다
```

`up` 이 하는 일:

1. **DB.** `DB_CONTAINER` 를 주지 않으면 `compose/robo-age.yml`(프로젝트 `robo-age`)로 DB 를
   새로 띄운다(127.0.0.1:55449). 관리자 비밀번호는 처음에 만들어
   `~/.age-supabase/secrets/admin.password`(0600)에 둔다.
2. **AGE 와 그래프 DB.** `scripts/install-age.sh` 를 돌린다. 이미 있으면 아무것도 바꾸지 않는다.
   모든 DB 와 template1 에 age 확장을 만들고, `graph` DB 가 없으면 만든다(LC_COLLATE C).
3. **역할.** `scripts/create-robo-role.sh` 로 로그인 역할 `robo` 를 만들고 `graph` 에 권한을 준다.
   비밀번호는 `~/.age-supabase/secrets/robo.password`(0600)에 둔다. 서버에는 SCRAM 검증값만 간다.
4. **게이트웨이.** `bolt/` 를 cargo 로 빌드해 `127.0.0.1:7697` 에 띄운다(`GATEWAY=native`).
   붙인 뒤 `MATCH (n) RETURN count(n)` 로 확인한다.
5. **Robo.** `ROBO_DIR` 에서 백엔드를 띄운다. 넘기는 환경은
   `NEO4J_URI=bolt://127.0.0.1:7697`, `NEO4J_USER=robo`, `NEO4J_PASSWORD=<robo.password>`,
   `NEO4J_DATABASE=robo` 이다. 프런트엔드는 `npm run dev` 로 띄우고,
   `ARCHITECT_API_URL` 로 백엔드를 가리킨다. 로그와 pid 는 `~/.age-supabase/robo/` 에 남는다.

그래프 `robo` 는 처음 쓸 때 게이트웨이가 만든다. 만드는 역할은 `robo` 다. 그래서 `robo` 가
그 그래프의 스키마와 라벨 테이블의 주인이 되고, 새 라벨을 더할 수 있다. 빈 그래프에서
시작하므로 예시 설계가 필요하면 Robo 의 `docs/cypher/schema/01~04` 를 Bolt 로 넣는다.
런처가 하는 것과 같다.

## 설정

| 변수 | 기본 | 뜻 |
|---|---|---|
| `DB_CONTAINER` | 없음 | 이미 떠 있는 AGE 이미지 컨테이너를 쓴다(예: `agesb-db`, Supabase compose 의 `supabase-db`). 이때는 `ADMIN_PW_FILE` 에 그 DB 의 supabase_admin 비밀번호가 있어야 한다 |
| `GATEWAY` | `native` | `native`: cargo 빌드 후 호스트에서 실행. `docker`: 이미지 `uengine/ontological-bolt:age`(standalone 이면 compose 의 `graph-bolt`, `DB_CONTAINER` 면 그 네트워크의 `robo-age-bolt`). `none`: 이미 떠 있는 것을 쓴다 |
| `ROBO_PARTS` | `api,ui` | `api` 는 백엔드만, `none` 은 DB 와 게이트웨이만 띄운다 |
| `ROBO_GRAPH` | `robo` | 프로젝트 그래프 이름(= Bolt database) |
| `BOLT_PORT` · `API_PORT` · `UI_PORT` · `AGE_DB_PORT` | 7697 · 8000 · 5173 · 55449 | |
| `SECRETS_DIR` · `ADMIN_PW_FILE` · `ROBO_PW_FILE` | `~/.age-supabase/secrets/…` | |
| `STATE_DIR` | `~/.age-supabase/robo` | 로그, pid |

예

```bash
# 이 저장소의 시험 DB(agesb-db)에 붙이기
DB_CONTAINER=agesb-db SECRETS_DIR=/path/to/agesb/secrets ./run-robo.sh up

# DB 와 게이트웨이만(백엔드는 직접: ./run-robo.sh env 가 접속값을 보여 준다)
ROBO_PARTS=none ./run-robo.sh up
```

## 회사 Supabase compose 에 얹기

Supabase 자체 호스팅 compose(DB 서비스 이름이 `db` 인 것)에 덮어쓰기 파일 하나를 더한다.

```bash
./gateway/build.sh                                   # uengine/ontological-bolt:age
cd <Supabase compose 폴더>
docker compose -f docker-compose.yml -f <이 저장소>/age-supabase/compose/supabase-age.override.yml up -d
cd <이 저장소>/age-supabase
DB_CONTAINER=supabase-db ADMIN_PW_FILE=<POSTGRES_PASSWORD 가 든 파일> GATEWAY=none ./run-robo.sh up
```

- `db` 의 이미지만 바뀐다. Supabase 판(15.8.1.060)이 같으므로 역할·확장·서비스는 그대로다.
  Lite 설치판은 `migrate` 서비스도 같은 이미지로 바꾼다.
- **새 볼륨이면** 이미지 initdb 가 AGE, `graph` DB, `_supabase`·template1 의 age 확장을 만든다.
- **이미 있는 볼륨이면** initdb 가 다시 돌지 않는다. `run-robo.sh up`(속의 `install-age.sh`)이 같은
  SQL 을 한 번 돌린다. 기존 데이터는 바뀌지 않는다.
- 게이트웨이(`graph-bolt`)는 `db:5432` 의 `graph` DB 에 붙는다. 열리는 곳은 `127.0.0.1:7697` 뿐이다.

## 알아 둘 것

- **그래프는 `graph` DB 에 있다.** `postgres` DB 가 아니다. 문자열 정렬을 Neo4j 와 맞추려면
  LC_COLLATE C 가 필요한데, `postgres` DB 의 정렬은 바꿀 수 없다. 그래서 PostgREST·pg_graphql·
  Studio·Realtime 은 이 그래프를 보지 못한다. 그래프는 Bolt 게이트웨이로만 다룬다.
- **template0 으로 만든 DB**(`CREATE DATABASE … TEMPLATE template0`, `pg_restore --create`)에는
  age 확장이 없다. AGE 1.8.0 의 TRUNCATE 결함 때문에 첫 TRUNCATE 전에 `install-age.sh` 를 다시 돌린다.
- **덤프·복원** 뒤에는 `sql/age-after-restore.sql` 을 돌린다(README 의 Known limits).
- Robo 의 기능 가운데 그래프 DB 의 패브릭(데이터소스·가상 노드), 분석기(robo-data-analyzer),
  카탈로그는 Ontological 의 기능이다. 이 구성에서는 쓰지 않는다. 게이트웨이는 패브릭 문장을
  이름을 밝혀 거부한다.
- 무엇이 되고 무엇이 안 되는지, 그리고 잰 값은
  `uengine-platform/reports/age-on-supabase/ROBO-ON-AGE.md` 에 있다.
