# 10. LLM / RAG 계층 개선 포인트

> **이 문서가 답하는 질문**
> - RAG 아키텍처 관점에서 이 데이터베이스의 검색·에이전트 계층에 무엇이 부족한가?
> - 각 문제의 코드 근거는 어디이고, 심각도와 수정 리스크는?
> - 무엇부터 고쳐야 하는가?

> **읽는 법**
> 이 문서의 모든 항목은 **실제 소스를 읽고 확인한 것**이다. 일반론은 없다.
> "미확인"이라고 표기된 것은 코드만으로 판정할 수 없어 실측이 필요한 항목이다.
> 심각도는 *RAG 파이프라인의 답변 품질·안전성에 미치는 영향* 기준이다.

---

## 1. 요약표

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| LLM-01 | RRF의 k=60 하드코딩, 그래프 항이 벡터 항을 지배 | **High** | `engine/src/vector/mod.rs:248,253,272-274` | 기본 가중치에서 앵커 3홉 이내 노드가 벡터 순위와 무관하게 항상 상위 | `rrf_k`/`max_hops`/`pool`을 인자·설정으로 노출, 그래프 신호도 rank로 변환 | 하이브리드 nDCG 실질 개선, 튜닝 가능 | 기존 순위가 바뀜 — 회귀 기준선 필요 |
| LLM-02 | 스키마 토큰 예산이 출력 크기를 제한하지 않음 | **High** | `engine/src/agent/mod.rs:60-63,65-86` | 타입 개수만 자르고 프로퍼티·role은 무제한 → 예산 초과 | 직렬화 후 크기 측정 + 프로퍼티/role 2차 절단 | SC-003 달성 가능, 컨텍스트 초과 방지 | 응답 생성 비용 소폭 증가 |
| LLM-03 | `og_explain_error` 가 오타 레이블을 잡지 못함 | **High** | `engine/src/agent/mod.rs:294-296,317`, `engine/src/catalog/types.rs:160-175`, `engine/src/cypher/compile.rs:709-715` | 오타 레이블은 `{"ok": true}` — `UNKNOWN_LABEL` 은 사문(死文) | 컴파일러가 `LabelMatch::Nothing` 을 진단 채널로 반환 | 재시도 1회 교정률 대폭 상승 (SC-002) | Neo4j 호환(오타=빈결과) 유지하려면 별도 필드로 |
| LLM-04 | `og_diagnose_empty` 가 파라미터를 전달하지 않아 오진 | **Med** | `engine/src/cypher/mod.rs:776`, `engine/src/cypher/compile.rs:1156-1157` | `{name: $x}` 패턴은 항상 0행 → 잘못된 지점을 지목 | 시그니처에 `params jsonb` 추가 후 전달 | 진단 신뢰도 확보 | 시그니처 변경(하위 호환 위해 default 인자) |
| LLM-05 | `og_estimate` 의 빈 파라미터 + 패닉 미포착 | **Med** | `engine/src/agent/mod.rs:352-359` | 추정 왜곡, 컴파일 오류 시 트랜잭션 중단 | `params` 인자 추가 + `PgTryBuilder` 로 감싸기 | SC-009 상관계수 개선, 루프 중단 방지 | 없음(순수 개선) |
| LLM-06 | `genai.vector.encode` 에 재시도·백오프·레이트리밋 없음, VOLATILE | **High** | `engine/src/compat/genai.rs:95,139-149`, `engine/Cargo.toml:25-29` | 1회 실패 = 트랜잭션 중단. 행마다 재호출. 429/5xx 구분 없음 | 재시도(지수 백오프+지터), 429 존중, 동시성 상한, 결과 캐시 | 적재/조회 안정성, 비용 절감 | 백엔드 점유 시간 증가 → `statement_timeout` 과 상호작용 |
| LLM-07 | 배치 임베딩 미지원 | **Med** | `engine/src/compat/genai.rs:70-75,96-97` | 시그니처가 단일 `&str`. 두 provider 모두 배열 입력을 지원하는데 사용 안 함 | `og_genai_encode_batch(text[]) → vector[]` 추가 | 재임베딩 처리량 수십 배 | 응답 크기·메모리, 부분 실패 처리 설계 필요 |
| LLM-08 | API 키 평문 저장 + `pg_dump` 포함 + `og_set_setting` 무권한 | **High** | `engine/src/compat/genai.rs:55-63,140`, `engine/sql/bootstrap.sql:252-255,420-422` | `genai.token` 이 평문 컬럼, 백업에 포함, PUBLIC이 엔드포인트 변경 가능(SSRF) | 토큰을 덤프 제외 + `REVOKE`/`SECURITY DEFINER` + 엔드포인트 allowlist | 자격증명·SSRF 노출 차단 | 기존 배포에서 설정 경로 변경 필요 |
| LLM-09 | 스키마 인트로스펙션의 N+1 SPI + `kind='a'` 예산 누수 | **Med** | `engine/src/agent/mod.rs:38-42,66-74,213,223` | 타입 N개면 SPI 질의 O(N), attribute 타입이 정원만 소비 | 단일 조인 질의로 통합, kind 필터를 SQL로 이동 | 대형 온톨로지에서 응답 지연 급감 | 없음(순수 개선) |
| LLM-10 | 리랭킹 단계 부재 + FTS 융합 미구현 | **Med** | `engine/src/vector/mod.rs` 전체, `specs/004-vector-hybrid-search/tasks.md:29` | ANN 결과가 그대로 최종 순위. 벡터+FTS RRF는 T014 미착수 | `og_hybrid_search` 에 FTS 항 추가, 외부 리랭커용 후보 확장 API | nDCG@10 개선(SC-006) | FTS는 `simple` 사전 — 한국어 재현율 한계 동반 |
| LLM-11 | 임베딩 차원/모델 변경 마이그레이션 경로 부재 | **High** | `engine/src/catalog/types.rs:539-545,550`, `engine/src/vector/mod.rs:49-53,69-71` | 같은 슬롯 재선언 시 유니크 위반. 통과해도 컬럼 차원 불변 | `og_alter_embedding(graph,type,prop,dims)` + 온라인 재구축 절차 | 모델 교체가 운영 작업으로 성립 | 재인덱싱 중 검색 가용성(FR-027) 설계 필요 |
| LLM-12 | stale 추적이 `source_prop` 에 의존, Neo4j DDL 경로에서 유실 | **Med** | `engine/src/vector/mod.rs:308-311,368`, `engine/src/compat/ddl.rs:225-228` | `CREATE VECTOR INDEX` 로 만든 슬롯은 stale 목록에 영원히 안 나옴 | DDL의 `OPTIONS` 에 source 프로퍼티 수용 + 미설정 시 경고 | stale 탐지율 100%(SC-007) 근접 | Neo4j DDL 확장(비표준 옵션) |
| LLM-13 | ANN 재현율 튜닝·측정 부재 | **Med** | 전수 grep: `ef_search`/`iterative_scan` 0건; `bench/harness.py` 에 recall 없음 | 저선택도 필터에서 top-k 미충족 방지책 없음, 재현율 미측정 | `hnsw.ef_search` 노출 + `hnsw.iterative_scan` 활용 + recall 하네스 | SC-001(재현율 95%) 검증 가능 | pgvector 버전 의존 |
| LLM-14 | `og_vector_search` 의 `filter` 가 원시 SQL 보간 | **High** | `engine/src/vector/mod.rs:115-118,126-132` | LLM 생성 문자열을 넘기면 임의 SQL 실행 | 구조화 필터(jsonb) 또는 파서 검증, 원시 경로는 `REVOKE` | 인젝션 경로 제거 | API 변경, 기존 호출부 수정 |
| LLM-15 | 감사 로그 불완전 (실패 롤백·파라미터 누락·함수 미포함) | **High** | `engine/src/cypher/mod.rs:93-99,122-135`; `engine/src/vector/mod.rs`·`agent/mod.rs` 에 감사 부재 | 실패 질의는 롤백돼 사라지고, 파라미터는 기록 안 되며, 벡터/스키마 호출은 미기록 | 서브트랜잭션/`pgaudit`/로그 채널 병행, 감사 대상 확대, 마스킹 정책 | SC-008(100% 기록) 접근 | 감사 쓰기 비용, PII 정책 필요 |
| LLM-16 | groundedness 검증 훅 및 결과 단위 provenance 부재 | **Med** | `specs/008-agent-native-interface/tasks.md:27-28`; `og_cypher_provenance` 미존재 | 답변 근거를 결과 행 단위로 되짚을 수 없음 | 결과 행 → 기여 id 집합 반환 모드 추가 | 환각 검출 루프의 입력 확보 | 질의 지연 증가(FR-004는 50% 이내 요구) |
| LLM-17 | `og.max_rows` 가 강제되지 않음 | **Med** | `engine/src/agent/mod.rs:437-438` (설정만), 읽는 코드 0건 | 결과 행 수 상한이 사실상 없음 | 컴파일된 SQL에 `LIMIT` 주입(T011) 또는 GUC 등록 후 강제 | 폭주 질의 차단(SC-007) | 기존 질의 결과가 잘릴 수 있음 |
| LLM-18 | `og_hybrid_search` 가 관계 타입과 필터를 지원하지 않음 | **Low** | `engine/src/vector/mod.rs:222-231,247,276` | `is_edge=false` 고정, `og_node_json` 고정, `filter` 인자 없음 | `is_edge` 분기 + `filter` 인자 추가 | 관계 임베딩(FR-002)이 하이브리드에서도 1급 | 시그니처 확장 |

