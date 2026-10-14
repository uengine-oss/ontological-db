# 반영된 수정 — 무엇이 실제로 바뀌었나

> **이 문서가 답하는 질문**
> - 감사(`7d60c82`) 이후 실제로 고쳐진 항목은 무엇인가?
> - 각 수정이 무엇을 바꿨고, 어떻게 검증했는가?
> - **아직 안 고친 것은 무엇이고, 왜인가?**

> 07_security 의 나머지 문서는 감사 시점 `7d60c82` 의 스냅샷이다. 그 서술은
> 기록으로서 유효하지만 **현재 코드 상태가 아니다.** 현재 상태는 이 문서다.

---

## 0. 요약

Critical 5건은 전부 수정했다. High 13건은 **4건 수정 · 5건 부분 수정 · 4건 미수정**
이다. 부분과 미수정의 이유는 4절에 있다. (스펙 014 이후 SEC-08 이 미수정 → 부분으로 바뀌었다 — 5절.)

| ID | 항목 | 상태 | 검증 |
|---|---|---|---|
| SEC-01 | Studio 무인증 임의 SQL | **fixed** | 기동 테스트 |
| SEC-02 | Studio CSRF | **fixed** | 기동 테스트 |
| SEC-03 | PackStream 길이 필드 선할당 | **fixed** | 회귀 테스트 |
| SEC-04 | 메시지 총 길이 상한 없음 | **fixed** | 회귀 테스트 |
| SEC-05 | 재귀 깊이 제한 없음 | **fixed** | 회귀 테스트 |
| SEC-06 | 생성 뷰 `security_invoker` | **fixed** | `cargo check` |
| SEC-07 | `FORCE ROW LEVEL SECURITY` | **fixed** | `cargo check` |
| SEC-08 | 레지스트리 테이블 RLS 없음 | **부분** — 쓰기만 | `07_write_security.sql`, `10_evolution_temporal_coverage.sql` |
| SEC-09 | `og_vector_search(filter)` 조각 보간 | **완화** | 권한으로 축소 |
| SEC-10 | `og_stale_embeddings` 문자열 보간 | **fixed** | `cargo check` |
| SEC-11 | `og_map_table`/`og_enable_rls` 보간 | **부분** | 식별자만 |
| SEC-12 | `catch_unwind` 가 오류 상태를 삼킴 | 미수정 | — |
| SEC-13 | 쓰기 시점 자동 DDL | 미수정 | — |
| SEC-14 | `og_csr_build` 무제한 할당 | 미수정 | — |
| SEC-15 | Bolt 평문 + `0.0.0.0` 바인드 | **부분** | 바인드만 |
| SEC-16 | Bolt 연결 무제한 | **부분** | 상한만 |
| SEC-17 | `genai` 설정 경유 SSRF | **fixed** | 권한으로 차단 |
| SEC-18 | 감사 로그가 실패를 안 남김 | 미수정 | — |

---

## 1. 권한 모델 — 없던 것을 만들었다

감사에서 가장 큰 발견은 개별 결함이 아니라 **권한 모델 자체의 부재**였다.
두 SQL 파일에 `GRANT` 도 `REVOKE` 도 0줄이었고, 그 결과 상태는 한쪽으로
치우친 게 아니라 양쪽으로 동시에 잘못돼 있었다.

- PostgreSQL 은 새 함수에 **PUBLIC EXECUTE 를 기본으로 부여한다.** 아무도
  그것을 되돌리지 않았으므로 클러스터의 모든 롤이 `og_set_setting`,
  `og_enable_rls`, `og_map_table`, `og_drop_graph` 를 호출할 수 있었다.
- 반면 **테이블에는 어떤 GRANT 도 없었다.** 그래서 같은 롤이 질의는 하나도
  실행할 수 없었다.

즉 열려 있어야 할 곳은 닫혀 있고 닫혀 있어야 할 곳은 열려 있었다. 이 상태에서
SEC-17(SSRF)의 실제 성립 조건도 정정이 필요하다 — `og_set_setting` 은
`og_catalog.setting` 에 INSERT 하므로, **감사 시점에도 소유자가 아니면 실제로는
권한 오류로 실패했다.** SSRF 는 "누구나"가 아니라 "카탈로그에 쓸 수 있는 자"의
문제였다. 지금은 함수 자체가 admin 전용이므로 두 겹 모두 닫힌다.

