# 행 수준 보안(RLS)과 격리

> ⚠️ **이 문서는 감사 커밋 `7d60c82` 시점의 스냅샷이다.** 이후 Critical 5건과
> High 8건(4건 수정 · 4건 부분)이 반영되었으므로, 여기 서술된 결함 중 일부는 **현재 코드에 더 이상
> 존재하지 않는다.** 현재 상태는 [10_fixed.md](10_fixed.md) 를 볼 것.


> **이 문서가 답하는 질문**
> - `og_enable_rls`는 실제로 어떤 SQL을 실행하는가?
> - "RLS가 순회 중간에도 적용된다"는 문서의 주장은 코드에서 성립하는가?
> - RLS가 **적용되지 않는** 경로는 어디인가?
> - 다중 그래프 / 다중 테넌트 격리에 이것을 써도 되는가?

---

## 0. 결론 먼저 (사실)

`docs/architecture.md:264`, `docs/comparison.md:180`, `docs/deep-traversal.md:13`은
"컴파일된 질의가 평범한 테이블을 읽으므로 RLS가 순회 중간에도 적용된다"고 말한다.

**감사 결과: 이 주장은 `MATCH`가 타입 테이블에 직접 닿을 때만 성립하며,
실제 컴파일 경로에서는 성립하지 않는다.** 두 가지 독립적인 이유가 있다.

1. 라벨이 붙은 `MATCH (n:Person)` 은 타입 테이블이 아니라 **생성 뷰**
   `og_data.v_<tid>` 를 읽는데, 이 뷰에 `security_invoker` 가 설정되어 있지 않다
   (`engine/src/cypher/views.rs:135`). PostgreSQL에서 `security_invoker`가 없는
   뷰는 **뷰 소유자 권한으로** 기반 테이블에 접근하며, 기반 테이블의 RLS 정책도
   뷰 소유자를 기준으로 평가된다.
2. 관계 순회는 항상 `og_data.og_adj` 를 읽는데(`engine/src/cypher/compile.rs:901`),
   이 테이블에는 RLS가 **한 번도** 활성화되지 않는다.

---

## 1. `og_enable_rls`가 실제로 하는 일 (사실)

```rust
// engine/src/interop/mod.rs:18-32
#[pg_extern]
fn og_enable_rls(graph: &str, type_name: &str, policy_expr: &str) {
    let gid = types::graph_id(graph);
    let tid = types::type_id(gid, type_name);
    for sub in labeling::og_subtypes(tid) {
        let Some(table) = types::storage_table(sub) else { continue };
        Spi::run(&format!("ALTER TABLE {table} ENABLE ROW LEVEL SECURITY"))
            .unwrap_or_else(|e| error!("failed to enable RLS on {table}: {e}"));
        Spi::run(&format!("DROP POLICY IF EXISTS og_policy ON {table}")).ok();
        Spi::run(&format!(
            "CREATE POLICY og_policy ON {table} USING ({policy_expr})"
        ))
        .unwrap_or_else(|e| error!("failed to create policy on {table}: {e}"));
    }
}
```

실행되는 것의 전부:

| 대상 | 실행되는 문장 | 비고 |
|---|---|---|
| `og_data.n_<sub>` / `e_<sub>` / `a_<sub>` (해당 타입과 **모든 하위 타입**) | `ALTER TABLE … ENABLE ROW LEVEL SECURITY` | `og_subtypes(tid)` 로 열거 |
| 같은 테이블 | `DROP POLICY IF EXISTS og_policy ON …` | 기존 `og_policy`만 제거 |
| 같은 테이블 | `CREATE POLICY og_policy ON … USING (<policy_expr>)` | 명령 종류 미지정 = `FOR ALL` |

**실행되지 않는 것 (중요)**:

| 빠진 것 | 결과 |
|---|---|
| `ALTER TABLE … FORCE ROW LEVEL SECURITY` | **테이블 소유자에게 정책이 적용되지 않는다.** 저장소 전체 `grep -i "force row level"` → 0건 |
| `og_data.og_node` / `og_edge` / `og_adj` 에 대한 RLS | 레지스트리와 인접 구조는 전면 노출 |
| `og_data.og_history` / `og_source` / `og_iri` / `og_audit` 에 대한 RLS | 과거 값·출처·감사 로그 노출 |
| 생성 뷰(`og_data.v_*`, `ve_*`, `og_data."<TypeName>"`)의 재생성/`security_invoker` 설정 | 뷰가 정책을 우회 |
| 별도 `WITH CHECK` 절 | `FOR ALL`이라 PostgreSQL이 `USING`을 `WITH CHECK`로도 쓴다 — 이 부분은 문제없음 |