---

## 2. 상세

### LLM-01 · RRF의 k=60 하드코딩, 그래프 항이 벡터 항을 지배 — **High**

**근거**: [engine/src/vector/mod.rs:248](../../engine/src/vector/mod.rs),
[:253](../../engine/src/vector/mod.rs), [:272-274](../../engine/src/vector/mod.rs)

```rust
let pool = (k * 10).max(50);                                             // :248
"prox AS (SELECT node, min(depth) AS hops FROM og_vlp({a}::int8, NULL, 'b'::\"char\", 0, 3) …)"  // :253
"COALESCE(1.0 / (1.0 + p.hops), 0)::float8 AS gscore,                     // :272
  {vector_weight} * (1.0 / (60 + c.vrank))                                // :273
+ {graph_weight}  * COALESCE(1.0 / (60 + p.hops), 0) AS fscore"           // :274
```

**현상**

1. RRF 상수 `60` 이 `format!` 문자열 안에 리터럴로 두 번 박혀 있다. 함수 인자도,
   `og_catalog.setting` 키도, GUC도 없다. `pool`(후보 수)과 앵커 최대 홉 `3` 도 같다.
2. 융합되는 두 항의 스케일이 다르다. `vrank` 는 1..pool 순위지만 `hops` 는 0..3 거리다.
   기본 가중치 `(1.0, 1.0)` 에서:

   | 항 | 최대 | 최소 |
   |---|---|---|
   | 벡터 `1/(60+vrank)` | 0.016393 (rank 1) | 0.006250 (rank 100, k=10) |
   | 그래프 `1/(60+hops)` | 0.016667 (0홉) | 0.015873 (3홉) / **0** (도달 불가) |

   최악의 연결 노드 `0.006250 + 0.015873 = 0.022123` > 최선의 비연결 노드
   `0.016393 + 0 = 0.016393`. 즉 **앵커 3홉 이내가 경성 상위 집단이 되고 벡터 순위는
   집단 내부 순서만 정한다.**
3. 반환되는 `graph_score` 는 `1/(1+hops)` 인데 융합에는 `1/(60+hops)` 가 쓰인다.
   구성 점수로 총점을 재현할 수 없다(spec 004 FR-020 위배).

**제안**

```rust
// (a) 상수를 노출한다 — 함수 인자 또는 og_catalog.setting 키
//     rrf_k (default 60), max_hops (default 3), candidate_pool (default k*10)
// (b) 그래프 신호도 rank로 바꾼다: hops로 정렬한 dense_rank()를 RRF에 넣는다
//     그러면 두 항의 값 범위가 같아지고 가중치가 의미를 갖는다
// (c) 반환하는 component score를 융합 항과 일치시킨다
//     vector_rrf = vw/(rrf_k + vrank), graph_rrf = gw/(rrf_k + grank)
```

**예상 효과**: 그래프 신호의 세기를 실제로 조절할 수 있게 되어 하이브리드 랭킹이
튜닝 가능한 축이 된다. spec 004 SC-006("nDCG@10 10% 이상 향상")을 측정 가능한
형태로 만든다.

**리스크**: 기존 `og_hybrid_search` 결과 순위가 바뀐다. 기본값을 현재 동작과 동일하게
두고 새 인자를 opt-in으로 하면 회귀는 없다. 측정 기준선(정답 셋)이 저장소에 없으므로
개선 여부를 검증하려면 그것부터 만들어야 한다.

**당장 가능한 회피(코드 변경 없음)**: `graph_weight := 0.05` 수준으로 낮춘다.
`graph_weight := 0` 은 순수 벡터 순위와 동일해진다.

---

### LLM-02 · 스키마 토큰 예산이 출력 크기를 제한하지 않음 — **High**

**근거**: [engine/src/agent/mod.rs:60-63](../../engine/src/agent/mod.rs), [:65-86](../../engine/src/agent/mod.rs)

```rust
let cap = token_budget.map(|b| ((b as usize) / 30).max(8)).unwrap_or(usize::MAX);
let truncated = total > cap;
for (tid, name, kind, is_abstract, instances) in rows.into_iter().take(cap) {
    let parents = parent_names(tid);      // 상한 없음
    let props   = property_list(tid);     // 상한 없음
    …  "roles": role_list(tid),           // 상한 없음
}
```

**현상**: 예산은 **타입 개수**로만 환산된다(타입 1개 ≈ 30 토큰 고정). 각 타입의
프로퍼티·role·부모 목록은 무제한으로 포함된다. 프로퍼티 200개짜리 타입 하나가
`token_budget = 4000` 을 통째로 넘긴다. 출력 크기를 측정하는 코드는 없다.
추가로 `kind` 가 `'e'`/`'r'` 이 아닌 타입(TypeQL attribute, `'a'`)은 `take(cap)` 의
정원을 소비하면서 두 배열 어디에도 담기지 않는다([agent/mod.rs:68-85](../../engine/src/agent/mod.rs)).

**제안**

```
1. kind 필터를 SQL WHERE 로 올린다 (kind IN ('e','r')) — 정원 누수 제거
2. 타입을 하나씩 직렬화하며 누적 바이트를 센다. 예산의 ~4배 바이트(대략 1토큰≈4바이트)
   를 넘기면 중단하고 truncated 에 실제 shown 을 기록한다
3. 2차 절단: 남은 예산이 부족하면 프로퍼티를 required/key 우선으로 상위 N개만,
   나머지는 "properties_omitted": M 으로 표기
4. truncated 에 "budget_tokens", "estimated_tokens" 를 추가해 검증 가능하게 한다
```

**예상 효과**: spec 008 SC-003(1,000타입 → 4,000토큰)이 실제로 성립한다.
에이전트 컨텍스트 초과로 인한 스키마 유실이 사라진다.

**리스크**: 직렬화 중 크기 측정으로 `og_schema` 응답 생성 비용이 증가한다.
`STABLE` 함수이므로 같은 트랜잭션 내 재사용은 캐시된다.

---

### LLM-03 · `og_explain_error` 가 오타 레이블을 잡지 못함 — **High**

**근거**: [engine/src/agent/mod.rs:294-296](../../engine/src/agent/mod.rs),
[:317](../../engine/src/agent/mod.rs),
[engine/src/catalog/types.rs:160-175](../../engine/src/catalog/types.rs),
[engine/src/cypher/compile.rs:709-715](../../engine/src/cypher/compile.rs)