### 무엇을 했나

**기본 거부.** `engine/sql/access.sql` 끝(`finalize` 단계라 확장의 모든 함수가
이미 카탈로그에 존재한다)에서 확장 스키마의 모든 `og_*` 함수에 대해
`REVOKE ALL … FROM PUBLIC` 을 돈다.

**되돌려 주는 통로.** `og_grant(role, level)` — level 은 `read` / `write` /
`admin` 이고 중첩된다. `og_revoke(role)` 로 되돌린다.

**역할은 만들지 않는다.** 롤은 클러스터 전역이고 `DROP EXTENSION` 보다 오래
살며, `CREATE ROLE` 은 설치자가 못 가질 수도 있는 권한이다. DBA 가 만들고
`og_grant` 를 부른다.

**부여를 기억한다.** 저장 테이블은 구체 타입마다 런타임에 생성되므로, 오늘 준
권한이 아직 없는 테이블에 닿아야 한다. `ALTER DEFAULT PRIVILEGES` 로는 안 되는데
그건 **객체를 만든 롤** 기준이고 이 테이블들은 `og_create_type` 을 호출한 아무나가
만들기 때문이다. 그래서 `og_catalog.grantee` 에 의도를 기록하고 테이블·뷰 생성
시점에 다시 부여한다(`catalog/privileges.rs` 의 `apply_to_table`/`apply_to_view`,
호출 지점 7곳).

**함수 목록은 fail-closed 로.** `READ` 와 `WRITE` 만 열거하고 나머지는 전부
admin 이다. 나중에 추가되는 함수는 누군가 목록에 넣기 전까지 admin 전용이며,
이게 실수가 나야 할 방향이다.

### 읽기 롤이 `og_cypher` 를 가지는 이유

`og_cypher` 는 READ 목록에 있다. Cypher 가 `CREATE` 와 `DELETE` 를 할 수 있는데도
그렇다. **컴파일된 질의는 평범한 테이블을 읽고 쓰기 때문이다.** `og_data` 에
`SELECT` 만 가진 롤은 문장 자체에서 권한 오류를 받는다. 경계는 `EXECUTE` 가 아니라
테이블 권한이고, 그쪽이 우리가 질의를 정확히 파싱하는 데 의존하지 않는다.

> **부수 효과 하나.** 쓰기 롤이 선언되지 않은 프로퍼티를 쓰면 승격(자동 DDL)이
> 권한 부족으로 실패한다. 스키마 변경이 admin 쪽에 남는 건 의도한 선이지만,
> `declare_new_props` 는 실패를 `is_ok()` 로만 보므로 **SPI 오류가 트랜잭션을
> 중단시키는 경로가 남아 있다.** SEC-13 과 함께 미해결이다(4절).
> **→ 스펙 014 에서 수정(6절 결함 2).** 이제 승격 전에 스키마 권한을 확인하고, 없으면
> 값을 `__ext` 에 남긴다.

---

## 2. 뷰와 RLS — SEC-06, SEC-07

**SEC-06.** 생성 뷰 3곳 전부에 `WITH (security_invoker = true)` 를 붙였다:
생성 타입 뷰(`cypher/views.rs`), 별칭 뷰(`catalog/types.rs`), 매핑 뷰
(`interop/mod.rs`). PostgreSQL 15 에서 생긴 옵션이라 pg13/pg14 에는 없고,
`spiu::VIEW_SECURITY` 가 `#[cfg]` 로 갈라진다. **pg13/pg14 에서는 이 구멍이 그대로
남는다** — 거기서는 뷰를 통한 RLS 를 신뢰하면 안 된다.

**SEC-07.** `og_enable_rls` 가 `ENABLE` 만 하고 `FORCE` 를 하지 않았다. `ENABLE`
단독은 **테이블 소유자를 면제하는데**, 저장 테이블의 소유자는 확장 설치자다.
즉 질의를 돌릴 가능성이 가장 높은 롤에게 정책이 평가되지 않았다. 두 문장 다
필요하다.

`docs/07_security/08_secure_deployment.md` 의 수동 `ALTER VIEW … SET
(security_invoker = true)` 스크립트와 `FORCE` 스크립트는 **pg15 이상에서는 더
이상 필요 없다.** 그 문서의 점검 질의는 그대로 두는 게 좋다 — 회귀를 잡는다.

---

