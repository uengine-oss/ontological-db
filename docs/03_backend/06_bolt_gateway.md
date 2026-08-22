# Bolt 게이트웨이 — 핸드셰이크, 메시지, 트랜잭션, 인증, PackStream

> **이 문서가 답하는 질문**
> - Neo4j 드라이버가 `bolt://localhost:7687`에 붙으면 무슨 일이 일어나는가?
> - 인증은 어디서 되는가? 사용자 저장소가 따로 있는가?
> - 트랜잭션은 어떻게 매핑되는가?
> - PackStream은 무엇을 지원하고 무엇을 지원하지 않는가?
> - 게이트웨이가 Cypher를 해석하는가?

원문 근거: spec 011, `bolt/src/main.rs`, `bolt/src/session.rs`, `bolt/src/packstream.rs`.

---

## 1. 결정 — 게이트웨이는 상태를 갖지 않는다

`bolt/src/main.rs:3-5`:

> Speaks Bolt to Neo4j drivers and executes what they send with `og_cypher()`.
> It holds no state of its own: no parser, no planner, no cache, no user store.
> Cypher is never interpreted here — one query path, spec 003's.

이것이 이 바이너리의 **유일한 설계 원칙**이며, 코드 전반에서 강제된다:

| 질문 | 게이트웨이가 하는 일 | 라인 |
|---|---|---|
| 이 질의는 쓰기인가? | `SELECT og_cypher_check($1::text)` | `session.rs:444-461` |
| 컬럼 순서는? | `SELECT og_cypher_columns($1::text)` | `session.rs:283-289` |
| 실행 | `SELECT og_cypher($1,$2,$3::jsonb)::text` | `session.rs:294-298` |
| 변경 카운트는? | `SELECT og_cypher_stats()::text` | `session.rs:430-436` |

`session.rs:438-441`:

> Does this query write? Answered by the engine's own parser, never by a keyword
> scan here — `CREATE` inside a string literal is not a write, and only the parser
> knows that.

---

## 2. 사실 — 프로세스 구조와 설정

`bolt/src/main.rs:36-80`. 설정은 **환경 변수만** 쓴다 (`main.rs:9-15`):

| 변수 | 기본값 |
|---|---|
| `OG_BOLT_LISTEN` | `0.0.0.0:7687` |
| `OG_BOLT_PGHOST` | `localhost` |
| `OG_BOLT_PGPORT` | `5432` |
| `OG_BOLT_PGDATABASE` | `og` |
| `OG_BOLT_GRAPH` | `default` |
| `OG_BOLT_ADVERTISED` | `OG_BOLT_LISTEN` 값 |

동시성 모델 (`main.rs:69-79`):

```rust
// A connection per thread, a PostgreSQL connection per session: the
// concurrency limit is PostgreSQL's, which is the one that matters.
thread::spawn(move || { ... session::serve(stream, &config) ... });
```

즉 **Bolt 커넥션 1개 = OS 스레드 1개 = PostgreSQL 백엔드 1개**. 커넥션 풀은 없다.
드라이버 종료(`UnexpectedEof`)는 오류로 로깅하지 않는다 (`main.rs:74-78`).

---

## 3. 사실 — 핸드셰이크

`bolt/src/session.rs:85-108`.

```
클라이언트 → 20바이트: 매직 4 + 제안 버전 4개
서버 → 4바이트: 합의된 버전 또는 0
```

| 상수 | 값 | 라인 |
|---|---|---|
| `MAGIC` | `60 60 B0 17` | `session.rs:11` |
| `BOLT_4_4` | `0x0000_0404` (major가 하위 바이트) | `session.rs:13` |
| `NO_VERSION` | `0x0000_0000` | `session.rs:14` |

`speaks(proposal)` (`session.rs:103-108`):

```rust
let major = proposal & 0xFF;
let minor = (proposal >> 8) & 0xFF;
let range = (proposal >> 16) & 0xFF;
major == 4 && minor >= 4 && minor - range.min(minor) <= 4
```

