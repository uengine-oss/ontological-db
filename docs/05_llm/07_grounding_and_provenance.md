# 07. 근거와 출처 — `og_set_source` / `og_history` / `og_as_of`

> **이 문서가 답하는 질문**
> - 답변에 인용을 붙이려면 이 DB에서 무엇을 꺼낼 수 있는가?
> - 출처 메타데이터는 어디에 어떤 형태로 저장되는가?
> - 시점 질의(`og_as_of`)는 무엇을 보장하고 무엇을 보장하지 않는가?
> - RAG 환각 방지 관점에서 이 표면으로 **할 수 있는 것과 없는 것**은?

---

## 1. 사실 — 근거 재료 3종

| 재료 | 함수 / 테이블 | 무엇을 답하는가 |
|---|---|---|
| 출처 메타데이터 | `og_set_source` → `og_data.og_source` | "이 엔티티는 어디서 왔는가" |
| 변경 이력 | `og_enable_history` / `og_history` → `og_data.og_history` | "이 엔티티는 언제 어떻게 바뀌었는가" |
| 시점 상태 | `og_as_of` | "그 시점 기준으로 무엇이 참이었는가" |

여기에 **미구현** 항목이 하나 있다: 질의 결과 행별 기여 노드/엣지/경로 추적
(`og_cypher_provenance`, spec 008 FR-012~FR-014). plan에는 함수 이름까지 있으나
([specs/008-agent-native-interface/plan.md:27](../../specs/008-agent-native-interface/plan.md)),
공개 함수 목록에 없고 tasks.md의 T014/T015가 미체크다
([specs/008-agent-native-interface/tasks.md:27-28](../../specs/008-agent-native-interface/tasks.md)).

---

## 2. 사실 — `og_set_source`

정의: [engine/src/agent/mod.rs:529-545](../../engine/src/agent/mod.rs).

```sql
SELECT og_set_source(
  <entity_id>,                          -- int8
  'https://example.org/doc/42',         -- source
  0.92,                                 -- confidence (real, 기본 NULL)
  'ingest-v3'                           -- author (기본 NULL)
);
```

저장 테이블 ([engine/sql/bootstrap.sql:324-330](../../engine/sql/bootstrap.sql)):

```sql
CREATE TABLE og_data.og_source (
    entity_id   int8 PRIMARY KEY,
    source      text,
    ingested_at timestamptz NOT NULL DEFAULT now(),
    confidence  real,
    author      text
);
```

특성:

- **엔티티당 1행**(`entity_id PRIMARY KEY`). upsert이므로 재호출 시 덮어쓴다
  ([agent/mod.rs:539-541](../../engine/src/agent/mod.rs)). 한 엔티티가 여러 문서에서 왔다는
  것을 표현할 수 없다.
- 노드/엣지 구분이 없다. `entity_id` 는 두 공간을 함께 쓰는 int8 식별자다
  ([engine/src/id.rs](../../engine/src/id.rs) 참조).
- `og_data.og_node` / `og_data.og_edge` 에 대한 외래키가 없다. 엔티티가 삭제돼도
  `og_source` 행은 남는다(고아 행). 정리 함수는 없다.
- **조회 함수가 없다.** `og_get_source` 같은 것이 없으므로 평문 SQL로 읽는다:

```sql
-- RAG 답변에 인용 붙이기: 검색 결과 id 집합에 출처를 조인
SELECT r.id,
       r.entity ->> 'title'  AS title,
       r.score,
       s.source, s.confidence, s.author, s.ingested_at
  FROM og_vector_search('kb', 'Doc', 'emb', $1, 10) r
  LEFT JOIN og_data.og_source s ON s.entity_id = r.id
 ORDER BY r.score DESC;
```

- 백업 대상으로 등록되어 있다
  ([engine/sql/bootstrap.sql:430](../../engine/sql/bootstrap.sql)).

---

## 3. 사실 — 히스토리

### 3.1 `og_enable_history(graph, type_name)` — 타입 단위 opt-in

