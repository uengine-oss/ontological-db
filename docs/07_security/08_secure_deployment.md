# 안전 배포 체크리스트

> **이 문서가 답하는 질문**
> - 이것을 안전하게 띄우려면 정확히 무엇을 해야 하는가?
> - 복사해서 바로 쓸 수 있는 설정은 무엇인가?
> - 배포 형태별(단일 사용자 / 애플리케이션 / 다중 테넌트)로 무엇이 다른가?

> **전제**: 이 문서의 모든 항목은 [`09_improvements_security.md`](09_improvements_security.md)의
> SEC-nn 중 하나에 대응한다. 코드 수정 없이 **운영 설정만으로** 완화할 수 있는
> 것만 여기 담았다. 코드 수정이 필요한 것은 09 문서에 있다.

---

## 0. 배포 형태 판정

| 형태 | 조건 | 필요한 절 |
|---|---|---|
| **L — 로컬 단일 사용자** | 개발자 노트북, 외부 접근 없음 | §1, §2 |
| **A — 단일 테넌트 애플리케이션** | 서버 배포, 애플리케이션 하나가 접속 | §1 ~ §3, §5, §6 |
| **M — 다중 테넌트** | 여러 조직의 데이터가 한 DB에 | §1 ~ §7 **전부**, 그리고 §7의 경고를 먼저 읽을 것 |

---

## 1. 네트워크 (모든 형태 필수)

### 1.1 `start.sh` 를 그대로 쓰지 말 것

`start.sh:26-27` 은 컨테이너 포트를 `0.0.0.0`에 게시한다. 루프백으로 고정한다.

```bash
# start.sh 의 docker run 부분을 다음으로 대체
docker run -d --name "$CONTAINER" \
    -v "$ROOT":/work \
    -v ontological-target:/work/engine/target \
    -v ontological-cargo:/home/dev/.cargo/registry \
    -p 127.0.0.1:"$PGPORT":"$PGPORT" \
    -p 127.0.0.1:"$BOLTPORT":7687 \
    -w /work ontological-dev:latest sleep infinity
```

### 1.2 Studio 를 루프백에 고정

현재 코드는 모든 인터페이스에 바인드한다(`portal/server/index.js:368`).
코드를 고치지 않는다면 방화벽으로 막는다.

```bash
# 코드 수정 없이 — 호스트 방화벽
sudo iptables -A INPUT -p tcp --dport 7474 ! -i lo -j DROP
sudo ip6tables -A INPUT -p tcp --dport 7474 ! -i lo -j DROP
```

```js
// 코드를 고칠 수 있다면 — portal/server/index.js:368
server.listen(PORT, '127.0.0.1', () => { … });
```

> **중요**: 루프백 고정만으로는 CSRF를 막지 못한다.
> [`06_network_exposure.md`](06_network_exposure.md) §2.1 참조.
> 형태 A·M에서는 §1.3을 따라 Studio 자체를 실행하지 않는 것이 맞다.

### 1.3 형태 A·M: Studio 를 실행하지 말 것

`portal/server/index.js:296-308` 의 `POST /api/sql` 은 인증 없이 임의 SQL을
실행한다. 인증을 붙일 자리가 코드에 없다. 프로덕션에서는 프로세스를 띄우지 않는다.

```bash
# start.sh 사용 시 Studio 기동 부분을 건너뛴다 (OG_BOLT 와 달리 스위치가 없으므로
# 스크립트를 직접 편집하거나 아래처럼 개별 구성 요소만 띄운다)
docker exec "$CONTAINER" bash -lc "cd /work/engine && cargo pgrx start pg16"
# node portal/server/index.js  ← 실행하지 않는다
```

### 1.4 Bolt 게이트웨이

애플리케이션 계층 TLS가 없다(`bolt/src/main.rs:46`, `bolt/src/session.rs:182`).