## 3. Bolt 와 Studio

### PackStream — SEC-03, SEC-04, SEC-05

세 가지 모두 **인증 전에** 도달 가능했다. 각각에 회귀 테스트를 붙였고, 테스트가
끝난다는 사실 자체가 결과다(전부 `bolt/src/packstream.rs`).

- **선할당(SEC-03).** `0xD6 FF FF FF FF` 5바이트가 `Vec::with_capacity(4294967295)`
  였다. 이제 남은 버퍼 길이로 상한을 둔다 — 임의 상수가 아니라 **증거 기반**이다.
  멤버 하나는 최소 1바이트를 차지하므로 버퍼에 남은 바이트 수가 정직한 상한이다.
- **메시지 길이(SEC-04).** 청크 헤더에 총 길이가 없어 종료 청크가 안 오면 무한히
  누적했다. `MAX_MESSAGE = 16 MiB`. 빈 청크 연속도 `MAX_EMPTY_CHUNKS = 1024` 로
  막았다 — 이전에는 `io::repeat(0x00)` 하나로 무한 루프였다.
- **재귀 깊이(SEC-05).** `0x91` 한 바이트가 스택 프레임 하나를 산다.
  `MAX_DEPTH = 64`. Bolt 자체 메시지는 3~4단이다.

검증: `cargo test` — 10건 통과(신규 4건 포함).

### Bolt 노출 — SEC-15, SEC-16(부분)

기본 바인드를 `0.0.0.0:7687` → **`127.0.0.1:7687`** 로 바꿨다. Bolt 는 비밀번호를
평문으로 받고, 이 프로세스는 클라이언트가 주장한 롤로 PostgreSQL 에 접속해
인증한다. 즉 도달 가능한 포트는 자격증명 스니핑 기회이자 로그인 오라클이다.

`start.sh` 는 `OG_BOLT_LISTEN=0.0.0.0:7687` 을 **명시적으로** 넘긴다. 거기서는
컨테이너 네트워크 네임스페이스가 경계이기 때문이다. 컨테이너 내부 포트는
`7687` 이고 `$BOLTPORT` 가 아니다(`-p "$BOLTPORT":7687`).

세션 상한 `OG_BOLT_MAX_SESSIONS`(기본 256)를 추가했다. 연결당 스레드 + 연결당
PostgreSQL 백엔드였으므로 accept 폭주가 양쪽을 동시에 소진시켰다.

**TLS 는 여전히 없다**(SEC-15 의 절반). 오류 메시지를 통한 계정 열거도 그대로다
(SEC-16 의 절반).

### Studio — SEC-01, SEC-02

- `server.listen(PORT, cb)` 가 모든 인터페이스에 바인드하면서 기동 로그는
  `http://localhost` 라고 찍었다. 이제 기본 `127.0.0.1` 이고, **실제 바인드 주소를
  출력한다.** 루프백이 아닌 주소는 `OG_STUDIO_ALLOW_REMOTE=1` 없이는 기동을 거부한다.
- `POST /api/sql` 은 `OG_STUDIO_ALLOW_RAW_SQL=1` 없이는 403.
- CSRF: `/api/*` 전체에 `Sec-Fetch-Site` / `Origin` 동일 출처 검사, 그리고 본문이
  있는 요청에 `content-type: application/json` 강제. 후자가 핵심이다 —
  `text/plain` 본문은 프리플라이트 없는 "단순 요청"이라 루프백 바인드로도 막히지
  않았고, 예전 `readBody` 는 content-type 을 보지 않고 JSON 으로 파싱했다.
  `application/json` 을 요구하면 프리플라이트가 강제되고, 여기엔 그걸 처리할
  CORS 핸들러가 없다.
- 헤더가 둘 다 없는 요청은 통과시킨다. 브라우저가 아니라는 뜻이고, curl 류는
  애초에 속는 쪽이 아니다.

검증(실기동): 기본 403(raw SQL) / 교차 출처 403 / `text/plain` 415 /
`sec-fetch-site: cross-site` 403 / 정적 파일 200 / 동일 출처는 라우트까지 도달 /
`OG_STUDIO_HOST=0.0.0.0` 단독은 종료코드 1. `ss -ltn` 으로 `127.0.0.1:7999` 단독
바인드 확인.

---

## 4. 안 고친 것과 이유