제안 목록 중 하나라도 통과하면 **항상 `BOLT_4_4`로 응답**한다 (`session.rs:91-96`).
매직이 틀리면 아무것도 쓰지 않고 커넥션을 닫는다 (`session.rs:88-90`).

> **주의**: 세 번째 항 `minor - range.min(minor) <= 4`는 `minor >= 4`가 이미 요구된 상태에서
> 항상 참이다. 실질적으로 range 바이트는 무시된다. → `CODE-19`.

합의 후 `set_nodelay(true)` (`session.rs:68`). **TLS는 없다** — `NoTls`(`session.rs:182`).
`bolt+s://` / `neo4j+s://`는 지원하지 않는다.

---

## 4. 사실 — 메시지

`bolt/src/session.rs:16-36`.

**클라이언트 → 서버**

| 시그니처 | 이름 | 처리 |
|---|---|---|
| `0x01` | `HELLO` | `hello()` `session.rs:168` |
| `0x02` | `GOODBYE` | 루프 즉시 종료 `session.rs:117-119` |
| `0x0F` | `RESET` | `reset()` `session.rs:195` |
| `0x10` | `RUN` | `run_query()` `session.rs:242` |
| `0x11` | `BEGIN` | `begin()` `session.rs:208` |
| `0x12` | `COMMIT` | `end_tx("COMMIT")` `session.rs:215` |
| `0x13` | `ROLLBACK` | `end_tx("ROLLBACK")` |
| `0x2F` | `DISCARD` | `pull(..., deliver=false)` `session.rs:143` |
| `0x3F` | `PULL` | `pull(..., deliver=true)` `session.rs:142` |
| `0x66` | `ROUTE` | `routing_table()` `session.rs:388` |

그 외 시그니처는 `Neo.ClientError.Request.Invalid`로 거절된다 (`session.rs:149-152`).

**서버 → 클라이언트**: `SUCCESS 0x70`, `RECORD 0x71`, `IGNORED 0x7E`, `FAILURE 0x7F`.

**그래프 구조체**: `NODE 0x4E`, `RELATIONSHIP 0x52`.
**`PATH 0x50`은 없다** — 경로 값은 구조체가 아니라 일반 리스트/맵으로 나간다.

### 4.1 상태 기계

`session.rs:38-46`:

| 상태 | 의미 |
|---|---|
| `Ready` | 질의를 받을 수 있음 |
| `Streaming` | 결과가 열려 있음. `PULL`/`DISCARD`/`RESET`만 의미 있음 |
| `Failed` | 실패함. `RESET`까지 전부 `IGNORED` (FR-007) |

`Failed` 처리는 디스패치 이전에 있다 (`session.rs:120-123`):

```rust
if self.state == State::Failed && sig != RESET {
    ps::write_message(stream, &Value::Struct(IGNORED, vec![]))?;
    continue;
}
```

어떤 핸들러든 `Err(Failure)`를 반환하면 상태가 `Failed`가 된다 (`session.rs:156-159`).

---

## 5. 사실 — 인증은 PostgreSQL 역할이다

`bolt/src/session.rs:165-193`:

```rust
let user     = extra.map_get("principal")  ...;
let password = extra.map_get("credentials") ...;

let mut cfg = postgres::Config::new();
cfg.host(...).port(...).dbname(...).user(&user).application_name("ontological-bolt");
if !password.is_empty() { cfg.password(&password); }
let client = cfg.connect(NoTls).map_err(|e|
    Failure::client("Neo.ClientError.Security.Unauthorized", pg_message(&e)))?;
```

주석 (`session.rs:165-167`):

> Authentication is PostgreSQL's: the credentials in HELLO are the role's
> credentials, and failing to connect is what "unauthorized" means here.
> No second user store (FR-015).

따라서:

- **권한 부여도 PostgreSQL의 것이다.** `GRANT` / `REVOKE` / RLS 정책이 그대로 적용된다.
- 실패 메시지는 PostgreSQL 원문이 그대로 전달된다 (`pg_message` `session.rs:604-606`).
- HELLO 없이 다른 메시지를 보내면 `client()`가 `Unauthorized`를 낸다 (`session.rs:411-418`).

**HELLO 응답** (`session.rs:187-192`):

```
server        = "Neo4j/4.4.0 (ontological-bolt)"
connection_id = "bolt-<pid>"
hints         = {}
```

`server` 문자열이 `Neo4j/4.4.0`인 이유 (`session.rs:188`): 드라이버가 이 문자열로 기능을 가른다.

---

## 6. 사실 — 트랜잭션과 그래프 선택

### 6.1 트랜잭션

| Bolt | SQL | 라인 |
|---|---|---|
| `BEGIN` | `BEGIN` (+ `select_graph`) | `session.rs:208-213` |
| `COMMIT` | `COMMIT` | `session.rs:215-219` |
| `ROLLBACK` | `ROLLBACK` | 같음 |
| `RESET` | 열려 있으면 `ROLLBACK`, 버퍼 비움, `Ready`로 | `session.rs:195-206` |

`COMMIT` / `ROLLBACK` 응답에는 `bookmark = "ontological:0"`가 실린다 (`session.rs:218`).
**북마크는 상수다** — 인과적 일관성 추적은 구현돼 있지 않다.

**자동 커밋 모드**(`BEGIN` 없는 `RUN`)에서는 게이트웨이가 아무 트랜잭션 문장도 보내지 않는다.
PostgreSQL의 암묵적 트랜잭션이 그대로 경계가 된다 — `og_cypher()` 호출 1개 = 트랜잭션 1개.

### 6.2 그래프(= "데이터베이스") 선택

`select_graph(&extra)` (`session.rs:222-238`):

```
extra["db"]가 있고, "" / "neo4j" / "system"이 아니면 → self.graph = 그 값
그렇지 않고 in_tx 이면 → 유지 (트랜잭션이 시작한 그래프)
그렇지 않으면 → config.default_graph
```

`"neo4j"` / `"system"`을 걸러내는 이유 (`session.rs:224-226`): 애플리케이션이 아무것도
지정하지 않았을 때 드라이버가 보내는 값이며, "neo4j라는 이름의 그래프"가 아니라 "기본값"을 뜻한다.

명시적 트랜잭션 중에는 드라이버가 `BEGIN`에만 `db`를 싣고 이후 `RUN`에서는 생략한다.
그래서 `in_tx`일 때 `db` 부재는 "기본값"이 아니라 "이 트랜잭션이 시작한 그것"이다 (`session.rs:231-236`).

---

## 7. 사실 — 질의 실행

`run_query` (`session.rs:242-329`).

```
1. fields에서 query / params / extra 분리                         242-251
2. select_graph(&extra)                                           252
3. params → JSON 문자열 (to_json)                                 254
4. EXPLAIN / PROFILE 접두사 제거 (split_plan_prefix)              265
5. og_cypher_check(body) → 쓰기 여부. qtype = "w" | "r"           266-267
6. plan_only 이면 빈 결과 + Streaming 상태로 즉시 반환             269-279
7. og_cypher_columns(query) → 필드 순서                           283-289
8. og_cypher(graph, query, params) → 전 행 수집                   291-299
9. 행마다 record(json, fields) 로 변환해 pending 에 적재            301-317
10. SUCCESS { fields, t_first: 0, qid: -1 }                       324-328
```

### 7.1 `EXPLAIN` / `PROFILE`

`split_plan_prefix` (`session.rs:471-482`)가 앞의 `EXPLAIN` / `PROFILE`을 벗겨 낸다.
그 뒤에는 **레코드 없이** summary만 나가고, summary의 `type`이 `"r"` / `"w"`를 알려준다.