```bash
# 루프백에만 바인드
OG_BOLT_LISTEN=127.0.0.1:7687 \
OG_BOLT_PGHOST=127.0.0.1 \
OG_BOLT_PGPORT=5432 \
OG_BOLT_PGDATABASE=og \
OG_BOLT_ADVERTISED=127.0.0.1:7687 \
  ./bolt/target/release/ontological-bolt
```

원격 클라이언트가 필요하면 암호화 터널을 밖에 둔다.

```ini
# /etc/stunnel/bolt.conf — 서버 측
[bolt]
accept  = 0.0.0.0:7688
connect = 127.0.0.1:7687
cert    = /etc/stunnel/server.pem
CAfile  = /etc/stunnel/clients.pem
verify  = 2
```

Bolt 게이트웨이는 **무제한 암호 추측 오라클**이므로
([`06`](06_network_exposure.md) §3.2) 접근 자체를 제한한다.

```bash
# fail2ban 대용: 소스 IP당 동시 연결 수 제한
sudo iptables -A INPUT -p tcp --dport 7687 -m connlimit \
     --connlimit-above 8 --connlimit-mask 32 -j REJECT
```

---

## 2. PostgreSQL 인증 (모든 형태 필수)

저장소는 `pg_hba.conf` 를 제공하지 않는다. 명시적으로 설정한다.

```conf
# pg_hba.conf
# TYPE  DATABASE  USER            ADDRESS         METHOD
local   all       postgres                        peer
host    og        og_app          127.0.0.1/32    scram-sha-256
host    og        og_app          ::1/128         scram-sha-256
hostssl og        og_app          10.0.0.0/8      scram-sha-256
# trust 는 어떤 줄에도 쓰지 않는다
```

```conf
# postgresql.conf
listen_addresses = 'localhost'          # 또는 명시적 주소 목록
password_encryption = 'scram-sha-256'
ssl = on
ssl_cert_file = '/etc/postgresql/server.crt'
ssl_key_file  = '/etc/postgresql/server.key'
```

> **Bolt 게이트웨이 사용 시 주의**: `bolt/src/session.rs:182` 가 `NoTls` 이므로
> 게이트웨이가 쓰는 경로에는 `hostssl` 을 쓸 수 없다. 게이트웨이를
> PostgreSQL과 같은 호스트에 두고 `host … 127.0.0.1/32 scram-sha-256` 로만 열 것.

---

## 3. 역할과 권한 (형태 A·M 필수)

확장이 만드는 테이블에는 `GRANT` 가 하나도 없다(`engine/sql/bootstrap.sql` 전체).
따라서 애플리케이션 역할에 필요한 권한을 **명시적으로** 부여해야 한다.

```sql
-- 1) 설치는 슈퍼유저로 (extension 은 trusted = false)
CREATE EXTENSION ontological CASCADE;

-- 2) 애플리케이션 역할
CREATE ROLE og_app LOGIN PASSWORD '...';

-- 3) 스키마 사용 권한
GRANT USAGE ON SCHEMA og_data, og_catalog TO og_app;

-- 4) 카탈로그는 읽기 전용 — 이 테이블들의 값이 동적 SQL 로 보간된다
--    (docs/07_security/04_injection_surface.md §4)
GRANT SELECT ON ALL TABLES IN SCHEMA og_catalog TO og_app;
REVOKE INSERT, UPDATE, DELETE, TRUNCATE
    ON ALL TABLES IN SCHEMA og_catalog FROM og_app;

-- 5) 데이터 테이블
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA og_data TO og_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA og_data TO og_app;
-- 새로 만들어지는 타입 테이블에도 자동 적용
ALTER DEFAULT PRIVILEGES IN SCHEMA og_data
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO og_app;

-- 6) 위험한 함수의 실행 권한 회수
REVOKE EXECUTE ON FUNCTION og_set_setting(text, text)            FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_genai_encode(text, text, jsonb)    FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_map_table(text, text, text, text, jsonb) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_enable_rls(text, text, text)       FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_materialize_mapping(text, text)    FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_drop_graph(text)                   FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_drop_type(text, text, boolean)     FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_create_role(text, jsonb)           FROM PUBLIC;

-- 7) 스키마 변경이 필요 없다면 (권장) — 쓰기 시점 DDL 을 차단하려면
--    og_app 이 타입 테이블 소유자가 아니어야 한다. 아래 3.1 참조.
```