**현상**: `classify()` 는 메시지에 `"unknown label"` 이 있는지 보지만, **저장소에서
그 문자열을 생성하는 코드가 없다**(전수 grep: `agent/mod.rs` 의 두 분기와 주석 1줄뿐).
Cypher 컴파일러는 알 수 없는 레이블을 오류가 아니라 `LabelMatch::Nothing` →
`constrain("false")` 로 처리한다. 교정 후보는 PostgreSQL **NOTICE** 로만 나가며
JSON 응답에 들어가지 않는다.

결과: `og_explain_error('g', 'MATCH (p:Emploee) RETURN p')` → `{"ok": true}`.
`UNKNOWN_LABEL` 코드와 `suggestions()` 의 두 번째 분기는 도달 불가능한 죽은 코드다.
[docs/agents.md:87-105](../../docs/agents.md) 의 예제 출력은 현재 코드에서 재현되지 않는다.

**제안**

```rust
// Compiler 에 진단 채널을 둔다 (이미 self.notes 가 있다 — compile.rs:649)
// resolve_label_match 가 Nothing 을 반환할 때 notes 에 구조화 항목을 push:
//   { code: "UNKNOWN_LABEL", label: "Emploee", suggestions: ["Employee"] }
// compile_read 의 결과에 notes 를 실어 보내고,
// og_explain_error 는 ok=true 여도 notes 가 비지 않으면
//   { ok: true, warnings: [...] } 로 반환한다.
```

Neo4j 호환("오타 레이블 = 빈 결과")은 그대로 유지된다 — 질의는 여전히 성공하고,
경고만 추가된다.

**예상 효과**: spec 008 SC-002("1회 재시도 내 교정 성공률 90% 이상")의 가장 큰
누수를 막는다. 레이블 오타는 LLM이 만드는 가장 흔한 Cypher 오류다
([engine/src/agent/mod.rs:3-6](../../engine/src/agent/mod.rs) 모듈 주석이 직접 지목).

**리스크**: `og_explain_error` 응답에 필드가 추가되므로 FR-011(오류 형식 안정성)
관점에서 버전 표기가 필요할 수 있다. `ok` 의 의미는 바뀌지 않으므로 기존 소비자는
영향받지 않는다.

---

### LLM-04 · `og_diagnose_empty` 가 파라미터를 전달하지 않아 오진 — **Med**

**근거**: [engine/src/cypher/mod.rs:776](../../engine/src/cypher/mod.rs),
[engine/src/cypher/compile.rs:1156-1157](../../engine/src/cypher/compile.rs)

```rust
let n = exec_json(&compiled.sql, &json!({}))     // mod.rs:776 — 빈 파라미터
…
let base = format!("({PARAM} ->> {})", sql_str(p));   // compile.rs:1157
```

**현상**: 부분 패턴이 빈 jsonb로 실행된다. `MATCH (m:MeetingRoom {name: $room})…` 같은
패턴은 `($1 ->> 'room')` 이 NULL이 되어 **항상 0행**이 되고, 진단은 첫 요소를
"여기서 매치가 비었다"고 지목한다. 실제 원인과 무관한 오보다.

부수적으로 `WHERE` 절은 부분 컴파일에 포함되지 않으므로
([cypher/mod.rs:763-775](../../engine/src/cypher/mod.rs)) `rows` 는 WHERE 이전 값이다 —
이건 의도된 설계지만 응답에 명시되지 않는다.

**제안**

```sql
-- 시그니처 확장 (하위 호환: default)
og_diagnose_empty(graph text, query text, params jsonb DEFAULT '{}')
-- 내부: exec_json(&compiled.sql, params) 로 전달
-- 추가: 각 step 에 "where_applied": false 를 명시
```

**예상 효과**: 파라미터를 쓰는 질의(= 이 DB가 권장하는 유일한 방식,
spec 003 FR-026)에서 진단이 실제로 동작한다.

**리스크**: 없음. 기본값이 현재 동작과 같다.

---

### LLM-05 · `og_estimate` 의 빈 파라미터 + 패닉 미포착 — **Med**

**근거**: [engine/src/agent/mod.rs:352-359](../../engine/src/agent/mod.rs)

```rust
let sql = match crate::cypher::compile_for_diagnostics(graph, query) {   // :352 — catch_unwind 없음
    Ok(s) => s, Err(e) => return JsonB(json!({ "error": e })),
};
let plan = crate::spiu::one_mut::<JsonB>(
    &format!("EXPLAIN (FORMAT JSON) {sql}"),
    &[JsonB(json!({})).into()],                                          // :358 — 빈 파라미터
);
```

**현상**

1. `EXPLAIN` 이 빈 파라미터로 실행되므로 파라미터 질의의 선택도 추정이 왜곡된다.
   spec 008 SC-009(상관계수 0.8 이상)를 파라미터 질의에 대해 보장할 근거가 없다.
2. `og_explain_error` 는 `catch_unwind` 로 감싸지만([agent/mod.rs:271-273](../../engine/src/agent/mod.rs))
   `og_estimate` 는 감싸지 않는다. `types::graph_id(graph)` 의 `error!`
   ([engine/src/cypher/compile.rs:155](../../engine/src/cypher/compile.rs))가 트랜잭션을
   중단시킨다 — dry-run 함수가 세션을 죽인다.
3. 응답의 `sql` 필드에 컴파일된 SQL 전문이 들어간다([agent/mod.rs:393](../../engine/src/agent/mod.rs)).
   에이전트 컨텍스트에 그대로 흘리면 토큰을 크게 소비한다.

**제안**

```
1. og_estimate(graph, query, params jsonb DEFAULT '{}') 로 확장하고 EXPLAIN 에 전달
2. 컴파일 호출을 pgrx::PgTryBuilder 로 감싸 { error, code } 로 정규화
   (원시 std::panic::catch_unwind 는 og_explain_error 에서도 PgTryBuilder 로 교체)
3. 응답에 "sql_length" 를 추가하고 "sql" 은 include_sql 인자로 opt-in
4. advice 임계값 3개를 og_catalog.setting 키로 노출
   (estimate.rows_warn, estimate.cost_warn)
```

**예상 효과**: 에이전트 루프가 dry-run 때문에 끊기지 않고, 추정치가 실제와 상관을 갖는다.

**리스크**: 없음(전부 기본값 보존 가능). `PgTryBuilder` 전환은 pgrx 관용 경로이므로
오히려 안전성이 올라간다.

---

### LLM-06 · `genai.vector.encode` 에 재시도·백오프·레이트리밋 없음 — **High**

**근거**: [engine/src/compat/genai.rs:95](../../engine/src/compat/genai.rs) (VOLATILE),
[:139-149](../../engine/src/compat/genai.rs),
[engine/Cargo.toml:25-29](../../engine/Cargo.toml)

```rust
#[pg_extern]                                        // :95 — stable/immutable 아님 → VOLATILE
…
let mut request = ureq::post(&endpoint).timeout(Duration::from_millis(timeout));   // :139
match request.send_json(request_body(&provider, &model, resource)) {               // :144
    Ok(response) => response.into_json()…,
    Err(e) => error!("embedding request to '{endpoint}' failed: {e}"),             // :148
}
```

**현상** (전부 코드로 확인)

| 항목 | 상태 |
|---|---|
| 재시도 | 없음 — `send_json` 1회, 실패 시 즉시 `error!` (트랜잭션 중단) |
| 백오프 / 지터 | 없음 |
| 429 / 5xx / 네트워크 오류 구분 | 없음 — 전부 동일 메시지 |
| `Retry-After` 존중 | 없음 |
| 동시성 상한 | 없음 (백엔드 수만큼 동시 요청 가능) |
| 결과 캐시 | 없음 |
| 함수 휘발성 | VOLATILE → 같은 텍스트라도 행마다 재호출 |
| 요청 감사 | 없음 |

유일한 안전장치는 `genai.timeout_ms`(기본 5000,
[genai.rs:41,135-137](../../engine/src/compat/genai.rs))다. N행 UPDATE에서 임베딩을 호출하면
최악 `N × 5초` 동안 백엔드 하나가 네트워크에 묶인다.

**제안**