이 한 필드가 왜 중요한지 주석에 있다 (`session.rs:256-264`):
드라이버 위에 얹힌 도구가 실행 전에 읽기/쓰기를 가르는 유일한 수단이며,
공식 Neo4j MCP 서버가 두 질의 도구를 정확히 여기에 걸어 둔다.

`PROFILE`은 `EXPLAIN`처럼 취급한다 (`session.rs:467-470`) — 계측할 플랜 통계가 없으므로,
없는 플랜을 지어내는 대신 질의 종류만 정직하게 보고한다.

### 7.2 필드 순서

`og_cypher()`는 jsonb 객체를 돌려주고 **jsonb는 키를 정렬한다.**
따라서 행 자체로는 `RETURN`이 요구한 순서를 알 수 없다.
그래서 파서에 물어본다 (`cypher/mod.rs:711-738` `og_cypher_columns`).

`RETURN *`처럼 파서가 순서를 알 수 없는 경우 빈 배열이 오고,
게이트웨이는 첫 행의 키 순서로 폴백한다 (`session.rs:309-315`).

### 7.3 ★ 스트리밍이 아니다

`session.rs:291-320` — `og_cypher()`의 **모든 행을 먼저 수집**해 `self.pending`에 담고,
그 다음에야 RUN에 응답한다. `PULL n`은 이미 메모리에 있는 것을 잘라 보낼 뿐이다.

즉 `PULL {n: 10}`을 보내도 서버는 이미 전체 결과를 물질화했다.
큰 결과에서 게이트웨이 메모리가 결과 크기에 비례한다. → `CODE-20`.

---

## 8. 사실 — `PULL` / `DISCARD`와 summary

`pull` (`session.rs:333-384`).

```
state != Streaming → FAILURE "no result is open on this connection"
n = extra["n"]  (없으면 -1 = 전부)
take = min(n, 남은 개수)
deliver 이면 take 개의 RECORD 전송
cursor += take
```

**summary 메타** (`session.rs:359-382`):

| 키 | 조건 |
|---|---|
| `t_last: 0` | 항상 |
| `has_more: true` | 아직 남았을 때 |
| `type: "r"` \| `"w"` | 다 보냈을 때 |
| `stats: {...}` | 다 보냈고 **쓰기일 때만** |
| `db: <graph>` | 다 보냈을 때 |
| `bookmark: "ontological:0"` | 다 보냈고 명시적 트랜잭션이 아닐 때 |

`stats`는 `og_cypher_stats()`에서 온다 (`session.rs:430-436`).
`session.rs:368-372`가 그 타이밍이 유일하게 유효한 창임을 명시한다 —
같은 백엔드에서, 직전 `og_cypher()` 호출이 무엇을 바꿨는지 읽는 것이기 때문이다.

**best effort**다 (`session.rs:426-429`): 요약 하나 때문에 성공한 쓰기를 실패시키지 않는다.
`query_one(...).ok()?` — 실패하면 카운터 없이 넘어간다.

---

## 9. 사실 — 라우팅

`routing_table` (`session.rs:388-407`) — 서버 하나를 `WRITE` / `READ` / `ROUTE` 세 역할로
모두 광고한다. TTL 300.

`session.rs:386-387`:

> One server, announced as every role. A real routing table belongs to the cluster
> spec (007); this exists so `neo4j://` URIs connect at all.

---

## 10. 사실 — 값 변환

### 10.1 결과 → Bolt (`to_bolt`, `session.rs:498-541`)

`og_cypher()`는 노드를 `{_id, _type, …props}`, 관계를 `{_id, _type, _src, _dst, …props}`로 준다.
드라이버는 구조체를 원한다:

| 조건 | Bolt 값 |
|---|---|
| `_id` + `_type` + `_src` + `_dst` 전부 있음 | `Struct(0x52, [id, src, dst, type, props])` |
| `_id` + `_type`만 있음 | `Struct(0x4E, [id, [type], props])` |
| 그 외 객체 | 일반 `Map` |