> **함수 시그니처는 배포된 버전에서 확인할 것**:
> `\df og_*` 또는 `SELECT p.oid::regprocedure FROM pg_proc p JOIN pg_namespace n
> ON n.oid = p.pronamespace WHERE p.proname LIKE 'og\_%';`

### 3.1 쓰기 시점 DDL 문제

`engine/src/storage/mod.rs:87-158` 의 `declare_new_props` 는 새 프로퍼티를
만나면 `ALTER TABLE` 을 실행한다. 타입이 충돌하면 **테이블 전체를 재작성**한다
(`storage/mod.rs:138-140`) — `ACCESS EXCLUSIVE` 잠금과 함께.

`og_app` 이 테이블 소유자가 아니면 이 DDL 은 실패하고, 새 프로퍼티는
`__ext` jsonb 에 남는다. 그것이 **더 안전한 기본값**이다.
스키마 승격이 필요하면 별도 관리 역할로 `og_add_property` 를 미리 호출한다.

```sql
-- 관리 역할로 스키마를 미리 확정
SELECT og_add_property('default', 'Person', 'name',  'string');
SELECT og_add_property('default', 'Person', 'age',   'int');
-- 이후 og_app 은 선언된 컬럼에만 쓴다
```

### 3.2 리소스 한도는 역할에 건다

`og_apply_role`(`engine/src/agent/mod.rs:415-441`)은 세션 GUC 만 설정하며
호출자가 되돌릴 수 있고, `og.max_rows` 는 **어떤 코드도 읽지 않는다**.

```sql
ALTER ROLE og_app SET statement_timeout            = '30s';
ALTER ROLE og_app SET idle_in_transaction_session_timeout = '60s';
ALTER ROLE og_app SET work_mem                     = '32MB';
ALTER ROLE og_app SET lock_timeout                 = '5s';
ALTER ROLE og_app SET temp_file_limit              = '1GB';
ALTER ROLE og_app CONNECTION LIMIT 40;
```

### 3.3 `search_path` 하이재킹 차단

확장 함수 어디에도 `SET search_path` 가 없고, 컴파일된 SQL 은 확장 함수를
스키마 한정 없이 부른다([`04`](04_injection_surface.md) §8).

```sql
-- public 에 아무나 객체를 만들지 못하게
REVOKE CREATE ON SCHEMA public FROM PUBLIC;   -- PG15+ 는 기본값

-- 역할별 search_path 고정
ALTER ROLE og_app SET search_path = "$user", public;

-- og_data 에 임의 객체 생성 금지 (RLS 를 쓸 때는 3.4 의 예외 참조)
REVOKE CREATE ON SCHEMA og_data, og_catalog FROM PUBLIC;
```

---

## 4. RLS (형태 M — 그러나 §7을 먼저 읽을 것)

`og_enable_rls` 만으로는 격리가 성립하지 않는다
([`03_rls_and_isolation.md`](03_rls_and_isolation.md)). 아래 3단계를 모두 수행한다.

```sql
-- 1) 정책 생성 (확장이 제공하는 부분)
SELECT og_enable_rls('default', 'Person',
    'p_tenant_id = current_setting(''app.tenant'', true)::int');

-- 2) 소유자 우회 차단 — 확장이 하지 않는다
DO $$
DECLARE t text;
BEGIN
    FOR t IN
        SELECT storage_table FROM og_catalog.type
         WHERE storage_table IS NOT NULL
    LOOP
        EXECUTE format('ALTER TABLE %s FORCE ROW LEVEL SECURITY', t);
    END LOOP;
END $$;

-- 3) 생성 뷰의 security_invoker — 확장이 하지 않는다
--    (engine/src/cypher/views.rs:135 가 WITH (security_invoker) 없이 만든다)
DO $$
DECLARE v text;
BEGIN
    FOR v IN
        SELECT 'og_data.' || quote_ident(c.relname)
          FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'og_data' AND c.relkind = 'v'
    LOOP
        EXECUTE format('ALTER VIEW %s SET (security_invoker = true)', v);
    END LOOP;
END $$;
```