```rust
// (a) 재시도: 429/5xx/타임아웃에 한해 지수 백오프 + 지터, 상한은 설정으로
//     genai.max_retries (default 2), genai.retry_base_ms (default 200)
//     Retry-After 헤더가 있으면 그것을 따른다
// (b) 오류 분류: 4xx(설정 오류, 재시도 무의미) / 429(대기) / 5xx·타임아웃(재시도)
//     를 서로 다른 메시지와 SQLSTATE 로
// (c) 휘발성: 순수 함수이므로 STABLE 로 낮추면 같은 문장 안의 중복 호출이 접힌다
//     (엔드포인트 설정이 트랜잭션 중 바뀌지 않는다는 전제 — 문서화 필요)
// (d) 동시성: genai.max_concurrent 를 advisory lock 으로 강제
// (e) og_data.og_audit 에 lang='genai' 로 호출을 남긴다
```

**예상 효과**: 일시적 네트워크 오류로 적재 트랜잭션 전체가 실패하는 일이 사라진다.
레이트 리밋이 있는 상용 임베딩 API(OpenAI/Azure)에서 실사용 가능해진다.

**리스크**: 재시도는 백엔드 점유 시간을 늘린다. `statement_timeout` 과의 상호작용을
문서화해야 한다(재시도 총 시간 < statement_timeout). `STABLE` 전환은 설정 변경
가시성 의미가 바뀌므로 신중히.

---

### LLM-07 · 배치 임베딩 미지원 — **Med**

**근거**: [engine/src/compat/genai.rs:70-75](../../engine/src/compat/genai.rs),
[:96-97](../../engine/src/compat/genai.rs)

```rust
fn request_body(provider: &str, model: &str, text: &str) -> Value {
    match provider {
        "ollama" => json!({ "model": model, "input": text }),
        _        => json!({ "model": model, "input": text }),   // 두 분기가 동일
    }
}
…
fn og_genai_encode(resource: &str, …) -> Vec<f64>              // 단일 문자열
```

**현상**: 시그니처가 단일 문자열이므로 텍스트 1건당 HTTP 왕복 1회다.
Ollama `/api/embed` 와 OpenAI `/v1/embeddings` 는 **둘 다 `input` 에 배열을 받는다**
(둘 다 `"input"` 키를 쓰므로 구조 변경 없이 배열 전달 가능). 저장소의
`request_body` 는 두 분기가 문자 그대로 동일하다 — 주석은 "Two shapes"라고 하지만
실제로는 한 가지 형태만 만든다.

전체 재임베딩(모델 교체, 대량 적재)에서 이것이 처리량 병목이 된다.

**제안**

```sql
-- 새 함수
og_genai_encode_batch(resources text[], provider text DEFAULT NULL,
                      configuration jsonb DEFAULT '{}') → vector[]
-- 내부: {"model": m, "input": [t1, t2, …]} 한 번, 응답의 배열 순서로 매핑
-- genai.batch_max (default 64) 로 청크 분할
-- 부분 실패: 응답 길이 ≠ 입력 길이면 전체 실패 (조용한 오정렬 방지)
```

사용례:

```sql
UPDATE og_data.n_5 x SET p_emb = b.v
  FROM (SELECT unnest(ids) AS id, unnest(og_genai_encode_batch(texts)) AS v
          FROM (SELECT array_agg(id) ids, array_agg(p_body) texts
                  FROM og_data.n_5 WHERE p_emb IS NULL LIMIT 64) s) b
 WHERE x.id = b.id;
```

**예상 효과**: 왕복 횟수가 1/64로 줄어 재임베딩 시간이 수십 배 단축된다.

**리스크**: 응답 크기(64 × 1024 × 8바이트 ≈ 512KB)와 메모리. 순서 보장이 provider
계약에 의존하므로 길이 검증 필수. 부분 실패 시 어떤 항목이 실패했는지 알 수 없다.

---

### LLM-08 · API 키 평문 저장 + `pg_dump` 포함 + `og_set_setting` 무권한 — **High**

**근거**: [engine/src/compat/genai.rs:55-63](../../engine/src/compat/genai.rs),
[:140-142](../../engine/src/compat/genai.rs),
[engine/sql/bootstrap.sql:252-255](../../engine/sql/bootstrap.sql),
[:420-422](../../engine/sql/bootstrap.sql)

```sql
CREATE TABLE og_catalog.setting ( key text PRIMARY KEY, value text NOT NULL );   -- :252-255
…
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (''chunk_size'', ''supernode_threshold'',
                       ''inference_max_depth'', ''schema_version'')');           -- :420-422
```

**현상**

1. `genai.token` 은 평문 `text` 로 저장된다([genai.rs:140](../../engine/src/compat/genai.rs)).
2. 덤프 제외 목록에 없으므로 **`pg_dump` 산출물에 평문으로 들어간다**.
3. `og_set_setting` 에 권한 검사가 없다([genai.rs:55-63](../../engine/src/compat/genai.rs)).
   `engine/sql/bootstrap.sql` / `access.sql` 에 `GRANT`/`REVOKE` 가 **한 줄도 없으므로**
   (전수 grep 0건) 함수의 `EXECUTE` 는 PostgreSQL 기본대로 `PUBLIC` 이다.
4. 따라서 함수를 호출할 수 있는 주체는 `genai.endpoint` 를 임의 URL로 바꿀 수 있다.
   모듈 주석의 "Query rights are not fetch rights"
   ([genai.rs:22-24](../../engine/src/compat/genai.rs))는 **Cypher 경로에서만** 참이다.
   Studio의 `POST /api/sql`([portal/server/index.js:296-308](../../portal/server/index.js))은
   임의 SQL을 받으므로 이 보증을 우회한다. → 데이터베이스 백엔드에서 나가는 SSRF.

**제안**

```sql
-- (a) 토큰을 덤프에서 제외
SELECT pg_catalog.pg_extension_config_dump('og_catalog.setting',
    'WHERE key NOT IN (…) AND key NOT LIKE ''%.token''');

-- (b) 설정 함수와 테이블 권한 회수 (확장 스크립트에 포함시킬 것)
REVOKE ALL ON og_catalog.setting FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION og_set_setting(text, text) FROM PUBLIC;
```

```rust
// (c) 엔드포인트 allowlist: genai.endpoint_allow (콤마 구분 호스트)를 두고
//     og_set_setting('genai.endpoint', …) 시 호스트를 검증한다.
//     사설 대역(169.254.169.254, 127.0.0.0/8, 10/8, 172.16/12, 192.168/16)은 기본 거부.
// (d) 토큰은 값 대신 참조를 저장한다:
//     genai.token_env = 'OG_GENAI_TOKEN' → 서버 환경변수에서 읽는다.
//     DB에 비밀이 남지 않으므로 덤프·복제·RLS 문제가 동시에 사라진다.
```

**예상 효과**: 백업 유출로 인한 임베딩 API 키 노출 차단, 클라우드 메타데이터
엔드포인트를 겨냥한 SSRF 경로 차단.

**리스크**: 기존 배포에서 토큰 설정 경로가 바뀐다(마이그레이션 안내 필요).
`REVOKE` 는 단일 사용자 개발 환경에서 기능을 깨뜨릴 수 있으므로 확장 소유자에게는
`GRANT` 를 남겨야 한다.

**당장 가능한 회피**: [08_guardrails_and_roles.md](08_guardrails_and_roles.md) 6.1절의
`REVOKE` 스크립트를 운영자가 직접 적용한다.

---

### LLM-09 · 스키마 인트로스펙션의 N+1 SPI + `kind='a'` 예산 누수 — **Med**

**근거**: [engine/src/agent/mod.rs:38-42](../../engine/src/agent/mod.rs),
[:66-74](../../engine/src/agent/mod.rs), [:213,223](../../engine/src/agent/mod.rs)

**현상**

1. `og_schema` 의 타입 스캔은 타입마다 상관 서브쿼리 2개(`og_node` count, `og_edge`
   count)를 돌린다([:38-42](../../engine/src/agent/mod.rs)). 그 뒤 반환되는 타입마다
   `parent_names` + `property_list`(+ 관계면 `role_list`) SPI 질의가 붙는다([:66-74]).
   → 타입 N개에 대해 SPI 질의 O(N).