정의: [engine/src/agent/mod.rs:448-468](../../engine/src/agent/mod.rs).

대상 타입과 **모든 서브타입**의 저장 테이블에 트리거를 건다:

```sql
CREATE OR REPLACE TRIGGER og_hist_{type_id}
  AFTER INSERT OR UPDATE OR DELETE ON {table}
  FOR EACH ROW EXECUTE FUNCTION og_capture_history()
```

그리고 `og_catalog.setting` 에 `history.{graph}.{type_name} = 'on'` 을 기록한다
([agent/mod.rs:462-467](../../engine/src/agent/mod.rs)).

**기본은 꺼짐**이다 — 이유는 주석에 있다
([agent/mod.rs:447](../../engine/src/agent/mod.rs)): "Off by default: it costs writes."

**주의점 (전부 코드에서 확인)**

- 트리거는 **호출 시점에 존재하는 서브타입**에만 걸린다
  ([agent/mod.rs:452](../../engine/src/agent/mod.rs) — `og_subtypes(tid)` 를 순회).
  나중에 만들어진 서브타입은 자동으로 이력을 남기지 않는다. 재호출이 필요하다.
- **끄는 함수가 없다.** `og_disable_history` 는 공개 함수 목록에 없다.
- 보존 정책(기간·용량 상한, FR-023)이 없다. `og_data.og_history` 는 무한 증가한다.
  청소 함수도 없다.
- `og_history_entity_idx (entity_id, recorded_at DESC)` 와
  `og_history_valid_idx (valid_from, valid_to)` 가 있다
  ([engine/sql/bootstrap.sql:321-322](../../engine/sql/bootstrap.sql)).

### 3.2 `og_capture_history()` 트리거의 실제 동작

[engine/sql/access.sql:274-295](../../engine/sql/access.sql):

```sql
IF TG_OP = 'DELETE' THEN eid := OLD.id; op := 'd'; doc := to_jsonb(OLD);
ELSE eid := NEW.id;
     op  := CASE TG_OP WHEN 'INSERT' THEN 'i' ELSE 'u' END;
     doc := to_jsonb(NEW);
END IF;

UPDATE og_data.og_history SET valid_to = now()
 WHERE entity_id = eid AND valid_to IS NULL;

INSERT INTO og_data.og_history (entity_id, is_edge, op, payload)
VALUES (eid, doc ? 'src', op, doc);
```

- `payload` 는 **행 전체의 jsonb 스냅샷**이다(델타가 아님). 컬럼 이름은 물리 컬럼명
  (`p_title`, `p_emb` 등)이며 논리 프로퍼티 이름이 아니다.
  **임베딩 벡터 컬럼도 스냅샷에 포함된다** — 1536차원 벡터 하나가 매 UPDATE마다
  jsonb로 복제된다.
- `is_edge` 판정은 `doc ? 'src'` — 페이로드에 `src` 키가 있는지로 결정된다.
  `src` 라는 이름의 사용자 프로퍼티를 가진 **노드**는 엣지로 오분류된다.
- `valid_from` / `valid_to` 는 트리거가 관리하지만 이 값은 **기록 시각(transaction
  time)** 계열이다. spec 008 FR-019가 요구하는 valid time(사실의 유효 기간)과
  transaction time의 분리 질의는 미구현이다(T019 미체크,
  [specs/008-agent-native-interface/tasks.md:34](../../specs/008-agent-native-interface/tasks.md)).

### 3.3 `og_history(id)`

정의: [engine/src/agent/mod.rs:471-499](../../engine/src/agent/mod.rs). `STABLE`.

```sql
SELECT recorded_at, op, payload FROM og_history(<entity_id>);
```

- `recorded_at DESC` 정렬 ([agent/mod.rs:484](../../engine/src/agent/mod.rs)).
- **`LIMIT` 인자가 없다.** 고빈도 갱신 엔티티는 전체 이력이 반환된다.
  spec 008 Edge Cases의 "출처 추적의 폭증" 상한(FR-017)은 미구현.