**2·3단계는 스키마가 바뀔 때마다 다시 실행해야 한다.**
`labeling::bump_schema_version()`(`engine/src/catalog/labeling.rs:172-182`)이
모든 생성 뷰를 지우고 다음 질의자가 다시 만들기 때문이다. 이벤트 트리거로
자동화할 수 있다.

```sql
CREATE OR REPLACE FUNCTION og_secure_new_views() RETURNS event_trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE obj record;
BEGIN
    FOR obj IN SELECT * FROM pg_event_trigger_ddl_commands()
                WHERE object_type = 'view'
                  AND schema_name = 'og_data'
    LOOP
        EXECUTE format('ALTER VIEW %s SET (security_invoker = true)',
                       obj.object_identity);
    END LOOP;
END $$;

CREATE EVENT TRIGGER og_secure_views
    ON ddl_command_end WHEN TAG IN ('CREATE VIEW')
    EXECUTE FUNCTION og_secure_new_views();
```

### 4.1 RLS 배포에서 반드시 회수할 실행 권한

```sql
-- 스냅샷 CSR 은 RLS 를 조회하지 않는다 (traverse.rs:19-23 이 명시)
REVOKE EXECUTE ON FUNCTION og_csr_build(int4[], text)  FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_csr_reach(int8, int4, int4) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_csr_hops(int8, int8, int4)  FROM PUBLIC;

-- 히스토리는 RLS 밖 테이블에 같은 값을 복제한다
REVOKE EXECUTE ON FUNCTION og_enable_history(text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_history(int8)              FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_as_of(int8, timestamptz)   FROM PUBLIC;

-- 감사 로그는 다른 주체의 질의 원문을 담는다
REVOKE SELECT ON og_data.og_audit FROM og_app;
GRANT  INSERT ON og_data.og_audit TO og_app;   -- 기록은 계속되게
GRANT  USAGE, SELECT ON SEQUENCE og_data.og_audit_audit_id_seq TO og_app;
```

---

## 5. 아웃바운드와 비밀 (`genai` 사용 시)

```sql
-- 필요할 때만 켠다 (기본은 꺼져 있다 — genai.rs:101-107)
SELECT og_set_setting('genai.enabled',  'on');
SELECT og_set_setting('genai.endpoint', 'https://embeddings.internal:8443/v1/embeddings');
SELECT og_set_setting('genai.provider', 'openai');
SELECT og_set_setting('genai.model',    'text-embedding-3-small');
SELECT og_set_setting('genai.timeout_ms', '3000');
-- 토큰은 평문으로 저장되고 pg_dump 에 포함된다 (bootstrap.sql:420-422)
SELECT og_set_setting('genai.token',    '...');
```

토큰이 평문이라는 사실을 전제로 운영한다.

```sql
-- 설정 테이블의 읽기 권한을 좁힌다
REVOKE SELECT ON og_catalog.setting FROM og_app, PUBLIC;
-- og_genai_encode 는 이 테이블을 읽으므로, 필요하면 전용 역할을 둔다
```

```bash
# 데이터베이스 호스트의 아웃바운드를 임베딩 엔드포인트로만 제한
# (코드에 허용 목록이 없다 — genai.rs:139 가 URL 을 그대로 쓴다)
sudo iptables -A OUTPUT -m owner --uid-owner postgres \
     -d 10.20.0.15 -p tcp --dport 8443 -j ACCEPT
sudo iptables -A OUTPUT -m owner --uid-owner postgres \
     -d 169.254.169.254 -j DROP           # 클라우드 메타데이터 차단
sudo iptables -A OUTPUT -m owner --uid-owner postgres \
     -p tcp -j REJECT
```