또한 `policy_expr`은 원시 SQL로 보간된다(`interop/mod.rs:27-29`).
설계 의도(“a SQL boolean expression over the type table's columns”,
`interop/mod.rs:16-17`)이지만, 이 함수에 최종 사용자 입력을 넘기면 SQL 주입이다.

---

## 2. 라벨 있는 MATCH가 실제로 읽는 릴레이션 (사실)

`engine/src/cypher/compile.rs:772-775`:

```rust
let rel = match tid {
    Some(t) => views::ensure_view(t, false),
    None => "og_data.og_node".to_string(),
};
```

즉 두 갈래다.

| 패턴 | 읽는 릴레이션 | RLS 상태 |
|---|---|---|
| `MATCH (n:Person)` | `og_data.v_<tid>` (생성 뷰) | **뷰 소유자 기준으로 평가 → 우회** |
| `MATCH (n)` (라벨 없음) | `og_data.og_node` | **RLS 정책 자체가 없음** |
| `MATCH ()-[r:KNOWS]->()` | `og_data.og_adj` + (필요 시) `og_data.ve_<tid>` | **og_adj에 정책 없음** |

생성 뷰는 이렇게 만들어진다:

```rust
// engine/src/cypher/views.rs:135
Spi::run(&format!("CREATE OR REPLACE VIEW {view} AS {}", selects.join("\nUNION ALL\n")))
```

`WITH (security_invoker = true)` 가 없다. 저장소 전체 `grep security_invoker` → **0건**.

같은 문제가 사람이 읽는 별칭 뷰에도 있다:

```rust
// engine/src/catalog/types.rs:89-98
pub fn ensure_alias_view(tid: i32, name: &str, table: &str) {
    let view = alias_view_name(name);
    let _ = Spi::run(&format!("DROP VIEW IF EXISTS {view}"));
    if let Err(e) = Spi::run(&format!("CREATE VIEW {view} AS SELECT * FROM {table}")) {
```

### 2.1 왜 이것이 우회인가 (PostgreSQL 동작)

PostgreSQL에서 `security_invoker`가 꺼진(기본) 뷰는 기반 테이블 접근을
**뷰 소유자의 권한으로** 수행한다. 이때 기반 테이블의 RLS 정책도 뷰 소유자를
기준으로 평가된다. 뷰 소유자가 테이블 소유자이고 `FORCE ROW LEVEL SECURITY`가
꺼져 있으면(§1) **정책은 아예 평가되지 않는다.**

### 2.2 뷰 소유자는 누구인가 (사실)

`ensure_view`는 컴파일 시점에 **처음 그 타입을 질의한 세션의 역할**로
`CREATE OR REPLACE VIEW`를 실행한다. 따라서 소유자는 결정적이지 않다.

| 시나리오 | 뷰 소유자 | 결과 |
|---|---|---|
| 관리자가 먼저 질의 → 뷰 생성 | 관리자 | 이후 모든 저권한 호출자가 관리자 권한으로 기반 테이블 열람 |
| 저권한 사용자가 먼저 질의 | 저권한 사용자 (단 `og_data` 스키마에 `CREATE` 권한 필요) | 그 사용자 기준 평가 |
| 저권한 사용자에게 `CREATE ON SCHEMA og_data` 가 없음 | 뷰 생성 실패 → `error!("failed to build type view…")` | 라벨 있는 질의가 **동작하지 않음** |

세 번째 줄이 운영상의 함정이다: RLS로 격리하려면 저권한 역할에게
`og_data`의 `CREATE` 권한을 주어야 하는데, 그것을 주면 그 역할이 뷰를 소유하고
스키마에 임의 객체를 만들 수 있다.

### 2.3 뷰가 언제 사라지는가

`labeling::bump_schema_version()` 이 `views::drop_all_views()` 를 호출한다
(`engine/src/catalog/labeling.rs:172-182`). 즉 스키마 변경이 있을 때마다 뷰가
전부 삭제되고 다음 질의자가 다시 만든다 — **소유자가 바뀔 수 있다**는 뜻이다.

---

## 3. RLS가 적용되지 않는 경로 — 전수 표 (사실)