- `op` 는 `'i'` / `'u'` / `'d'` 단일 문자다.

### 3.4 `og_as_of(id, at)` — 이력이 없으면 오류

정의: [engine/src/agent/mod.rs:502-526](../../engine/src/agent/mod.rs).

```rust
// engine/src/agent/mod.rs:504-516
let tracked = one::<bool>("SELECT true FROM og_data.og_history WHERE entity_id = $1 LIMIT 1", …);
if !tracked {
    error!("no history is retained for entity {id}. enable it with \
            og_enable_history(graph, type) — returning the current value instead would be a lie");
}
```

그 다음 해당 시점 이하의 가장 최근 스냅샷을 반환한다
([agent/mod.rs:517-525](../../engine/src/agent/mod.rs)):

```sql
SELECT payload FROM og_data.og_history
 WHERE entity_id = $1 AND recorded_at <= $2
 ORDER BY recorded_at DESC LIMIT 1
```

**결정(Decision)**: 이력 없는 엔티티에 대해 **현재 값을 반환하지 않고 오류를 던진다.**
FR-021의 직접 구현이며, RAG 관점에서 가장 중요한 안전장치 중 하나다 — 시점 질문에
현재 값을 조용히 돌려주는 것이 지식베이스 신뢰를 무너뜨리는 방식이기 때문이다.

**보장하지 않는 것**

- 이력이 켜지기 **이전**의 시점을 물으면 `NULL` 이 반환된다
  ([agent/mod.rs:525](../../engine/src/agent/mod.rs) — `unwrap_or(JsonB(Value::Null))`).
  "이력이 있지만 그 시점 이전"과 "그 시점에 존재하지 않았음"이 구분되지 않는다.
- **엔티티 1개 단위**다. "3개월 전 기준 이 회사의 자회사는?" 같은 그래프 시점 질의는
  지원되지 않는다. plan이 구상한 "히스토리에서 해당 시점 상태를 재구성한 임시 뷰로
  질의를 실행"([specs/008-agent-native-interface/plan.md:35-36](../../specs/008-agent-native-interface/plan.md))
  은 구현되지 않았다.
- 반환 `payload` 는 물리 컬럼명 기반 jsonb다. `og_node_json` 형식이 아니므로
  Cypher 결과와 형태가 다르다.
- 스키마가 바뀐 뒤의 과거 시점 조회 시 컬럼 집합이 현재와 다를 수 있다
  (spec 008 Edge Cases "시점 질의와 스키마 변경" — 처리 정의 없음).

---

## 4. 결정(Decision) — RAG 환각 방지 관점에서의 활용

이 DB가 제공하는 것은 **판정기가 아니라 판정 재료**다. 그 재료로 조립할 수 있는
방어선은 다음 네 가지다.

### D-1. 인용 강제 (citation grounding)

검색 결과에 `og_source` 를 조인해, **출처가 없는 엔티티는 답변 근거에서 제외**한다.
DB는 이 정책을 강제하지 않는다 — 애플리케이션이 한다.

```sql
SELECT r.id, r.entity, r.score, s.source, s.confidence
  FROM og_vector_search('kb','Doc','emb', $1, 20) r
  JOIN og_data.og_source s ON s.entity_id = r.id      -- INNER JOIN = 출처 필수
 WHERE s.confidence IS NULL OR s.confidence >= 0.8
 ORDER BY r.score DESC LIMIT 10;
```

### D-2. 신뢰도 게이트

`og_source.confidence` (`real`)를 임계값으로 쓴다. 이 값은 `og_set_source` 호출자가
넣은 것이며 DB가 계산하지 않는다 — 의미는 적재 파이프라인이 정의한다.

### D-3. 시점 고정 (temporal pinning)