2. `og_schema_for` 는 **점수 계산 단계에서 그래프의 모든 타입**에 대해
   `property_list(tid)` 를 호출한다([:213,223](../../engine/src/agent/mod.rs)). 상위 12개를
   확정한 뒤 같은 데이터를 다시 조회한다([:243-245]).
3. `kind` 필터가 Rust 쪽에 있어([:68-85]) `'a'`(TypeQL attribute) 타입이
   `take(cap)` 정원을 소비하고도 출력에 나타나지 않는다.

**제안**

```sql
-- 단일 질의로 통합 (LATERAL + json 집계)
SELECT t.type_id, t.name, t.kind, t.is_abstract, c.n,
       (SELECT jsonb_agg(p.name ORDER BY p.name) FROM og_catalog.type_parent tp
          JOIN og_catalog.type p ON p.type_id = tp.parent_id WHERE tp.type_id = t.type_id) AS parents,
       (SELECT jsonb_agg(jsonb_build_object('name',pr.name,'type',pr.data_type,
                                            'required',pr.required,'key',pr.is_key) ORDER BY pr.name)
          FROM og_catalog.property pr WHERE pr.type_id = t.type_id) AS props,
       (SELECT jsonb_agg(…) FROM og_catalog.role r WHERE r.rel_type_id = t.type_id) AS roles
  FROM og_catalog.type t
  LEFT JOIN LATERAL (SELECT count(*) n FROM og_data.og_node WHERE type_id = t.type_id) c ON true
 WHERE t.graph_id = $1 AND t.kind IN ('e','r')      -- ← 정원 누수 제거
 ORDER BY c.n DESC, t.name
 LIMIT $2;                                          -- ← cap 을 SQL 로 내림
```

`og_schema_for` 는 점수 계산에 필요한 프로퍼티 이름만 한 번에 가져온다
(`SELECT type_id, array_agg(name) FROM og_catalog.property WHERE type_id = ANY($1) GROUP BY 1`).

**예상 효과**: 타입 1,000개 그래프에서 SPI 질의가 O(N) → O(1). 에이전트 루프의 첫
호출 지연이 크게 줄어든다. 정확한 개선폭은 실측 필요(**미확인**).

**리스크**: 없음(순수 리팩터링). 응답 형태를 그대로 유지할 수 있다.

---

### LLM-10 · 리랭킹 단계 부재 + FTS 융합 미구현 — **Med**

**근거**: [engine/src/vector/mod.rs](../../engine/src/vector/mod.rs) 전체에 리랭킹 코드 없음;
[specs/004-vector-hybrid-search/tasks.md:29](../../specs/004-vector-hybrid-search/tasks.md)
(T014 `PostgreSQL 전문검색(FTS) 결합` 미체크);
[engine/src/compat/procs.rs:203-240](../../engine/src/compat/procs.rs) (FTS는 Cypher 표면 전용)

**현상**

- ANN이 뽑은 순위가 그대로 최종 순위다. 2단계 랭킹(cross-encoder, LLM judge, MMR)이
  들어갈 자리가 없다.
- FTS는 `db.index.fulltext.queryNodes` 로 Cypher에서만 쓸 수 있고, SQL 함수가 없다.
  `og_hybrid_search` 는 FTS 항을 갖지 않는다([vector/mod.rs:263-278](../../engine/src/vector/mod.rs)).
  spec 004 FR-019("벡터 검색과 PostgreSQL 전문검색 결과의 순위 결합")는 미구현이다.
- 결과적으로 이 DB의 "하이브리드"는 **벡터 + 그래프 근접**이지 업계 통념의
  **벡터 + 키워드**가 아니다. 문서에서 오해를 부르기 쉬운 지점이다.

**제안**

```
1. og_fulltext_search(graph, type, props text[], q text, k int) → TABLE(id, score)
   를 SQL 함수로 노출 (procs.rs 의 fulltext_query 로직 재사용)
2. og_hybrid_search 에 text_query 인자를 추가하고 세 번째 RRF 항으로 융합:
      vw/(k + vrank) + gw/(k + grank) + tw/(k + trank)
3. 리랭킹용 후보 확장 API: og_vector_search(..., k := 100) 결과에
   entity 전문을 함께 반환(이미 반환함) → 외부 리랭커가 소비
4. MMR: 후보 pool 안에서 상호 유사도로 다양화하는 옵션 (선택)
```

**예상 효과**: 고유명사·코드·식별자처럼 임베딩이 약한 질의에서 재현율이 오른다.
spec 004 SC-006(nDCG@10 +10%)의 현실적 달성 경로다.

**리스크**: PostgreSQL FTS는 `simple` 사전을 쓰므로 어간 분석·CJK 분절이 없다
([engine/src/compat/ddl.rs:253-258](../../engine/src/compat/ddl.rs)). 한국어에서는 FTS 항의
기여가 낮을 수 있다 — 그 자체가 별도 과제(사전/분절기 도입)다.

---

### LLM-11 · 임베딩 차원/모델 변경 마이그레이션 경로 부재 — **High**

**근거**: [engine/src/catalog/types.rs:539-545](../../engine/src/catalog/types.rs),
[:550](../../engine/src/catalog/types.rs),
[engine/src/vector/mod.rs:49-53](../../engine/src/vector/mod.rs),
[:69-71](../../engine/src/vector/mod.rs)

```rust
// vector/mod.rs:49-53 — 슬롯 선언은 일반 프로퍼티 경로를 재사용
Spi::run_with_args("SELECT og_add_property($1,$2,$3,$4,false,false)", …)
    .unwrap_or_else(|e| error!("failed to declare embedding property: {e}"));
// vector/mod.rs:69-71 — 카탈로그는 upsert (dims 갱신됨)
"ON CONFLICT (type_id, prop) DO UPDATE SET dims = EXCLUDED.dims, …"
```
```rust
// catalog/types.rs:539-545 — 그러나 property 는 ON CONFLICT 가 없다
"INSERT INTO og_catalog.property (…) VALUES (…)"     // UNIQUE (type_id, name) 위반
// catalog/types.rs:550 — 컬럼도 IF NOT EXISTS → 차원 불변
"ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {col} {dtype}"
```

**현상**: 모델을 바꿔 차원이 달라지면 같은 슬롯 이름으로 `og_add_embedding` 을 다시
호출할 수 없다(유니크 위반). 설령 통과해도 물리 컬럼의 `vector(N)` 차원은 그대로다.
`og_catalog.embedding.dims` 만 갱신되어 **카탈로그와 스토리지가 어긋난다** —
`og_vector_search` 의 차원 검사([vector/mod.rs:134-139](../../engine/src/vector/mod.rs))는
카탈로그 값을 쓰므로, 통과한 뒤 pgvector 단계에서 실패한다.

임베딩 슬롯을 삭제하는 함수도 없다(공개 함수 목록에 `og_drop_embedding` 부재).

**제안**

```sql
-- (a) 신규 함수
og_alter_embedding(graph, type, prop, new_dims int, new_metric text DEFAULT NULL)
-- 내부 절차:
--   1. og_catalog.property.data_type 을 vector(new_dims) 로 갱신
--   2. ALTER TABLE … ALTER COLUMN … TYPE vector(new_dims) USING NULL   (값 폐기)
--      또는 새 컬럼 추가 → 백필 → 스왑 → 구 컬럼 DROP (온라인 경로)
--   3. HNSW 인덱스 DROP/CREATE (CONCURRENTLY)
--   4. og_data.og_embedding_state 에서 해당 prop 행 삭제 (전량 stale 로 만듦)

-- (b) og_drop_embedding(graph, type, prop) — 슬롯 회수
```

**권장 운영 절차(현재 코드에서 가능한 유일한 경로)**

```sql
-- 새 이름의 슬롯을 추가하고, 백필이 끝나면 애플리케이션이 참조를 옮긴다
SELECT og_add_embedding('kb','Doc','emb_v2', 1024, 'cosine', 'body');
-- (백필)
-- 애플리케이션의 prop 인자를 'emb' → 'emb_v2' 로 교체
-- 구 슬롯은 남는다 (제거 함수 없음)
```