| # | 경로 | 읽는 릴레이션 | 근거 | 무엇이 새는가 |
|---|---|---|---|---|
| B1 | `MATCH (n:Label)` | `og_data.v_<tid>` (비-invoker 뷰) | `compile.rs:773`, `views.rs:135` | 정책이 숨긴 행의 **모든 컬럼** |
| B2 | `MATCH (n)` (라벨 없음) | `og_data.og_node` | `compile.rs:735, 774` | 노드 id·type_id 전량 |
| B3 | 모든 관계 홉 | `og_data.og_adj` | `compile.rs:901` | **토폴로지 전량** (src/dst/eid) |
| B4 | 가변 길이 홉 `[*1..k]` | `og_vlp` / `og_reach` → `og_data.og_adj` | `compile.rs:868-877`, `access.sql:138-187`, `traverse.rs:94-97` | 도달 가능성·경로 |
| B5 | `og_expand` / `og_expand_batch` | `og_data.og_adj` | `access.sql:14-37` | 이웃 목록 |
| B6 | `og_csr_build` / `og_csr_reach` / `og_csr_hops` | `og_data.og_adj` → 백엔드 로컬 배열 | `traverse.rs:241-292, 359, 442` | 토폴로지 전량 (**문서화된 의도**, `traverse.rs:19-23`) |
| B7 | `og_history(id)` / `og_as_of(id, ts)` | `og_data.og_history.payload` | `agent/mod.rs:471-526`, `bootstrap.sql:310-322` | **정책이 숨긴 행의 과거 전체 컬럼값** |
| B8 | `og_node_json(id)` / `og_edge_json(id)` | `t.storage_table` 을 `EXECUTE format('… FROM %s …')` | `access.sql:220, 249` | 테이블 직접 접근이므로 RLS는 적용됨(호출자 권한). 단 §5의 2차 주입 위험 |
| B9 | `og_node_view` / `og_edge_view` / `og_type_view` 등 | `og_data.og_node`·`og_edge` 조인 (비-invoker 뷰) | `access.sql:81-126` | id·타입명·src·dst |
| B10 | `og_graph_stats` / `og_degree_distribution` / `og_estimate` | 집계·플래너 통계 | `storage/stats.rs`, `agent/mod.rs:350-397` | 카디널리티(측면 채널) |
| B11 | `og_schema(graph)` | `og_catalog.*` + 인스턴스 카운트 | `agent/mod.rs:35-45` | 타입별 인스턴스 수 |
| B12 | `og_typeql_attribute` / `og_typeql_role` 뷰 | `og_data.og_edge`·`og_role_player` | `access.sql:307-338` | TypeQL 소유 관계 |
| B13 | 감사 로그 | `og_data.og_audit` | `bootstrap.sql:380-390` | 다른 주체의 질의 원문 |

**B7이 특히 중요하다.** `og_enable_history`(`agent/mod.rs:448-468`)는 타입 테이블에
`AFTER INSERT OR UPDATE OR DELETE` 트리거를 붙이고, `og_capture_history()`가
`to_jsonb(NEW)` 전체를 `og_data.og_history.payload` 에 넣는다(`access.sql:280-292`).
`og_history`는 RLS 없는 그 테이블을 직접 읽는다. 즉 **RLS로 가린 값이 히스토리에
평문으로 복제되어 아무 제약 없이 조회된다.**

---

## 4. 다중 그래프 격리 (사실)

`og_catalog.graph`가 여러 그래프를 담고, 타입은 `graph_id`로 구분된다
(`bootstrap.sql:35-46`). 그러나:

| 사실 | 근거 |
|---|---|
| 데이터 테이블은 그래프가 아니라 **타입**별로 나뉜다(`n_<type_id>`) | `catalog/types.rs:68-74` |
| `og_data.og_node` / `og_edge` / `og_adj` 는 **모든 그래프가 공유**한다 | `bootstrap.sql:197-241` — `graph_id` 컬럼이 없다 |
| 식별자에 `type_id`가 인코딩되어 그래프를 간접 식별할 수 있다 | `engine/src/id.rs`, `compile.rs`의 `og_id_type` |
| 그래프 이름은 `og_cypher`의 첫 인자일 뿐 권한 경계가 아니다 | `cypher/mod.rs:83-88` |

**판정: 그래프는 네임스페이스이지 보안 경계가 아니다.** 두 테넌트를 두 그래프로
나누는 것만으로는 격리되지 않는다. `og_data.og_adj` 한 테이블만 읽을 수 있어도
모든 그래프의 토폴로지가 보인다.

---

## 5. 2차 위험 — 카탈로그가 동적 SQL의 원천 (사실)

`og_catalog.type.storage_table`, `og_catalog.property.column_name`,
`og_catalog.property.data_type` 은 전부 제약 없는 `text` 컬럼이며
(`bootstrap.sql:41, 91-92`), 그 값이 그대로 SQL 텍스트로 보간된다.