| ID | 왜 남겼나 |
|---|---|
| SEC-08 | 레지스트리·인접 테이블(`og_node`, `og_edge`, `og_adj`)에 RLS 를 걸면 **모든 순회 경로의 계획이 바뀐다.** 성능 영향이 크고 벤치마크 없이 넣을 변경이 아니다. 헌법 원칙 X. **스펙 014 이후 쓰기 쪽은 닫혔다 — 5절.** 읽기 쪽이 이 문장의 대상으로 남아 있다. |
| SEC-12 | `og_explain_error` 의 `catch_unwind` 는 PostgreSQL 오류 상태 정리 문제라 오류 처리 경로 전체를 다시 봐야 한다. 잘못 고치면 조용한 데이터 손상이 되는 종류다. |
| SEC-13 | 쓰기 시점 자동 DDL 은 **설계 결정**이다(ADR-006). 보안 수정이 아니라 설계 변경이고, 1절의 부수 효과와 함께 다뤄야 한다. |
| SEC-14 | `og_csr_build` 의 할당은 PostgreSQL 메모리 컨텍스트 밖이다. 상한을 두는 것 자체는 쉽지만 CSR 의 성능 전제를 건드리므로 측정이 먼저다. |
| SEC-18 | 감사 로그가 실패를 기록하지 않는 건 `cypher/mod.rs` 의 `audit()` → `error!()` 순서 문제다. 오류 경로 재작성이 필요하고 SEC-12 와 같은 묶음이다. |

**그리고 검증에 대해 정직하게.** 이 수정들은 `cargo check`(엔진, 경고 0),
`cargo test`(Bolt, 10/10), Studio 실기동 테스트로 확인했다. **PostgreSQL 에
실제로 설치해 돌려보지는 않았다** — 이 저장소에는 그럴 수 있는 CI 가 없고
(`.github` 부재), `tests/run.sh` 는 `^ERROR` 줄 수만 세므로 권한 회귀를 잡지
못한다. `og_grant` 의 SQL 은 타입 검사만 통과한 상태다.

---

## 5. SEC-08 상태 변경 — 레지스트리에 쓰기 RLS (스펙 014)

**상태: 미수정 → 부분.** 감사가 지적한 것은 두 가지였다: 레지스트리·인접 테이블
(`og_node`, `og_edge`, `og_adj`)에 RLS 가 없어 (a) 토폴로지가 모든 읽기 역할에게 보이고
(b) 쓰기 역할이 어떤 타입의 행이든 쓸 수 있다. **(b)만 닫혔다.**

- `engine/sql/access.sql` 이 `og_node`, `og_edge`, `og_adj`, `og_role_player` 에 RLS 를 켜고
  INSERT/UPDATE/DELETE 정책을 `og_type_priv(type_id, priv)` 로 건다 — 그 행의 타입 저장
  테이블에 해당 권한이 있는 역할만 레지스트리 행을 쓸 수 있다. 저장 테이블 권한은
  `og_grant_write` 가 주는 PostgreSQL 테이블/컬럼 GRANT 다
  ([ADR-026](../99_decisions/ADR-026-write-privileges-as-postgres-grants.md)).
- **읽기 정책은 `USING (true)` 다.** 토폴로지 노출(a, CWE-200)은 그대로다. 4절의 이유 —
  읽기 RLS 는 모든 순회 계획을 바꾼다 — 가 여전히 유효하며, 읽기 제한이 필요한 타입은 005 의
  `og_enable_rls` 로 저장 테이블을 막아야 한다. 이 경우에도 "어떤 id 가 존재하는가"는
  레지스트리로 보인다.
- 정책은 `FORCE` 되지 않는다. 소유자·슈퍼유저는 정책을 거치지 않는다(의도: 관리·벤치마크
  경로 무비용).
- 검증: `engine/tests/sql/07_write_security.sql` — `og_grant_write` 로 `Person` 만 받은
  역할이 `Doc` 타입의 `og_node` 행을 원시 SQL 로 INSERT 하면 `row-level security` 오류(42501).
  `engine/tests/sql/10_evolution_temporal_coverage.sql` — 관계 타입 쓰기 부여가 없는 역할이
  `og_edge`, `og_adj` 행을 원시 SQL 로 INSERT 하면 같은 오류(42501). `og_role_player` 정책은
  테스트가 없다.

---

## 6. 비소유자 역할로 돌려 보니 드러난 선행 결함 4건 (스펙 014)

