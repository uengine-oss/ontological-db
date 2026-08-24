# 실패 격리 — 무엇이 무엇을 무너뜨릴 수 있는가

> **이 문서가 답하는 질문**
> - 어떤 컴포넌트가 죽으면 무엇이 같이 죽는가?
> - 동기 경계와 비동기 경계는 어디인가?
> - 외부 네트워크(genai) 의존이 데이터베이스에 어떻게 번지는가?
> - 트랜잭션 경계 밖에 남는 상태는 무엇인가?

---

## 1. 실패 도메인

```
┌── 도메인 A: PostgreSQL 백엔드 프로세스 ─────────────────────────┐
│  Cypher/TypeQL 컴파일 · 실행 · 쓰기 경로 · 카탈로그            │
│  백엔드 로컬: PLAN_CACHE, 컴파일된 CSR, 쓰기 카운터            │
│  외부 의존: genai HTTP (동기, 블로킹) ← 도메인 D               │
│  실패 반경: 이 연결 하나. 다른 백엔드는 무사                    │
│  단, 백엔드가 OOM-kill되면 postmaster가 전체를 재시작한다        │
└──────────────────────────────────────────────────────────────┘
┌── 도메인 B: ontological-bolt 프로세스 ──────────────────────────┐
│  실패 반경: Bolt 클라이언트 전부. PostgreSQL은 무사             │
│  스레드 하나가 panic해도 다른 세션은 계속 (thread::spawn)       │
└──────────────────────────────────────────────────────────────┘
┌── 도메인 C: portal (Studio) 프로세스 ───────────────────────────┐
│  실패 반경: 브라우저 콘솔. 나머지 전부 무사                     │
└──────────────────────────────────────────────────────────────┘
┌── 도메인 D: 임베딩 엔드포인트 (외부 HTTP) ───────────────────────┐
│  실패 반경: genai.vector.encode 를 부른 질의. 타임아웃으로 제한  │
└──────────────────────────────────────────────────────────────┘
```

---

## 2. 실패 전파 매트릭스 (Facts)

| 죽는 것 | SQL 함수 표면 | Bolt 클라이언트 | Studio | 쓰기 정합성 |
|---|---|---|---|---|
| `ontological-bolt` | ✅ 무사 | ❌ 전부 끊김 | ✅ 무사 | ✅ 무사 |
| `portal` | ✅ 무사 | ✅ 무사 | ❌ 끊김 | ✅ 무사 |
| 임베딩 엔드포인트 | ⚠️ genai 질의만 실패 | ⚠️ 동일 | ⚠️ 동일 | ✅ 무사 |
| PostgreSQL 백엔드 1개 | ⚠️ 그 연결만 | ⚠️ 그 세션만 | ⚠️ 그 요청만 | ✅ 트랜잭션 롤백 |
| PostgreSQL 인스턴스 | ❌ 전부 | ❌ 전부 | ❌ 전부 | ✅ WAL 복구 |

**근거**: `bolt/README.md` — *"Nothing on the PostgreSQL path depends on it:
stop the gateway and every `og_cypher()` caller is unaffected."*

---

## 3. 동기 / 비동기 경계

### 이 시스템에 비동기 경계는 (거의) 없다

헌법 원칙 IX가 그것을 금지한다:

> "성능을 위해 인접 리스트만 비동기로 갱신" 같은 최종적 일관성 지름길은 금지한다.
> 캐시 계층은 허용하되, 캐시는 진실의 원천이 될 수 없다.

**결과**: 노드/엣지/인접/카탈로그/인덱스 변경이 전부 호출자 트랜잭션 안에서 동기적으로 일어난다.

```rust
// engine/src/storage/mod.rs:429-446 — 한 엣지 생성의 전부, 한 트랜잭션 안
Spi::run_with_args("INSERT INTO og_data.og_edge …")   // 레지스트리
Spi::run_with_args(&sql, …)                            // 타입 테이블
adjacency::append(src, tid, 'o', dst, eid);            // 정방향 인접
adjacency::append(dst, tid, 'i', src, eid);            // 역방향 인접
```

### 유일한 진짜 비동기 경계: genai HTTP