**예상 효과**: 임베딩 모델 교체가 스키마 재생성 없이 가능해진다. RAG 운영에서
가장 흔한 대형 변경이다.

**리스크**: `ALTER COLUMN TYPE` 은 테이블 재작성이며 ACCESS EXCLUSIVE 락을 잡는다.
spec 004 FR-027(온라인 인덱스 재구축, 그동안 검색 가능)을 지키려면 새 컬럼 스왑
방식이어야 한다. 저장소에 온라인 재구축을 검증하는 테스트가 없다(**미확인**).

---

### LLM-12 · stale 추적이 `source_prop` 에 의존, Neo4j DDL 경로에서 유실 — **Med**

**근거**: [engine/src/vector/mod.rs:308-311](../../engine/src/vector/mod.rs),
[:368](../../engine/src/vector/mod.rs),
[engine/src/compat/ddl.rs:225-228](../../engine/src/compat/ddl.rs)

```sql
-- vector/mod.rs:308-311 — source_prop 가 NULL 인 슬롯은 아예 조회 대상이 아니다
WHERE t.graph_id = $1 AND e.source_prop IS NOT NULL
```
```rust
// compat/ddl.rs:225-228 — CREATE VECTOR INDEX 는 인자 5개만 넘긴다 (source_prop 없음)
"SELECT og_add_embedding($1, $2, $3, $4, $5)", &[graph, label, prop, dims, metric]
```

**현상**: Neo4j DDL(`CREATE VECTOR INDEX`)로 만든 임베딩 슬롯은 `source_prop` 가
NULL이므로 `og_stale_embeddings` 에 **영원히 나타나지 않는다.**
`examples/meeting-rooms/load.py:48-55` 가 만드는 `room_name` 인덱스가 정확히 이 경우다.
MCP 경로로 운영하는 사용자는 staleness 추적을 전혀 받지 못한다.

`og_mark_embedded` 도 `source_prop` 가 없으면 **조용히 반환한다**
([vector/mod.rs:368](../../engine/src/vector/mod.rs)) — 오류도 경고도 없다.

추가로 `og_embedding_stats` 는 stale 비율·개수·인덱스 크기를 반환하지 않는다
([vector/mod.rs:383-408](../../engine/src/vector/mod.rs)) — spec 004 FR-029 미충족.

**제안**

```
1. CREATE VECTOR INDEX 의 OPTIONS 에 비표준 키를 수용:
     OPTIONS {indexConfig: {`vector.dimensions`: 1024,
                            `vector.similarity_function`: 'cosine',
                            `ontological.source_property`: 'descr'}}
   → ddl.rs 에서 6번째 인자로 전달
2. source_prop 가 NULL 인 슬롯은 og_embedding_stats 에
     "staleness_tracked": false 로 명시 (조용한 실패 → 관측 가능한 사실로)
3. og_mark_embedded 가 source_prop 없이 호출되면 NOTICE 를 낸다
4. og_embedding_stats 에 count / null_count / stale_count /
   pg_relation_size(index) 추가
```

**예상 효과**: spec 004 SC-007(stale 탐지율 100%)이 MCP 경로에서도 성립한다.
"조용히 추적되지 않음"이 "추적되지 않는다고 말함"으로 바뀐다.

**리스크**: Neo4j DDL에 비표준 옵션을 추가하는 것이므로, 같은 DDL을 Neo4j로 보내면
무시되거나 오류가 된다. `IF NOT EXISTS` 와 함께 문서화 필요.

---

### LLM-13 · ANN 재현율 튜닝·측정 부재 — **Med**

**근거**: 전수 grep 결과 `hnsw.ef_search` / `hnsw.iterative_scan` 설정 코드 **0건**;
`bench/harness.py` 에 `recall` 문자열 **0건**;
[engine/tests/sql/03_vector_agent_rdf.sql:34-36](../../engine/tests/sql/03_vector_agent_rdf.sql)
(비교 테스트 3행);
[specs/004-vector-hybrid-search/plan.md:86](../../specs/004-vector-hybrid-search/plan.md)

**현상**

1. HNSW 인덱스는 파라미터 없이 생성된다
   ([engine/src/vector/mod.rs:58-61](../../engine/src/vector/mod.rs)) — `m`, `ef_construction`
   전부 pgvector 기본값.
2. 질의 시 `ef_search` 를 설정하는 코드가 없다. 즉 재현율을 조절할 축이 API에 없다.
3. spec 004 FR-015("저선택도 필터에서 top-k 부족 방지, 필요 시 탐색 범위 자동 확대")를
   구현한 코드가 없다. pgvector 0.8의 iterative index scan도 켜지 않는다.
4. `og_vector_search_exact` 라는 기준선은 있지만
   ([vector/mod.rs:411-442](../../engine/src/vector/mod.rs)), 이를 이용해 재현율을 계산하는
   하네스가 없다. 존재하는 것은 3행짜리 회귀 테스트 하나다.
5. plan의 Complexity Tracking이 이 의존을 인정하고 있다:
   "재현율 보장을 pgvector `hnsw.ef_search` 튜닝에 의존".

**제안**

```sql
-- (a) 질의 시 ef_search 노출
og_vector_search(graph, type, prop, query, k, filter, ef_search int DEFAULT NULL)
-- 내부: SET LOCAL hnsw.ef_search = greatest(ef_search, k)

-- (b) 저선택도 필터에서 iterative scan 활용 (pgvector ≥ 0.8)
SET LOCAL hnsw.iterative_scan = relaxed_order;
SET LOCAL hnsw.max_scan_tuples = …;

-- (c) 인덱스 생성 파라미터 노출
og_add_embedding(…, m int DEFAULT 16, ef_construction int DEFAULT 64)
```

```python
# (d) bench/ 에 recall 하네스 추가
#   for selectivity in [1.0, 0.1, 0.01, 0.001, 0.0001]:
#       ann   = og_vector_search(..., k=10, filter=f)
#       exact = og_vector_search_exact(..., k=10)   # 같은 필터 적용 필요
#       recall@10 = |set(ann) ∩ set(exact)| / 10
#   결과를 bench/results/ 에 저장
```

**예상 효과**: SC-001(필터 통과율 0.01%에서 top-10 항상 10개, 재현율 95% 이상)을
주장이 아니라 측정치로 뒷받침할 수 있게 된다.

**리스크**: `iterative_scan` 은 pgvector 0.8+ 전용이다. plan은 "pgvector 0.8+"를
의존성으로 적고 있으나([specs/004-vector-hybrid-search/plan.md:23](../../specs/004-vector-hybrid-search/plan.md))
실제 설치본 버전은 확인해야 한다(**미확인**).
`og_vector_search_exact` 는 `filter` 인자를 받지 않으므로((b)를 위해) 시그니처 확장이
필요하다.

---

### LLM-14 · `og_vector_search` 의 `filter` 가 원시 SQL 보간 — **High**

**근거**: [engine/src/vector/mod.rs:115-118](../../engine/src/vector/mod.rs),
[:126-132](../../engine/src/vector/mod.rs)

```rust
let where_sql = match filter {
    Some(f) if !f.trim().is_empty() => format!("AND ({f})"),      // :116 — 검증 없음
    _ => String::new(),
};
let sql = format!("… WHERE v.{col} IS NOT NULL {where_sql} …");   // :129
```

**현상**: `filter` 문자열이 이스케이프·파싱 없이 SQL에 이어 붙는다.
spec 003 FR-026("사용자 값은 절대 SQL 텍스트로 보간하지 않는다")의 명시적 예외이며,
저장소 안에서 이 규칙을 깨는 유일한 사용자 대면 경로다.

GraphRAG 파이프라인에서 **LLM에게 메타데이터 필터를 생성시키는 것은 표준 패턴**이다
(self-query retriever). 그 문자열이 이 인자로 흘러들면 임의 SQL 실행이다.