| 소비 지점 | 코드 |
|---|---|
| `EXECUTE format('SELECT to_jsonb(x) FROM %s x WHERE x.id = $1', t.storage_table)` | `engine/sql/access.sql:220`, `:249` — **`%I`가 아니라 `%s`** |
| `format!("({param}->>{lit})::{dtype}")` | `engine/src/storage/mod.rs:216` |
| `ALTER TABLE {child_table} ADD COLUMN IF NOT EXISTS {col} {dtype}` | `engine/src/catalog/types.rs:482` |
| `NULL::{dt} AS {col}` (뷰 본문) | `engine/src/cypher/views.rs:114` |

따라서 **`og_catalog` 쓰기 권한은 사실상 SQL 실행 권한이다.** RLS 설계에서 이
테이블들의 권한을 반드시 분리해야 한다.

---

## 6. 격리가 실제로 성립하는 구성 (결정)

아래 조건을 **모두** 만족할 때에만 RLS 기반 격리를 신뢰할 수 있다.
현재 코드로는 1·2를 수동으로 보정해야 한다.

| # | 조건 | 왜 필요한가 | 현재 자동화 여부 |
|---|---|---|---|
| 1 | 모든 `og_data.v_*`, `ve_*`, `og_data."<Type>"` 뷰에 `ALTER VIEW … SET (security_invoker = true)` | B1 차단 | **없음 (수동)** |
| 2 | 모든 타입 테이블에 `ALTER TABLE … FORCE ROW LEVEL SECURITY` | 소유자 우회 차단 | **없음 (수동)** |
| 3 | `og_data.og_node` / `og_edge` / `og_adj` 에 RLS 정책 | B2·B3·B4·B5 차단 | **없음 — 정책을 걸 컬럼도 없다** |
| 4 | `og_data.og_history` 에 RLS 정책 또는 히스토리 미사용 | B7 차단 | **없음** |
| 5 | 애플리케이션 역할에서 `og_csr_*` `EXECUTE` 회수 | B6 차단 | **없음** |
| 6 | 애플리케이션 역할이 테이블 소유자가 아님 | 2와 함께 필요 | 배포 책임 |
| 7 | `og_catalog.*` 는 읽기 전용 부여 | §5 차단 | 배포 책임 |

3번은 스키마 변경 없이는 완결할 수 없다: `og_data.og_adj` 에는 테넌트를 식별할
컬럼이 없다(`bootstrap.sql:197-206`). 즉 **현재 스키마에서 토폴로지 기밀성은
RLS로 달성할 수 없다.** 이는 SEC-08의 근거이며, 개선안은
[`09_improvements_security.md`](09_improvements_security.md)에 있다.

---

## Forbidden (금지)

- **`og_enable_rls`만 호출하고 "격리되었다"고 결론짓지 말 것.**
  §6의 7개 조건을 확인하지 않았다면 격리는 성립하지 않는다.
- **`og_enable_rls`의 `policy_expr`에 최종 사용자 입력을 전달하지 말 것**
  (`interop/mod.rs:27-29` 원시 보간).
- **RLS를 쓰는 배포에서 `og_enable_history`를 쓰지 말 것.** 히스토리 테이블이
  정책 밖에서 같은 값을 보관한다(B7).
- **RLS를 쓰는 배포에서 애플리케이션 역할에 `og_csr_build`/`og_csr_reach`/
  `og_csr_hops` 실행 권한을 남겨두지 말 것** (`traverse.rs:19-23`이 명시적으로
  "RLS is never consulted"라고 적고 있다).
- **`og_data.og_adj` 에 대한 `SELECT` 권한을 다중 테넌트 애플리케이션 역할에
  부여하지 말 것.** 다만 이 권한 없이는 순회가 불가능하므로, 현재 아키텍처에서
  이는 "다중 테넌트를 같은 데이터베이스에 두지 말 것"과 동의어다.
- **그래프(`graph` 인자) 분리를 보안 경계로 쓰지 말 것.**

## Required (필수)

- `og_enable_rls` 호출 직후 반드시 다음을 수행할 것:
  `ALTER TABLE og_data.n_<tid> FORCE ROW LEVEL SECURITY;`
  (하위 타입 전부에 대해).
- 스키마가 변경될 때마다(뷰가 `drop_all_views()`로 재생성될 때마다) 뷰의
  `security_invoker` 를 다시 설정할 것. 구체 스크립트는
  [`08_secure_deployment.md`](08_secure_deployment.md) §4.
- 다중 테넌트가 요구되면 **데이터베이스 또는 클러스터 단위로 분리할 것.**
  현재 스키마에서 RLS 단독 격리는 §6-3에 의해 불가능하다.
- `views.rs` 또는 `interop/mod.rs`를 수정하면 §3 표 B1~B13을 재검증할 것.

<!-- affects: security, backend, data, ops -->
<!-- requires-update: 07_security/08_secure_deployment.md, 07_security/09_improvements_security.md, 07_security/07_audit_and_history.md -->