```rust
let mut request = ureq::post(&endpoint).timeout(Duration::from_millis(timeout));
let reply: Value = match request.send_json(...) { … }
```
— [`engine/src/compat/genai.rs:139-149`](../../engine/src/compat/genai.rs)

**동기 블로킹 호출**이다. 비동기 런타임을 쓰지 않는 이유는 명시되어 있다:
PostgreSQL 백엔드는 이미 기다리는 스레드이므로, 비async 스택이 얻는 게 없고
스케줄러 비용만 든다 ([`engine/Cargo.toml`](../../engine/Cargo.toml) 주석).

**즉, 임베딩 엔드포인트가 느리면 그 PostgreSQL 백엔드가 그만큼 멈춘다.**

### 그 위험에 대한 세 겹 방어 (Decisions)

```
1. 기본 비활성   genai.enabled 가 'on' 이 아니면 오류로 거부
2. 엔드포인트는 설정, 인자 아님
                 Neo4j는 호출이 자기 엔드포인트를 지정하게 한다. 여기서는 불가.
                 URL은 og_catalog.setting 의 genai.endpoint 에서 온다.
                 → "질의 권한이 곧 fetch 권한이 아니다" (SSRF 방어)
3. 시간 제한     genai.timeout_ms, 기본 5,000ms
```
— [`engine/src/compat/genai.rs:13-35, 101-137`](../../engine/src/compat/genai.rs)

**오류 메시지가 교정 가능하다** (헌법 원칙 VIII):
```
genai.vector.encode is disabled. It makes an outbound HTTP request from the
database, so it is off until that is chosen deliberately:
SELECT og_set_setting('genai.enabled', 'on')
```

### 남은 위험 (Facts)

| 위험 | 상태 |
|---|---|
| 5초 타임아웃 × 다수 동시 질의 → 백엔드 고갈 | 완화 장치 없음. `statement_timeout` 으로 운영 측 방어 필요 |
| 재시도 | 없음 — 한 번 실패하면 `error!` |
| 서킷 브레이커 | 없음 |
| 토큰(`genai.token`)이 `og_catalog.setting` 평문 저장 | ✅ 사실. `07_security/` 담당 항목 |
| 제공자 화이트리스트 | ✅ `ollama` / `openai` / `azureopenai` 만 허용 |

---

## 4. 트랜잭션 경계 밖에 남는 상태

이 절이 실무에서 가장 자주 물리는 곳이다.

| 상태 | 트랜잭션 롤백 시 | 근거 |
|---|---|---|
| `og_data.*` 데이터 | ✅ 롤백됨 | 평범한 힙 릴레이션 |
| `og_catalog.*` 카탈로그 | ✅ 롤백됨 (DDL도 트랜잭션 안) | 헌법 원칙 IX |
| 타입 스토리지 테이블 DDL | ✅ 롤백됨 (PostgreSQL은 트랜잭션 DDL) | — |
| **`PLAN_CACHE`** | ❌ **남는다** | `thread_local!`, [`cypher/mod.rs:26-31`](../../engine/src/cypher/mod.rs) |
| **쓰기 카운터** | ❌ **남는다** (명시적으로 문서화됨) | [`stats.rs:9-13`](../../engine/src/stats.rs) |
| **컴파일된 CSR** | ❌ 남는다 (의도적) | [`traverse.rs:205-210`](../../engine/src/storage/traverse.rs) |
| `og_data.og_audit` | ✅ 롤백됨 (같은 트랜잭션 INSERT) | [`cypher/mod.rs:122-135`](../../engine/src/cypher/mod.rs) |

### 쓰기 카운터의 명시적 한계

```rust
//! The state is per-backend and reset at the start of every `og_cypher()` call.
//! That is sound because a PostgreSQL backend serves one connection and runs one
//! statement at a time: there is no second writer to interleave with. It is
//! *not* a transaction log — a rolled-back statement leaves its counts behind,
//! and the next call clears them.
```
— [`engine/src/stats.rs:9-13`](../../engine/src/stats.rs)

Bolt 게이트웨이는 이 한계를 알고 **쓰기 직후, 요약을 쓰기 직전에, 같은 연결에서** 물어본다 —
답이 의미 있는 유일한 창이다 ([`cypher/mod.rs:111-116`](../../engine/src/cypher/mod.rs)).