시점을 묻는 질문에는 `og_as_of` 를 쓰고, 오류가 나면 **"그 시점의 상태를 모른다"고
답하게** 한다. 현재 값으로 대체하지 않는 것이 핵심이며, DB가 이미 그렇게 강제한다
(3.4절).

### D-4. 변경 감지

`og_history(id)` 의 최신 `recorded_at` 이 답변 캐시 생성 시각보다 나중이면 답을
무효화한다. 엔티티 단위이므로 여러 엔티티를 근거로 쓴 답변은 각각 확인해야 한다.

---

## 5. 사실 — RAG 관점에서 **없는** 방어선

| 방어선 | 상태 |
|---|---|
| 질의 결과 행 → 기여 노드/엣지/경로 매핑 (FR-012) | ❌ 미구현 |
| 추론된 사실의 도출 규칙·전제 반환 (FR-013) | ❌ 미구현. `og_add_rule` 로 규칙 선언은 가능하나 근거 반환 없음 |
| 집계 결과의 원본 엔티티 전개 (FR-014) | ❌ 미구현 |
| 출처 정보 크기 상한 (FR-017) | ❌ 미구현 |
| groundedness / 근거성 판정 훅 | ❌ 없음 — DB에 답변 텍스트가 없으므로 원리상 DB 범위 밖 |
| 답변–질문 정합성 검사 | ❌ 범위 밖 |
| 이력 보존 정책(기간/용량) (FR-023) | ❌ 미구현 |
| 감사 로그의 민감정보 마스킹 (FR-028) | ❌ 미구현 ([08_guardrails_and_roles.md](08_guardrails_and_roles.md) 4절) |

**결론(사실)**: self-correction loop의 "Hallucination/Relevance Check" 노드는
이 DB 안에 없다. 애플리케이션이 구현해야 하며, DB가 주는 입력은
`(id, score, entity, source, confidence, recorded_at)` 이다.

---

## 6. 필수(Required) / 금지(Forbidden)

**필수**

- 적재 파이프라인은 노드 생성 직후 `og_set_source` 를 호출할 것. 사후 보강은
  `entity_id` 를 다시 찾아야 하고, 그 사이의 답변에는 근거가 없다.
- `og_enable_history` 는 **서브타입을 추가할 때마다 재호출**할 것 (3.1절).
- 시점 질문에는 `og_as_of` 를 쓰고, 오류를 "모른다"로 번역할 것. 절대 현재 값으로
  대체하지 말 것.
- `og_as_of` 가 `NULL` 을 반환한 경우와 오류를 던진 경우를 **구분해서 처리**할 것 (3.4절).
- 이력을 켠 타입은 `og_data.og_history` 크기를 모니터링할 것. 보존 정책이 없다.

**금지**

- 임베딩 컬럼을 가진 타입에 이력을 켤 때 벡터가 매 UPDATE마다 jsonb로 복제된다는 점을
  무시하지 말 것 (3.2절). 필요하면 임베딩을 별도 타입/테이블로 분리한다.
- `og_source` 를 다중 출처 저장소로 쓰려 하지 말 것. `entity_id` 가 PK다 (2절).
- `og_history(id)` 를 상한 없이 에이전트 컨텍스트에 넣지 말 것 (3.3절).
- 이 DB가 "출처 추적(provenance)"을 제공한다는 서술을 **질의 결과 단위 추적**으로
  확대 해석하지 말 것. 제공되는 것은 **엔티티 단위 메타데이터**다 (5절).
- 노드 프로퍼티 이름으로 `src` 를 쓰지 말 것 — 이력의 `is_edge` 판정이 오작동한다 (3.2절).

---

## 7. 참고

- 원문: [docs/agents.md:142-154](../../docs/agents.md) "Cite the answer"
- 함수 계약: [docs/api.md:179-182](../../docs/api.md)
- 스펙: FR-012~FR-023
  ([specs/008-agent-native-interface/spec.md:235-256](../../specs/008-agent-native-interface/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-16

<!-- affects: llm, data, backend -->
<!-- requires-update: 02_api/00_index.md -->
