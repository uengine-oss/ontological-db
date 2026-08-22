# 시스템 개관 — 논리 아키텍처와 물리 아키텍처

> **이 문서가 답하는 질문**
> - 논리적으로 이 시스템은 몇 개의 계층으로 되어 있는가?
> - 물리적으로는 몇 개의 프로세스이고, 무엇이 어디서 도는가?
> - 논리와 물리가 어긋나는 지점은 어디인가? (그리고 그것이 왜 중요한가)

---

## 1. 논리 아키텍처

**논리 아키텍처는 "무엇이 무엇에 의존하는가"의 그림이다.** 프로세스 경계와 무관하다.

```
┌───────────────────────────────────────────────────────────────┐
│ L5  진입면 (protocol surfaces)                                 │
│     SQL 함수 · Bolt 4.4 · PostgREST RPC · Studio HTTP          │
├───────────────────────────────────────────────────────────────┤
│ L4  언어 프런트엔드                                             │
│     Cypher (spec 003)          TypeQL (spec 010)               │
│     lexer→parser→AST→compile   lexer→parser→AST→compile        │
│     ※ 공통 IR 없음 — 두 파이프라인이 독립적으로 SQL을 만든다      │
├───────────────────────────────────────────────────────────────┤
│ L3  어댑터 (adapters at the edge — 헌법 VI)                     │
│     RDF/OWL (006) · Neo4j 호환면 (compat/) · 에이전트 표면 (008) │
├───────────────────────────────────────────────────────────────┤
│ L2  카탈로그 & 벡터                                             │
│     타입 시스템 · 구간 라벨 · role/제약 (002)                    │
│     임베딩 선언 · HNSW · 하이브리드 RRF (004)                    │
├───────────────────────────────────────────────────────────────┤
│ L1  저장 계층 (001)                                             │
│     쓰기: Rust SPI (storage/)   읽기: 컴파일된 SQL이 직접        │
│     og_adj CSR 세그먼트 · og_node/og_edge 레지스트리 · 타입 테이블│
├───────────────────────────────────────────────────────────────┤
│ L0  PostgreSQL 16                                              │
│     힙 · MVCC · WAL · 플래너/실행기 · RLS · pgvector             │
└───────────────────────────────────────────────────────────────┘
```

### 각 계층이 보장하는 것

| 계층 | 보장 | 근거 |
|---|---|---|
| L0 | ACID, 크래시 복구, `pg_dump`, 논리복제, RLS | 헌법 IX. 모든 구조가 평범한 힙 릴레이션 ([`bootstrap.sql:8-10`](../../engine/sql/bootstrap.sql)) |
| L1 | 홉이 순차 배열 읽기. 쓰기 시 세 구조가 한 트랜잭션 | [`storage/mod.rs:1-10`](../../engine/src/storage/mod.rs) |
| L2 | 서브타입 판정이 인덱스 범위 비교 1회 | [`labeling.rs:1-16`](../../engine/src/catalog/labeling.rs) |
| L3 | 코어 의미론을 바꾸지 않음 | 헌법 VI, [`compat/mod.rs:9-14`](../../engine/src/compat/mod.rs) |
| L4 | 옵티마이저가 그래프 패턴 전체를 봄 | [`compile.rs:1-10`](../../engine/src/cypher/compile.rs) |
| L5 | 두 번째 인증·감사 경로를 만들지 않음 | spec 011 설계 결정 2 |

### 논리 의존 방향 (Facts)

`grep` 으로 확인한 실제 의존 관계:

```
cypher/  ──uses──> catalog/, storage/, compat/, id, spiu, stats
typeql/  ──uses──> catalog/, storage/, id, spiu        (cypher/ 를 쓰지 않음)
compat/  ──uses──> cypher/{ast, eval, compile, views}, catalog/, vector 함수
vector/  ──uses──> catalog/
interop/ ──uses──> catalog/
agent/   ──uses──> catalog/, cypher::compile_for_diagnostics
```