### `PLAN_CACHE` 가 롤백되지 않는 것의 결과

```
tx1: BEGIN; SELECT og_cypher('g', 'MATCH (n:A) RETURN n');   -- 컴파일, 캐시 저장
     SELECT og_add_property('g','A','x','int4');             -- drop_all_views()
     ROLLBACK;
     -- 뷰는 트랜잭션 DDL이라 복원된다.
     -- PLAN_CACHE 의 항목도 남아 있다. 이 경우는 우연히 일관적이다.

tx2: BEGIN; SELECT og_add_property('g','A','x','int4');      -- drop_all_views()
     COMMIT;
     SELECT og_cypher('g', 'MATCH (n:A) RETURN n');
     -- 캐시 히트 → ensure_view() 를 건너뛴다
     -- → 폐기된 og_data.v_<tid> 를 참조하는 SQL을 실행
```

두 번째 경우가 **ARCH-02** 다. 자세한 분석은
[`08_improvements_architecture.md`](08_improvements_architecture.md).

---

## 5. 오류 처리 전략 — panic vs ereport

### 헌법이 요구하는 것

> 어느 쪽이든 PostgreSQL 메모리 컨텍스트·에러 처리(elog/ereport) 규약을 따른다.
> **확장 안에서의 panic/unwind가 서버를 죽여서는 안 된다.**
> — 헌법 기술 제약, 구현 언어

### 현재 상태 (Facts)

`engine/Cargo.toml` 이 dev/release 양쪽에 `panic = "unwind"` 를 설정한다 —
pgrx가 panic을 잡아 `ereport(ERROR)` 로 바꿀 수 있게 하는 필수 설정이다.

그러나 **오류를 내는 방식이 세 갈래로 섞여 있다**:

| 방식 | 개수(대략) | 의미 | 사용자가 보는 것 |
|---|---:|---|---|
| `error!("…")` | 115 | pgrx가 `ereport(ERROR)` 로 변환 | 의도된 메시지 |
| `.expect("…")` | 111 | Rust panic → pgrx가 잡아 오류로 | panic 메시지 + 백트레이스 뉘앙스 |
| `.unwrap()` | 95 | Rust panic, 메시지 없음 | 정보 없는 실패 |

(개수는 `grep -c` 실측 합계. 파일별 분포는
[`../00_overview/04_repository_map.md`](../00_overview/04_repository_map.md) 참조.)

**설계 의도는 명확한 곳이 있다.** `id.rs` 는 주석으로 밝힌다:
```rust
/// Compose an identifier. Panics (→ `ereport(ERROR)` via pgrx) on overflow so a
/// silently truncated id can never reach storage.
```
— [`engine/src/id.rs:31-32`](../../engine/src/id.rs)

**그러나 일관되지 않다.** 예:
- `catalog/types.rs` 에 `.expect(` 31개, `error!(` 18개 — 같은 파일에서 두 방식 혼용.
- `storage/stats.rs` 는 `.expect(` 10개 · `.unwrap()` 10개인데 `error!(` 는 0개.
- `catalog/labeling.rs:207` 등은 `rows.map(|r| r.get::<i32>(1).unwrap().unwrap())` —
  이중 unwrap.

헌법 원칙 VIII은 "오류 메시지는 결정적이고 교정 가능해야" 한다고 요구한다.
`.unwrap()` 의 panic 메시지는 그 요구를 만족하지 않는다.
→ **ARCH-17**

### 오류가 잘 설계된 곳 (참고할 것)

```rust
error!("type '{name}' does not exist. did you mean: {}", hint.join(", "));
```
— [`catalog/types.rs:135`](../../engine/src/catalog/types.rs) — 편집 거리 기반 후보 제안

```rust
error!("cannot delete node {id}: it still has {deg} relationship(s). use DETACH DELETE");
```
— [`cypher/mod.rs:286-289`](../../engine/src/cypher/mod.rs) — 무엇을 하라고 말한다

```rust
error!("cypher execution failed: {e}\n--- compiled SQL ---\n{sql}");
```
— [`cypher/mod.rs:149`](../../engine/src/cypher/mod.rs) — 컴파일된 SQL을 함께 보여준다