완화 요인: Bolt/Cypher 경로는 `filter` 를 넘기지 않는다
([engine/src/compat/procs.rs:188-194](../../engine/src/compat/procs.rs)). 위험은 평문 SQL로
직접 호출하는 경로에 한정된다. `og_hybrid_search` 에는 `filter` 인자 자체가 없다.

**제안**

```sql
-- (a) 구조화 필터를 1급으로
og_vector_search(graph, type, prop, query, k,
                 filter     text  DEFAULT NULL,   -- 기존, 신뢰 경로 전용
                 conditions jsonb DEFAULT NULL)   -- 신규, 안전 경로
-- conditions 예: '[{"prop":"sector","op":"=","value":"manufacturing"},
--                  {"prop":"year","op":">=","value":2020}]'
-- 내부: prop 은 og_catalog.property 로 화이트리스트 검증 후 column_name() 매핑,
--       op 는 고정 집합(=, <>, <, <=, >, >=, IN, IS NULL), value 는 파라미터 바인딩
```

```sql
-- (b) 원시 경로 잠금
REVOKE EXECUTE ON FUNCTION og_vector_search(text,text,text,text,int4,text) FROM PUBLIC;
```

**예상 효과**: 에이전트 생성 필터를 안전하게 쓸 수 있게 되어, 필터 푸시다운의 이점을
RAG 파이프라인에서 실제로 활용할 수 있다.

**리스크**: `conditions` 로 표현 가능한 술어가 원시 SQL보다 좁다. 복잡한 필터가
필요한 신뢰 호출자를 위해 원시 경로는 남기되 권한으로 분리한다. 기존 호출부 수정 필요.

**당장 필요한 조치**: LLM 생성 문자열을 `filter` 로 넘기는 코드가 있다면 즉시 제거.

---

### LLM-15 · 감사 로그 불완전 — **High**

**근거**: [engine/src/cypher/mod.rs:93-99](../../engine/src/cypher/mod.rs),
[:122-135](../../engine/src/cypher/mod.rs),
[engine/src/typeql/mod.rs:100-113](../../engine/src/typeql/mod.rs);
[engine/src/vector/mod.rs](../../engine/src/vector/mod.rs)·[engine/src/agent/mod.rs](../../engine/src/agent/mod.rs)·[engine/src/compat/genai.rs](../../engine/src/compat/genai.rs) 에 `og_audit` 부재

**현상 3종**

1. **실패한 질의는 남지 않는다.**
   ```rust
   Err(e) => {
       audit(graph, query, 0, started, Some(&e));   // INSERT
       error!("cypher parse error: {e}")            // ← 트랜잭션 중단 → INSERT 롤백
   }
   ```
   서브트랜잭션/자율 트랜잭션이 없으므로 `error_code` 컬럼은 사실상 항상 NULL이다.
   [docs/agents.md:138-140](../../docs/agents.md) 및 [docs/api.md:184-185](../../docs/api.md) 의
   "records … error code" 서술과 어긋난다.
2. **파라미터가 기록되지 않는다.** 기록되는 값은 `format!("[{graph}] {query}")` 뿐이다
   ([cypher/mod.rs:128](../../engine/src/cypher/mod.rs)). 설계상 모든 사용자 값이
   파라미터로 들어가므로, 감사 로그만으로 "에이전트가 무엇을 조회했는지"를 재구성할 수 없다.
3. **감사 대상이 좁다.** `og_cypher` / `og_typeql` 만 기록된다.
   `og_typeql_script`, `og_vector_search`, `og_similar`, `og_hybrid_search`,
   `og_schema`, `og_estimate`, `og_explain_error`, `og_genai_encode`(외부 HTTP!),
   모든 DDL 함수는 기록되지 않는다.

   spec 008 SC-008("모든 질의 실행의 100%가 감사 로그에 기록된다")은 충족되지 않는다.
   FR-028(민감정보 마스킹 정책)도 미구현이다.

**제안**

```rust
// (a) 실패 기록: 서브트랜잭션으로 감사 INSERT 를 보호
//     pgrx::PgTryBuilder 로 감싸거나, 실패 경로는 ereport(LOG) 로 병행 기록
//     (로그는 트랜잭션 롤백의 영향을 받지 않는다)
// (b) 감사 대상 확대: vector/agent/genai 진입점에 audit() 추가.
//     lang 컬럼에 'vector' | 'agent' | 'genai' 를 쓴다
// (c) params 를 별도 컬럼(jsonb)으로 저장하되, 마스킹 정책을 설정으로:
//     audit.params = 'off' | 'hashed' | 'full'   (기본 'hashed')
// (d) 보존 정책: audit.retain_days 와 정리 함수 og_audit_prune()
```

**예상 효과**: "에이전트가 무엇을 했는가"가 사후 검토 가능해진다. 특히 **실패한**
질의가 남는 것이 중요하다 — 에이전트의 재시도 루프 병리를 진단할 유일한 근거다.

**리스크**: 감사 쓰기가 읽기 질의마다 발생하므로 처리량에 영향을 준다(현재도 그렇다).
파라미터 저장은 PII 정책과 직결되므로 기본값을 보수적으로.

**당장 가능한 대안**: PostgreSQL의 `log_statement` 또는 `pgaudit` 확장을 병행한다.
DB 레벨 로깅은 트랜잭션 롤백의 영향을 받지 않는다.

---

### LLM-16 · groundedness 검증 훅 및 결과 단위 provenance 부재 — **Med**

**근거**: [specs/008-agent-native-interface/tasks.md:27-28](../../specs/008-agent-native-interface/tasks.md)
(T014/T015 미체크); `og_cypher_provenance` 는 공개 함수 목록에 없음;
[specs/008-agent-native-interface/plan.md:27](../../specs/008-agent-native-interface/plan.md)
(plan에는 함수 이름이 있음)

**현상**: spec 008 FR-012~FR-014(질의 결과 행 → 기여 노드/엣지/경로, 추론 근거,
집계 원본 전개)가 전부 미구현이다. 제공되는 것은 **엔티티 단위 메타데이터**
(`og_data.og_source`, 엔티티당 1행,
[engine/sql/bootstrap.sql:324-330](../../engine/sql/bootstrap.sql))뿐이다.

RAG 관점에서:
- 다중 홉 질의 결과가 어떤 경로를 통해 나왔는지 되짚을 수 없다.
- 집계 결과(`count`, `avg`)의 기여 엔티티를 전개할 수 없다.
- 따라서 "답변이 검색된 문서에 근거하는가"를 판정하는 노드에 넘길 **구조화된 근거**가
  없다. 애플리케이션은 반환된 엔티티 JSON 전문에 의존해야 한다.

**제안**

```sql
-- (a) 결과 행별 기여 id 집합 (FR-016: 끄면 오버헤드 0 — 컴파일러가 다른 SQL 을 낸다)
og_cypher_provenance(graph, query, params jsonb DEFAULT '{}')
  → TABLE(row jsonb, node_ids int8[], edge_ids int8[], path jsonb)
-- 컴파일러가 프로젝션에 각 패턴 변수의 id 를 추가하는 모드를 갖는다

-- (b) 출처를 한 번에 얹는 헬퍼
og_sources(ids int8[]) → TABLE(entity_id, source, confidence, author, ingested_at)

-- (c) 크기 상한 (FR-017)
--     provenance.max_ids (default 1000) 초과 시 절단 + truncated 표기
```

**예상 효과**: groundedness 판정 노드의 입력이 `(답변, 근거 id 집합, 출처 메타)` 로
구조화된다. 현재는 애플리케이션이 직접 조립해야 한다.

**리스크**: spec 008 SC-004는 "출처 추적 활성화 시 질의 지연 증가 50% 이내"를 요구한다.
프로젝션 확장은 그 안에 들어갈 가능성이 높지만 실측이 필요하다(**미확인**).
비활성 시 오버헤드 0은 컴파일 분기로 보장 가능하다.

**참고**: groundedness "판정" 자체는 DB 범위 밖이다(답변 텍스트가 DB에 없다).
DB가 제공할 것은 판정 재료이며, 이 항목은 그 재료의 완성도에 관한 것이다.