- `crate::typeql` 를 참조하는 `cypher/` 코드가 **없고**,
  `crate::cypher` 를 참조하는 `typeql/` 코드도 **없다**.
  → 두 프런트엔드가 공유하는 중간 표현(IR)이 존재하지 않는다.
  자세한 분석은 [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-05**.
- **`compat/` 는 코어를 향해 위로 의존하지 않고, 코어가 `compat/` 를 향해 아래로 의존한다.**
  `cypher/compile.rs:421` 이 `crate::compat::procs` 를,
  `cypher/mod.rs:165` 가 `crate::compat::ddl::run` 을 부른다.
  → "어댑터는 엣지에" 라는 원칙 VI가 코드 의존 방향에서는 지켜지지 않았다.
  [`08_improvements_architecture.md`](08_improvements_architecture.md) **ARCH-06**.

---

## 2. 물리 아키텍처

**물리 아키텍처는 "무엇이 어느 프로세스에서 도는가"의 그림이다.**

```
┌─────────────────────────────────────────────────────────────────────┐
│ 호스트                                                                │
│                                                                     │
│  ┌───────────────────────┐   ┌──────────────────────┐               │
│  │ postgres (백엔드 N개) │   │ ontological-bolt     │               │
│  │  ─────────────────    │   │  (별도 Rust 바이너리)│               │
│  │  ontological.so       │◄──┤  스레드/연결 1:1     │◄── Neo4j 드라이버│
│  │   ├ cypher 컴파일러   │   │  세션당 PG 연결 1개  │    (bolt://)    │
│  │   ├ typeql 컴파일러   │   │  NoTls               │               │
│  │   ├ storage 쓰기경로  │   └──────────────────────┘               │
│  │   ├ thread_local:     │                                          │
│  │   │   PLAN_CACHE      │   ┌──────────────────────┐               │
│  │   │   CSR (og_csr_*)  │◄──┤ portal (Node.js)     │◄── 브라우저    │
│  │   │   stats 카운터    │   │  pg Pool max=8       │    (HTTP 7474) │
│  │   └ ureq (genai)──────┼──►│  고정 역할          │               │
│  │  pgvector.so          │   └──────────────────────┘               │
│  └───────────┬───────────┘                                          │
│              │                    ┌──────────────────────┐          │
│              │                    │ PostgREST (선택)     │◄── REST   │
│              │                    │  og_cypher_json RPC  │          │
│              ▼                    └──────────────────────┘          │
│      [ 힙 파일 · WAL ]                                               │
└──────────────┬──────────────────────────────────────────────────────┘
               │ 스트리밍 복제
               ▼
        [ 읽기 대기 서버 ]   ※ ARCH-03 참조
                                          외부: 임베딩 엔드포인트 (genai)
```

### 프로세스 목록 (Facts)

| 프로세스 | 필수? | 언어/런타임 | 무엇 | 근거 |
|---|---|---|---|---|
| `postgres` 백엔드 | ✅ 필수 | C + `ontological.so` (Rust cdylib) | 전부 | [`engine/Cargo.toml`](../../engine/Cargo.toml) `crate-type = ["cdylib"]` |
| `ontological-bolt` | 선택 | Rust (pgrx 아님) | Bolt 4.4 종단 | [`bolt/src/main.rs`](../../bolt/src/main.rs) |
| `portal` Studio | 선택 | Node.js | 콘솔 + 벤치 리포트 | [`portal/server/index.js`](../../portal/server/index.js) |
| PostgREST | 선택 | (외부) | `og_cypher_json` RPC 노출 | [`interop/mod.rs:34-54`](../../engine/src/interop/mod.rs) |
| 임베딩 엔드포인트 | 선택 | (외부) | `genai.vector.encode` 대상 | [`compat/genai.rs`](../../engine/src/compat/genai.rs) |

**`ontological-bolt` 를 멈춰도 `og_cypher()` 호출자는 영향이 없다.**
게이트웨이는 PostgreSQL의 평범한 클라이언트일 뿐이다 ([`bolt/README.md`](../../bolt/README.md)).

### 백엔드 로컬 상태 (물리적으로 중요)

PostgreSQL 백엔드는 프로세스이고, 아래 세 가지가 **그 프로세스 안에만** 산다:

| 상태 | 위치 | 수명 | 위험 |
|---|---|---|---|
| `PLAN_CACHE` | `thread_local!` HashMap, 512개 상한 | 연결 수명 | 스키마 변경 시 무효화되지 않음 → **ARCH-02** |
| 컴파일된 CSR | `thread_local!` `Option<Csr>` | 연결 수명 | 스냅샷 동결, RLS 미참조. 백엔드마다 8.4~9.2 MiB, 119~229 ms 재빌드 |
| 쓰기 카운터 | `thread_local!` `Cell<i64>` × 10 | `og_cypher()` 호출마다 리셋 | 롤백해도 카운트가 남음 (명시적으로 문서화됨) |

근거: [`cypher/mod.rs:26-31`](../../engine/src/cypher/mod.rs),
[`storage/traverse.rs:205-210`](../../engine/src/storage/traverse.rs),
[`stats.rs:9-13`](../../engine/src/stats.rs),
[`docs/deep-traversal.md`](../deep-traversal.md).

---

## 3. 논리 ↔ 물리가 어긋나는 지점

이 절이 이 문서의 핵심이다. **논리적으로 하나인 것이 물리적으로 여럿이거나 그 반대인 곳**이
운영·정합성 문제가 생기는 곳이다.

### (a) 하나의 질의 경로 — 여러 프로세스

논리적으로 Cypher 실행 경로는 하나다 (spec 003).
물리적으로는 진입점이 최소 넷이다: psql, Bolt 게이트웨이, Studio HTTP, PostgREST.

Bolt 게이트웨이는 의도적으로 **의미론을 갖지 않지만**, 물리적 경계 자체가 다음을 만든다:
- 게이트웨이 ↔ PostgreSQL 사이가 **평문**(`NoTls`, [`bolt/src/session.rs:182`](../../bolt/src/session.rs))
- Bolt `RUN` 하나가 **PostgreSQL 왕복 2회**
  (`og_cypher_columns` + `og_cypher`, [`session.rs:283-299`](../../bolt/src/session.rs))
- 결과 전체를 게이트웨이 메모리에 **먼저 다 모은 뒤** `PULL n` 을 서빙
  ([`session.rs:291-322`](../../bolt/src/session.rs)) → 스트리밍이 아니다

→ [`06_protocol_surfaces.md`](06_protocol_surfaces.md), **ARCH-07**

### (b) 하나의 그래프 — 두 개의 컴파일러

논리적으로 Cypher와 TypeQL은 "같은 그래프 위의 두 언어"다
([`typeql/compile.rs:1-13`](../../engine/src/typeql/compile.rs)).
물리적으로도 같은 테이블을 읽고 쓴다.

그러나 **컴파일러가 두 벌이고 공유하는 IR이 없다.**
`Bind` 열거형, `Compiler` 구조체, `fresh()` 별칭 생성기, `from`/`wheres` 누산기가
각 모듈에 독립적으로 존재한다
([`cypher/compile.rs:102-128`](../../engine/src/cypher/compile.rs) vs
[`typeql/compile.rs:23-55`](../../engine/src/typeql/compile.rs)).
→ 한쪽의 최적화(예: `og_reach` 재작성)가 다른 쪽에 자동으로 적용되지 않는다. **ARCH-05**

### (c) 하나의 저장소 — 두 개의 접근 규율

논리적으로 저장 계층은 하나다.
물리적으로는 **쓰기는 Rust SPI, 읽기는 컴파일된 SQL** 로 이원화되어 있고,
이것은 의도된 설계다 ([`storage/mod.rs:1-10`](../../engine/src/storage/mod.rs)).

대가: 스토리지 불변식(레지스트리 ↔ 타입 테이블 ↔ 양방향 인접)이
쓰기 경로 Rust 코드에만 존재하고, 읽기 경로 SQL은 그것을 가정한다.
불변식이 깨지면 읽기가 조용히 틀린 답을 낸다.
`og_check_integrity()` 가 이를 사후에 검사한다
([`storage/stats.rs:172`](../../engine/src/storage/stats.rs)). → **ARCH-08**

### (d) "확장 하나" — 실제로는 확장 + 게이트웨이 + 콘솔

README의 서사는 "`CREATE EXTENSION` 한 줄"이다. 이것은 **코어에 대해 사실**이다.
Bolt와 Studio는 별도 배포물이며, 이 구분을 흐리지 않는 것이 중요하다.
운영 문서(`08_operations/`)는 **세 개의 배포 단위**를 전제해야 한다.

---

## Decisions

| # | 결정 | 이유 | 근거 |
|---|---|---|---|
| D-1 | 커널 패치 없는 확장으로만 존재한다 | PostgreSQL 생태계 상속이 유일한 비대칭 우위 | 헌법 원칙 I (NON-NEGOTIABLE) |
| D-2 | 모든 그래프 구조는 평범한 힙 릴레이션이다 | MVCC/WAL/vacuum/`pg_dump` 를 공짜로 얻는다 | [`bootstrap.sql:8-10`](../../engine/sql/bootstrap.sql) |
| D-3 | Bolt 게이트웨이는 배경 워커가 아니라 별도 프로세스 | SPI가 스레드 안전하지 않아 배경 워커는 세션 간 질의를 직렬화한다 | spec 011 plan.md 설계 결정 1 |
| D-4 | Bolt는 자체 인증 저장소를 만들지 않는다 | 권한/RLS/감사가 하나로 유지된다 | spec 011 plan.md 설계 결정 2 |
| D-5 | 읽기는 SQL 한 문장, 쓰기는 절차적 실행 | 001 FR-012가 세 구조의 동시 갱신을 요구. 정확성 > 단일 문장 | spec 003 plan.md 설계 결정 3 |
| D-6 | 백엔드 로컬 CSR은 자동 라우팅하지 않는다 | 스냅샷 동결 + RLS 미참조를 기본값으로 두지 않는다 | [`traverse.rs:19-25`](../../engine/src/storage/traverse.rs) |

---

## Facts

- 필수 프로세스는 `postgres` 하나다. 나머지는 전부 선택이다.
- 확장 의존성은 pgvector 하나 (`requires = 'vector'`, [`ontological.control`](../../engine/ontological.control)).
- 확장 버전은 `0.1.0` 고정이고 `engine/sql/` 에 업그레이드 스크립트가 없다.
  (`bootstrap.sql`, `access.sql` 두 파일뿐) → **ARCH-10**
- 유일한 아웃바운드 네트워크는 `compat/genai.rs` 의 `ureq` 호출이며 기본 비활성이다.
- 릴리스 프로파일은 `panic = "unwind"` 로 설정되어 있다
  ([`engine/Cargo.toml`](../../engine/Cargo.toml)) — 확장 안 panic이 서버를 죽이지 않아야 한다는
  헌법 기술 제약과 정합한다.

---

## Forbidden / Required

**Forbidden**
- 그래프 데이터를 위한 **별도 백업/복제 경로**를 만들지 말 것 (헌법 원칙 I 안티패턴).
- 어떤 기능도 **superuser 전용 커널 기능**에 의존하지 말 것.
  의존한다면 optional 가속 경로여야 하고 fallback이 있어야 한다 (헌법 원칙 I).
- 백엔드 로컬 상태(PLAN_CACHE, CSR, 카운터)를 **진실의 원천**으로 삼지 말 것.
  캐시 계층은 허용하되 캐시가 진실이 될 수 없다 (헌법 원칙 IX).
- Bolt 게이트웨이나 Studio에 코어 의미론을 넣지 말 것 (헌법 원칙 VI).

**Required**
- 새 프로세스를 추가하면 이 문서의 물리 아키텍처 표와 `08_operations/` 를 함께 갱신할 것.
- 새 백엔드 로컬 상태를 도입하면 **수명과 무효화 조건**을 명시할 것.
- 헌법 원칙을 이탈하는 설계는 해당 `plan.md` 의 Complexity Tracking에 기록할 것.
  기록 없는 이탈은 반려된다.

<!-- affects: architecture, backend, operations, api -->
<!-- requires-update: 01_architecture/02_layer_boundaries.md, 01_architecture/06_protocol_surfaces.md, 08_operations/, 99_decisions/ -->