```rust
return Err(format!("unknown function '{other}'. supported: count, sum, …"));
```
— [`cypher/compile.rs:1558-1566`](../../engine/src/cypher/compile.rs) — 유효 대안 전체 목록

### 오류를 삼키는 곳 (주의)

| 위치 | 무엇을 삼키는가 | 정당한가 |
|---|---|---|
| [`cypher/mod.rs:134`](../../engine/src/cypher/mod.rs) | 감사 INSERT 실패 `.ok()` | 감사 실패가 질의를 죽이면 안 된다는 판단. 단, 감사 유실이 조용하다 |
| [`catalog/types.rs:96`](../../engine/src/catalog/types.rs) | 별칭 뷰 생성 실패 → `pgrx::log!` | ✅ 명시적. 뷰는 편의이고 이름 충돌이 타입 생성을 막으면 안 된다 |
| [`storage/mod.rs:138-147`](../../engine/src/storage/mod.rs) | 확장(widening) DDL 실패 `let _ =` | ⚠️ **실패가 조용하다.** 컬럼이 안 바뀌었는데 카탈로그만 바뀔 수 있다 |
| [`agent/mod.rs:427-438`](../../engine/src/agent/mod.rs) | `SET` 실패 `.ok()` | ⚠️ 한도가 조용히 적용되지 않는다 |
| [`storage/mod.rs:379`](../../engine/src/storage/mod.rs) | 인접 삭제 실패 `.ok()` | ⚠️ 정합성에 직결 |

→ **ARCH-17**

---

## 6. 리소스 상한

### 있는 것 (Facts)

| 상한 | 값 | 위치 |
|---|---|---|
| 인접 세그먼트 크기 | `CHUNK = 256` | [`adjacency.rs:15`](../../engine/src/storage/adjacency.rs) |
| 가변 길이 상한 (`*..` 무한대) | `MAX_VAR_LENGTH = 8` | [`cypher/mod.rs:24`](../../engine/src/cypher/mod.rs) |
| 컴파일 캐시 항목 | 512 (초과 시 전량 폐기) | [`cypher/mod.rs:61-63`](../../engine/src/cypher/mod.rs) |
| genai 타임아웃 | 5,000 ms 기본 | [`compat/genai.rs:41`](../../engine/src/compat/genai.rs) |
| 임베딩 차원 | 1..16,000 | [`vector/mod.rs:41-43`](../../engine/src/vector/mod.rs) |
| 감사 오류 메시지 | 200자로 자름 | [`cypher/mod.rs:131`](../../engine/src/cypher/mod.rs) |
| `og_schema` 토큰 예산 | 타입당 ~30토큰 추정, 최소 8개 | [`agent/mod.rs:60-63`](../../engine/src/agent/mod.rs) |
| 무결성 검사 | 항목당 `LIMIT 100` | [`storage/stats.rs:188 등`](../../engine/src/storage/stats.rs) |
| Bolt 세션 | PostgreSQL `max_connections` | [`bolt/src/main.rs:69-70`](../../bolt/src/main.rs) |
| Studio 풀 | `max: 8` | [`portal/server/index.js:27`](../../portal/server/index.js) |
| Studio 요청 본문 | 4 MB | [`portal/server/index.js:53`](../../portal/server/index.js) |

### 없는 것 (Facts)

| 없는 상한 | 결과 |
|---|---|
| 결과 행 수 상한 | `og_cypher()` 가 전체 결과를 `Vec<Value>` 로 모은다 → **ARCH-11** |
| `og_reach` 방문집합 크기 | `HashSet<i64>` 가 도달 가능 노드 수만큼 커진다 |
| `og_csr_build` 메모리 상한 | 백엔드 Rust 힙. 측정 8.4~9.2 MiB (50k/975k) |
| 에이전트 `max_rows` | GUC로 쓰지만 **읽는 코드가 없다** → **ARCH-09** |
| genai 동시 호출 수 / 서킷 브레이커 | 없음 |
| 컴파일 캐시 메모리 바이트 | 항목 수만 제한. 긴 질의는 무제한 |