---

### LLM-17 · `og.max_rows` 가 강제되지 않음 — **Med**

**근거**: [engine/src/agent/mod.rs:437-438](../../engine/src/agent/mod.rs) (설정만),
읽는 코드 전수 grep **0건**;
[specs/008-agent-native-interface/tasks.md:22](../../specs/008-agent-native-interface/tasks.md) (T011 미착수)

```rust
if let Some(rows) = limits.get("max_rows").and_then(|v| v.as_i64()) {
    Spi::run(&format!("SET og.max_rows = {rows}")).ok();      // 설정하고 끝
}
```

**현상**: 사용자 정의 GUC를 설정하지만 아무도 읽지 않는다. `og_create_role` 문서와
예제([docs/agents.md:133](../../docs/agents.md))는 `max_rows` 를 상한처럼 제시하지만
**결과 행 수 상한은 존재하지 않는다.** spec 008 FR-024가 요구하는 네 축(시간, 메모리,
방문 노드/엣지 수, 결과 행 수) 중 시간·메모리만 강제된다.

(주의: `docs/deep-traversal.md` 의 `max_rows` 는 pgGraph 벤치마크 파라미터로
무관하다 — [bench/harness.py:603,679](../../bench/harness.py).)

**제안**

```rust
// (a) GUC 를 정식 등록한다 (pgrx GucRegistry) — 임의 SET 이 아니라 타입/범위를 갖게
// (b) compile_read 의 최종 SELECT 에 LIMIT 가드를 주입한다 (T011)
//       기존 LIMIT 가 있으면 least(existing, og.max_rows)
//       가드가 걸렸으면 og_cypher_stats() 에 "truncated": true 를 남긴다
// (c) 또는 og_cypher 가 반환 행 수를 세다가 상한에서 멈추고 경고를 낸다
//     (컴파일 주입보다 단순하지만 SQL 은 이미 다 실행된 뒤다)
```

**예상 효과**: spec 008 SC-007("폭주 질의 50종 전부가 상한에서 차단")의 결과 크기 축이
성립한다. 에이전트가 만든 데카르트 곱이 클라이언트 메모리를 터뜨리는 것을 막는다.

**리스크**: 기존 질의 결과가 조용히 잘릴 수 있다. **잘렸다는 사실을 반드시 반환해야
한다** — 조용한 절단은 이 프로젝트가 다른 곳에서 피하고 있는 실패 양식이다
(`og_schema` 의 `truncated`, `og_as_of` 의 오류가 같은 원칙).

---

### LLM-18 · `og_hybrid_search` 가 관계 타입과 필터를 지원하지 않음 — **Low**

**근거**: [engine/src/vector/mod.rs:222-231](../../engine/src/vector/mod.rs),
[:247](../../engine/src/vector/mod.rs), [:276](../../engine/src/vector/mod.rs)

```rust
let view = crate::cypher::views::ensure_view(tid, false);   // :247 — is_edge 고정
…
"SELECT id, fscore::float8, vscore::float8, gscore::float8, og_node_json(id) …"  // :276
```

**현상**

1. `is_edge = false` 가 고정되어 있고 JSON 변환도 `og_node_json` 고정이다.
   `og_vector_search`/`og_similar` 는 `type_kind(tid) == 'r'` 로 분기하는데
   ([vector/mod.rs:111-113,169-172](../../engine/src/vector/mod.rs))
   `og_hybrid_search` 만 하지 않는다. 관계 타입을 넘기면 노드 뷰를 찾는다.
   spec 004의 차별 기능(FR-002, 관계 임베딩 1급)이 하이브리드에서 빠진다.
2. `filter` 인자가 없다. 하이브리드 경로에는 메타데이터 필터링 수단이 아예 없다.
3. `anchor` 가 없으면 `prox` CTE가 `WHERE false` 로 비고
   ([vector/mod.rs:255](../../engine/src/vector/mod.rs)) 그래프 항이 0이 된다 — 즉
   `anchor` 없는 `og_hybrid_search` 는 순수 벡터 검색과 순서가 같다. 이 경우 함수를
   부를 이유가 없는데, 그 사실이 문서화되어 있지 않다.

**제안**

```rust
// (a) is_edge 분기 도입
let is_edge = types::type_kind(tid) == 'r';
let view    = views::ensure_view(tid, is_edge);
let json_fn = if is_edge { "og_edge_json" } else { "og_node_json" };
// 엣지의 그래프 근접은 양 끝 노드 중 가까운 쪽으로 정의 (문서화 필요)

// (b) filter 인자 추가 — LLM-14 의 conditions jsonb 형태로
// (c) anchor 가 NULL 이면 NOTICE 로 "그래프 항이 비활성"임을 알린다
```

**예상 효과**: 관계 임베딩이 검색 표면 전체에서 1급이 된다.

**리스크**: 시그니처 확장(기본값으로 하위 호환 가능). 엣지의 "그래프 근접" 정의를
새로 정해야 하며, 이는 설계 결정이다.

---

## 3. 우선순위

### 즉시 (보안·정확성)

| 순위 | ID | 이유 |
|---|---|---|
| 1 | **LLM-08** | 자격증명 노출 + SSRF. 코드 변경 없이 `REVOKE` 로 대부분 완화 가능 |
| 2 | **LLM-14** | LLM 생성 필터를 넘기는 순간 임의 SQL 실행 |
| 3 | **LLM-15** | 실패 질의가 감사에 안 남는 것은 사고 조사 자체를 불가능하게 한다 |

### 단기 (에이전트 루프의 실효성)

| 순위 | ID | 이유 |
|---|---|---|
| 4 | **LLM-03** | 오타 레이블은 LLM Cypher 오류 1위. 지금은 교정 정보가 JSON으로 안 온다 |
| 5 | **LLM-02** | 예산이 지켜지지 않으면 스키마가 컨텍스트에서 잘려 나가고, 그러면 LLM-03이 더 자주 터진다 |
| 6 | **LLM-01** | 하이브리드 랭킹이 현재로선 튜닝 축이 없다. 회피책은 `graph_weight` 조정 |
| 7 | **LLM-06** | 임베딩 호출이 한 번 실패하면 트랜잭션 전체가 죽는다 |

### 중기 (운영)

LLM-11(모델 교체) → LLM-13(재현율 측정) → LLM-17(행 수 상한) →
LLM-07(배치) → LLM-12(stale) → LLM-09(N+1) → LLM-04/05(진단 정확도) →
LLM-10(FTS/리랭킹) → LLM-16(provenance) → LLM-18(관계 하이브리드)

### 이 목록에 없는 것

- **청킹 계층 도입**: 개선 항목으로 올리지 않았다. 그래프 노드를 청크로 모델링하는
  것이 이 DB의 설계 전제이고([06_retrieval_and_rrf.md](06_retrieval_and_rrf.md) 3.1절),
  DB 안에 문서 분할기를 넣는 것은 spec 004의 범위 결정
  ([specs/004-vector-hybrid-search/spec.md:269-270](../../specs/004-vector-hybrid-search/spec.md))과
  충돌한다. 애플리케이션이 `Chunk` 타입을 만드는 것이 정합적인 해법이다.
- **답변 생성·환각 판정 노드**: 범위 밖(spec 008 Assumptions). DB가 제공할 것은
  판정 재료이며 그 완성도는 LLM-16이 다룬다.

---

## 4. 참고

- 각 항목의 배경은 해당 세부 문서에 있다:
  [02](02_schema_introspection.md) · [03](03_correctable_errors.md) ·
  [04](04_dry_run_and_estimate.md) · [05](05_embedding_pipeline.md) ·
  [06](06_retrieval_and_rrf.md) · [07](07_grounding_and_provenance.md) ·
  [08](08_guardrails_and_roles.md)
- 스펙: [specs/004-vector-hybrid-search/](../../specs/004-vector-hybrid-search/),
  [specs/008-agent-native-interface/](../../specs/008-agent-native-interface/)

<!-- affects: llm, data, backend, security, ops -->
<!-- requires-update: 02_api/00_index.md, 99_decisions/ -->