백업에서 토큰을 제거하려면 덤프 후처리가 필요하다.

```bash
pg_dump -d og | grep -v "genai.token" > og.sql   # 최소한의 조치
```

---

## 6. 감사 (형태 A·M)

`og_data.og_audit` 는 성공한 질의만 담고 직접 SQL 은 담지 않는다
([`07_audit_and_history.md`](07_audit_and_history.md) §2). PostgreSQL 수준 로깅을 병행한다.

```conf
# postgresql.conf
log_destination = 'stderr,csvlog'
logging_collector = on
log_connections = on
log_disconnections = on
log_statement = 'ddl'                 # 또는 pgaudit 사용
log_min_duration_statement = 1000
log_line_prefix = '%m [%p] %q%u@%d %a '
```

```sql
-- 보존 정책 — 코드에 없다
DELETE FROM og_data.og_audit   WHERE at         < now() - interval '90 days';
DELETE FROM og_data.og_history WHERE recorded_at < now() - interval '1 year';
```

---

## 7. 다중 테넌트에 대한 경고 (읽고 결정할 것)

`og_data.og_adj`, `og_data.og_node`, `og_data.og_edge` 에는 **테넌트를 식별할
컬럼이 없다**(`engine/sql/bootstrap.sql:197-241`). 그리고 모든 관계 순회가
`og_data.og_adj` 를 직접 읽는다(`engine/src/cypher/compile.rs:901`).

따라서:

| 원하는 것 | 현재 스키마에서 가능한가 |
|---|---|
| 테넌트별 **프로퍼티 값** 격리 | 예 — §4를 모두 수행하면 |
| 테넌트별 **노드 존재 여부** 격리 | 아니오 — `og_node` 에 RLS 를 걸 컬럼이 없다 |
| 테넌트별 **토폴로지(관계 구조)** 격리 | **아니오** — `og_adj` 에 RLS 를 걸 컬럼이 없다 |
| 테넌트별 히스토리 격리 | 아니오 — §4.1로 접근을 막는 것이 최선 |

**결정**: 토폴로지 기밀성이 요구되는 다중 테넌트는
**데이터베이스 또는 클러스터를 분리할 것.** RLS 단독으로는 달성할 수 없다.

```bash
# 테넌트별 데이터베이스
createdb -O og_app_acme  og_acme  && psql -d og_acme  -c 'CREATE EXTENSION ontological CASCADE'
createdb -O og_app_globex og_globex && psql -d og_globex -c 'CREATE EXTENSION ontological CASCADE'
```

---

## 8. 배포 전 확인 스크립트