`props`는 `_`로 시작하지 않는 키만 모은다 (`session.rs:515-522`).

**노드의 labels는 항상 원소 1개**다 (`session.rs:536`) — 구체 타입 이름 하나.
Cypher의 `labels()` 함수는 상위 타입 사슬 전체를 주지만(`compile.rs:1470-1478`),
Bolt 노드 구조체는 그렇지 않다. → `CODE-21`.

### 10.2 파라미터 → JSON (`to_json`, `session.rs:546-561`)

```rust
Value::Struct(sig, _) => serde_json::Value::String(format!("<unsupported struct 0x{sig:02X}>")),
```

드라이버가 보내는 시간/공간 구조체(Date, DateTime, Duration, Point)는 **문자열 자리표시자로 바뀐다.**
`session.rs:557-559`:

> Drivers can send temporal/spatial structures; 4.4 support for those is out of
> scope (spec 011), and silently mangling them would be worse.

`session.rs:544-545`가 주입 보장을 다시 확인한다: 파라미터는 jsonb로 `og_cypher()`에 가고
질의 텍스트에 보간되지 않는다.

### 10.3 오류 매핑 (`Failure::from_pg`, `session.rs:578-593`)

```rust
let code = if message.contains("not supported") || message.contains("expected")
              || message.contains("unknown label") || message.contains("is not defined") {
    "Neo.ClientError.Statement.SyntaxError"
} else if message.contains("does not exist") {
    "Neo.ClientError.Database.DatabaseNotFound"
} else if message.contains("permission denied") {
    "Neo.ClientError.Security.Forbidden"
} else {
    "Neo.ClientError.Statement.ArgumentError"
};
```

메시지 본문은 **원문 그대로** 전달된다 (`session.rs:575-577`) — 그 메시지가 구성 요소를 지목하고
대안을 제시하므로(spec 003 FR-008) 에이전트가 그걸 보고 재시도한다(헌법 원칙 VIII).

> **주의**: 코드 결정이 **영어 메시지 부분 문자열 매칭**이다. 엔진의 오류 문구를 바꾸면
> Bolt 오류 코드가 조용히 바뀐다. → `CODE-11`.

---

## 11. 사실 — PackStream v2 (`bolt/src/packstream.rs`, 348줄)

크레이트를 쓰지 않고 직접 쓴 이유 (`packstream.rs:3-5`): 지원 매트릭스가 문서화된 산출물이며(FR-020),
의존성을 쓰면 그것을 우리 대신 결정해 버린다.

### 11.1 지원 타입

`packstream.rs:10-21`: `Null` `Bool` `Int` `Float` `String` `List` `Map` `Struct(u8, Vec<Value>)`.
**`Bytes` 타입은 없다.**

### 11.2 인코딩 마커

| 타입 | 마커 |
|---|---|
| Null / False / True | `C0` / `C2` / `C3` |
| Float64 | `C1` + 8바이트 BE |
| Int | tiny `-16..=127`, `C8`(i8), `C9`(i16), `CA`(i32), `CB`(i64) (`packstream.rs:95-113`) |
| String | tiny `80\|n`, `D0`/`D1`/`D2` (`packstream.rs:118-129`) |
| List | tiny `90\|n`, `D4`/`D5`/`D6` |
| Map | tiny `A0\|n`, `D8`/`D9`/`DA` |
| Struct | `B0 \| (n & 0x0F)` + 시그니처 |

**구조체는 tiny 형태만 지원한다** (`packstream.rs:84-86`) — 필드 최대 15개.
모든 Bolt 메시지와 그래프 타입이 그 안에 들어간다.

### 11.3 디코딩

`Reader::value()` (`packstream.rs:171-217`). 모르는 마커는
`packstream: unknown marker 0x..`로 `InvalidData` 오류가 된다.
맵 키가 문자열이 아니면 거절한다 (`packstream.rs:236-244`).