4절 끝에서 "PostgreSQL 에 실제로 설치해 돌려보지는 않았다"고 적었다. 스펙 014 의 테스트
(`engine/tests/sql/07_write_security.sql`)는 **처음으로 비소유자 역할**(`og_t_reader`,
`og_t_writer`, `og_t_clerk`, `og_t_agent`, `og_t_runner`)로 기존 경로를 실행했고, ADR-025 의
권한 모델에서 네 개의 결함이 나왔다. `tests/run.sh` 는 `ON_ERROR_STOP` 인 이 파일의 첫 오류에서
실패하므로, 아래 단언들은 이제 회귀 게이트다.

| # | 결함 | 수정 | 검증 |
|---|---|---|---|
| 1 | `og_data` 에 CREATE 권한이 없는 역할이 **아직 생성 뷰가 없는 타입**을 읽으면 `permission denied for schema og_data`. 생성 뷰는 첫 질의가 만들고 스키마 변경마다 전부 지워지므로, 스키마 변경 직후 읽기 역할의 첫 질의는 항상 실패했다 | 권한이 없으면 SECURITY DEFINER `og__ensure_view(tid, is_edge)` 가 소유자로 만든다. 뷰는 `security_invoker` 라 누가 만들었는지가 읽기 권한을 바꾸지 않는다 (`cypher/views.rs`) | 07: `og_add_property` 로 뷰를 모두 지운 뒤 `og_t_reader` 가 `MATCH (d:Doc)` 1행 |
| 2 | 스키마 권한 없는 쓰기 역할이 **미선언 속성**을 쓰면 `og_add_property` EXECUTE 에서 트랜잭션 실패. ADR-025 는 "`__ext` 에 남는다"고 서술했지만 그렇게 동작하지 않았다(1절 부수 효과) | `security::can_change_schema()` 가 거짓이면 승격하지 않고 `__ext` 에 둔다. 검사는 승격할 것이 있을 때만 한다 (`storage/mod.rs` `declare_new_props`) | 07: `og_t_writer` 의 `CREATE (:Person {mood:"fine"})` 성공, 값 읽힘, `og_catalog.property` 에 `mood` 없음 |
| 3 | `ensure_alias_view` 가 이미 인용된 뷰 이름을 `apply_to_view` 에 넘겨 **이중 인용** — 없는 릴레이션 이름이 되어, `og_grant` 를 한 번이라도 실행한 뒤에는 **모든 `og_add_property` 가 실패**했다 | 인용하지 않은 `og_data.<name>` 을 넘긴다 (`catalog/types.rs`) | 07: `og_grant` 두 번 이후의 `og_add_property('sec','Doc','pages','int')` 성공 (09 도 `og_grant` 이후 `og_add_property` 를 실행) |
| 4 | `write` 역할이 `og_audit`, `og_history` 를 **수정·삭제**할 수 있었다 — 감사 대상이 자기 기록을 고친다 | `og_grant(…, 'write'/'admin')` 가 `og_audit`, `og_history`, `og_source`, `h_*` 의 DML 을 회수(`protect_records`), 이력 테이블은 새로 생길 때도 DML 을 주지 않는다. 기록은 SECURITY DEFINER `og__audit`, `og_capture_history`, `og__temporal` 만 쓴다 | 07: `og_t_writer` 의 `DELETE`/`UPDATE og_audit` 42501. 09: `og_t_writer` 의 `DELETE`/`INSERT og_data.h_1` 42501, 그래도 쓰기는 이력에 남음 |

**남은 것.** 감사 행은 이제 쓰기 역할이 고칠 수 없지만, `og__audit` 가 READ 목록에 있으므로
읽기 역할도 직접 불러 임의 내용의 행을 **추가**할 수 있다(`db_user` 는 검증된다). 실패한 질의의
감사 행이 질의와 함께 롤백되는 SEC-18 은 그대로다.

관련: [09_improvements_security.md](09_improvements_security.md) ·
[ADR-026](../99_decisions/ADR-026-write-privileges-as-postgres-grants.md) ·
[spec 014](../../specs/014-write-security-context/spec.md) ·
[03_rls_and_isolation.md](03_rls_and_isolation.md) ·
[08_secure_deployment.md](08_secure_deployment.md) ·
[ADR-025](../99_decisions/ADR-025-privilege-model-default-deny.md)