```sql
-- 붙여 넣고 결과를 확인한다. 모든 행이 'OK' 여야 한다.
SELECT 'listen_addresses' AS check,
       CASE WHEN current_setting('listen_addresses') IN ('localhost','127.0.0.1')
            THEN 'OK' ELSE 'REVIEW: ' || current_setting('listen_addresses') END AS result
UNION ALL
SELECT 'ssl',
       CASE WHEN current_setting('ssl') = 'on' THEN 'OK' ELSE 'REVIEW: ssl off' END
UNION ALL
SELECT 'standard_conforming_strings',
       CASE WHEN current_setting('standard_conforming_strings') = 'on'
            THEN 'OK' ELSE 'FAIL: sql_str() 이스케이프가 무효' END
UNION ALL
SELECT 'password_encryption',
       CASE WHEN current_setting('password_encryption') = 'scram-sha-256'
            THEN 'OK' ELSE 'REVIEW: ' || current_setting('password_encryption') END
UNION ALL
SELECT 'catalog write grants',
       CASE WHEN NOT EXISTS (
            SELECT 1 FROM information_schema.role_table_grants
             WHERE table_schema = 'og_catalog'
               AND privilege_type IN ('INSERT','UPDATE','DELETE')
               AND grantee NOT IN ('PUBLIC', CURRENT_USER)
       ) THEN 'OK' ELSE 'FAIL: og_catalog 쓰기 권한 존재 (2차 주입)' END
UNION ALL
SELECT 'views security_invoker',
       CASE WHEN NOT EXISTS (
            SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'og_data' AND c.relkind = 'v'
               AND NOT COALESCE((c.reloptions::text LIKE '%security_invoker=true%'), false)
       ) THEN 'OK' ELSE 'FAIL: security_invoker 미설정 뷰 존재 (RLS 우회)' END
UNION ALL
SELECT 'RLS forced',
       CASE WHEN NOT EXISTS (
            SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'og_data' AND c.relkind = 'r'
               AND c.relrowsecurity AND NOT c.relforcerowsecurity
       ) THEN 'OK' ELSE 'FAIL: RLS 는 켜졌으나 FORCE 가 아님 (소유자 우회)' END
UNION ALL
SELECT 'genai token in setting',
       CASE WHEN NOT EXISTS (SELECT 1 FROM og_catalog.setting WHERE key = 'genai.token')
            THEN 'OK' ELSE 'NOTE: 토큰이 평문 저장 중 — pg_dump 취급 주의' END
UNION ALL
SELECT 'public CREATE',
       CASE WHEN NOT has_schema_privilege('public', 'public', 'CREATE')
            THEN 'OK' ELSE 'FAIL: search_path 하이재킹 가능' END;
```

```bash
# 프로세스·포트 확인
ss -lntp | grep -E ':(7474|7687|5432|28816)\b'
# 7474 가 보이면 형태 A·M 에서는 즉시 중단할 것
pgrep -af 'portal/server/index.js'
```

---

## Forbidden (금지)

- **`start.sh` 를 프로덕션에서 실행하지 말 것.** 포트를 `0.0.0.0`에 게시하고
  (`start.sh:26-27`), Studio를 함께 띄우며(`:79-80`), 데모 데이터를 넣는다(`:52`).
- **`docker/Dockerfile.dev` 를 프로덕션 이미지로 쓰지 말 것**
  (`dev ALL=(ALL) NOPASSWD:ALL`, `Dockerfile.dev:13`).
- **`pg_hba.conf` 에 `trust` 를 남겨두지 말 것.**
- **애플리케이션 역할을 타입 테이블의 소유자나 슈퍼유저로 만들지 말 것.**
  `FORCE ROW LEVEL SECURITY` 없이는 소유자가 RLS 를 우회한다.
- **`og_catalog` 에 애플리케이션 역할의 쓰기 권한을 주지 말 것.**
- **RLS 를 켠 뒤 §4의 2·3단계를 생략하지 말 것.** 정책이 있어도 적용되지 않는다.
- **토폴로지 기밀성이 필요한 다중 테넌트를 한 데이터베이스에 두지 말 것**(§7).

## Required (필수)

- 배포 전 §8의 SQL 확인 스크립트를 실행하고 모든 행이 `OK` 인지 볼 것.
- 스키마를 변경할 때마다 §4의 2·3단계를 다시 실행하거나 이벤트 트리거를 걸 것.
- `og_data.og_audit` / `og_data.og_history` 의 보존 정책을 운영자가 직접
  마련할 것 — 코드에 없다.
- Bolt 를 쓴다면 게이트웨이를 PostgreSQL과 같은 호스트에 두고 루프백으로만
  접속시킬 것 (`NoTls` 때문에 `hostssl` 을 쓸 수 없다).
- 이 문서를 수정하면 [`09_improvements_security.md`](09_improvements_security.md)의
  "운영 완화" 열과 일치하는지 확인할 것.

<!-- affects: security, ops, backend -->
<!-- requires-update: 07_security/09_improvements_security.md, 07_security/03_rls_and_isolation.md -->