문자열은 `String::from_utf8_lossy` (`packstream.rs:220-223`) — 잘못된 UTF-8은
오류가 아니라 대체 문자가 된다.

### 11.4 청킹

`packstream.rs:245-289`:

```
write_message: 본문을 65,535바이트 청크로 쪼개고 [len:u16][payload] 반복, 끝에 [0,0]
read_message : [0,0]을 만날 때까지 이어 붙임. 단, 본문이 비었으면 no-op 청크로 보고 계속
```

`packstream.rs:247-248`: 청크 경계는 메시지 경계와 무관하다 (FR-003).

### 11.5 단위 테스트

`packstream.rs:285-347` — 6개:

| 테스트 | 확인하는 것 |
|---|---|
| `scalars_round_trip` | Null/Bool/Float/String |
| `every_integer_width_round_trips` | 모든 정수 폭 경계 (`i64::MIN`, `i64::MAX` 포함) |
| `sizes_cross_every_header_boundary` | 0, 15, 16, 255, 256, 70,000 |
| `structures_and_maps_nest` | NODE 구조체 중첩 |
| `a_message_survives_chunking` | 200,000바이트 → 3청크 초과 + 재조립 |
| `utf8_survives` | `"한글 · émoji 🎬"` |

**`bolt/`에서 테스트가 있는 파일은 `packstream.rs`뿐이다.**
`session.rs`(606줄)와 `main.rs`에는 단위 테스트가 없다. → `CODE-22`.

---

## 12. 사실 — 지원하지 않는 것 (브리핑 7절 "011 working" 의 단서)

| 항목 | 상태 | 근거 |
|---|---|---|
| Bolt 5.x | ❌ | `session.rs:103-108` `major == 4 && minor >= 4` |
| Bolt 3.x 이하 | ❌ | 같음 |
| TLS (`bolt+s://`, `neo4j+s://`) | ❌ | `session.rs:182` `NoTls` |
| `PATH` 구조체 (`0x50`) | ❌ | `session.rs:34-36`에 상수 자체가 없음 |
| 시간/공간 타입 (파라미터) | ❌ → 자리표시자 문자열 | `session.rs:559` |
| `Bytes` PackStream 타입 | ❌ | `packstream.rs:10-21` |
| 진짜 스트리밍 | ❌ (전량 버퍼) | `session.rs:291-320` |
| 인과적 일관성 북마크 | ❌ (상수) | `session.rs:218,380` |
| 실제 라우팅 클러스터 | ❌ (단일 서버) | `session.rs:386-387` |
| `qid` 다중 결과 | ❌ (항상 `-1`) | `session.rs:277,327` |
| 다중 라벨 노드 | ❌ (항상 1개) | `session.rs:536` |

---

## 금지 / 필수

- **금지**: 게이트웨이에서 Cypher를 파싱하거나 키워드로 쓰기 여부를 판정하는 것.
  반드시 `og_cypher_check()`에 물어본다 (`session.rs:438-441`).
- **금지**: 두 번째 사용자 저장소를 만드는 것. 인증은 PostgreSQL 역할이다 (FR-015).
- **금지**: 엔진의 오류 메시지 문구를 바꾸면서 `Failure::from_pg`(`session.rs:578-593`)의
  부분 문자열 목록을 확인하지 않는 것.
- **금지**: PackStream 구조체를 16개 이상 필드로 만드는 것. 인코더가 `& 0x0F`로 잘라 버린다
  (`packstream.rs:86`).
- **필수**: `og_cypher_stats()`는 쓰기 직후 **같은 커넥션에서, 다음 질의 전에** 읽어야 한다
  (`cypher/mod.rs:111-116`).
- **필수**: 새 Bolt 메시지를 추가하면 `dispatch`(`session.rs:139-153`)의 `other =>` 분기와
  `State` 전이를 함께 갱신한다.

<!-- affects: backend, api -->
<!-- requires-update: 02_api/, 08_operations/ -->