---

## 7. 복제와 읽기 대기 서버

spec 007 P0의 주장:
> 별도 코드가 필요 없다는 것이 요점이다. 모든 그래프 구조가 일반 힙 릴레이션이므로
> PostgreSQL 스트리밍 복제가 그대로 동작한다.

**저장 계층에 대해서는 참이다.** 그러나 **질의 표면**에는 읽기 전용과 충돌하는 두 지점이 있다:

| 지점 | 무엇 | 근거 |
|---|---|---|
| 감사 INSERT | 모든 `og_cypher()` 호출이 `og_data.og_audit` 에 INSERT | [`cypher/mod.rs:122-135`](../../engine/src/cypher/mod.rs) |
| 컴파일 시 DDL | 처음 보는 라벨은 `CREATE OR REPLACE VIEW` | [`cypher/views.rs:135`](../../engine/src/cypher/views.rs) |

두 번째는 더 심각하다: 대기 서버에서는 뷰를 **만들 수 없으므로**,
주 서버에서 한 번도 컴파일된 적 없는 라벨에 대한 질의가 실패한다.

> **미확인**: pgrx가 SPI 안의 PostgreSQL ERROR를 `Result::Err` 로 돌려주는지
> 아니면 unwind로 전파하는지에 따라 `.ok()` 로 감싼 감사 INSERT의 동작이 달라진다.
> 대기 서버에서의 실제 동작은 **테스트로 확인해야 한다.**

→ **ARCH-03**

---

## Decisions

| # | 결정 | 근거 |
|---|---|---|
| D-1 | 인접 갱신을 비동기로 하지 않는다 | 헌법 원칙 IX. 최종적 일관성 지름길 금지 |
| D-2 | genai는 기본 비활성이다 | 백엔드가 남의 HTTP 서버에 블로킹되는 것은 실제 비용이다 |
| D-3 | genai 엔드포인트는 인자가 아니라 설정이다 | 질의 권한이 fetch 권한이 되면 안 된다 (SSRF 방어) |
| D-4 | genai는 동기 블로킹 클라이언트를 쓴다 | 백엔드가 이미 기다리는 스레드다. async는 스케줄러 비용만 든다 |
| D-5 | 쓰기 카운터는 트랜잭션 로그가 아니다 | 명시적으로 문서화하고, 소비자(Bolt)가 유효 창을 지킨다 |
| D-6 | 백엔드 로컬 CSR은 자동 라우팅하지 않는다 | 스냅샷 동결을 기본값으로 두면 원칙 IX가 조용히 깨진다 |
| D-7 | Bolt 게이트웨이는 연결당 스레드다 | 동시성 한계를 PostgreSQL의 것과 같게 |

---

## Forbidden / Required

**Forbidden**
- ❌ 인접 구조를 트랜잭션 밖에서 갱신하는 것.
- ❌ 백엔드 로컬 상태를 진실의 원천으로 삼는 것.
- ❌ 새 외부 네트워크 의존을 기본 활성으로 추가하는 것.
- ❌ 엔드포인트/URL을 질의 인자로 받는 것 (SSRF).
- ❌ 정합성에 직결되는 실패를 `.ok()` / `let _ =` 로 삼키는 것.
- ❌ 읽기 경로에서 쓰기(INSERT/DDL)를 수행하는 것 — 대기 서버를 막는다.

**Required**
- ✅ 새 외부 의존에는 **비활성 기본값 + 타임아웃 + 설정 기반 엔드포인트** 세 가지를 모두 붙일 것.
- ✅ 트랜잭션 경계 밖에 남는 새 상태를 도입하면 **수명·무효화·유효 창**을 문서화할 것.
- ✅ 사용자에게 보이는 오류는 `error!` 로 낼 것. `.unwrap()` 은 "여기 도달하면 버그"인
  불변식에만 쓸 것.
- ✅ 새 무한 성장 가능 자료구조(`Vec`, `HashSet`)에는 상한이나 스트리밍 경로를 함께 설계할 것.

<!-- affects: architecture, backend, operations, security, llm -->
<!-- requires-update: 08_operations/, 07_security/, 03_backend/, 01_architecture/08_improvements_architecture.md -->
