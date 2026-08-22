# 02. 스키마 인트로스펙션 — `og_schema` / `og_schema_for`

> **이 문서가 답하는 질문**
> - `og_schema` 의 `token_budget` 은 정확히 무엇을 어떻게 자르는가?
> - 예산을 넘겼을 때 **무엇이 남고 무엇이 사라지는가**?
> - `og_schema_for` 의 "관련성"은 어떻게 계산되는가?
> - 이 두 함수를 에이전트 파이프라인에서 쓸 때의 함정은?

---

## 1. 사실 — `og_schema(graph, token_budget)` 의 실제 로직

정의: [engine/src/agent/mod.rs:21-113](../../engine/src/agent/mod.rs). `STABLE`.

### 1.1 타입 수집과 정렬

```sql
-- engine/src/agent/mod.rs:38-42
SELECT t.type_id, t.name, t.kind::text, t.is_abstract,
       COALESCE((SELECT count(*) FROM og_data.og_node n WHERE n.type_id = t.type_id), 0)
     + COALESCE((SELECT count(*) FROM og_data.og_edge e WHERE e.type_id = t.type_id), 0)
  FROM og_catalog.type t WHERE t.graph_id = $1
 ORDER BY 5 DESC, t.name
```

- **중요도 = 인스턴스 수**다. 5번 컬럼(노드 수 + 엣지 수) 내림차순, 동수면 이름 오름차순.
- 이 카운트는 **서브타입을 롤업하지 않는다.** `t.type_id` 정확 일치만 센다. 즉 추상
  상위 타입은 인스턴스 0으로 계산되어 정렬 하위에 배치된다.

### 1.2 절단 계산 — 여기가 핵심

```rust
// engine/src/agent/mod.rs:60-63
// ~30 tokens per type description is a deliberate under-estimate: better to
// return slightly less than the budget than to blow past it.
let cap = token_budget.map(|b| ((b as usize) / 30).max(8)).unwrap_or(usize::MAX);
let truncated = total > cap;
```

| 입력 `token_budget` | `cap` (반환될 타입 개수) |
|---|---|
| `NULL` (기본값) | `usize::MAX` — 절단 없음 |
| `100` | `max(3, 8)` = **8** |
| `240` | `8` |
| `800` | `26` |
| `4000` | `133` |
| `16000` | `533` |

**결정(Decision)**: 예산은 **타입 1개 ≈ 30 토큰**이라는 고정 환산으로 타입 *개수*로만
변환된다. 실제 출력 크기는 측정하지 않는다.

### 1.3 잘려나가는 것과 남는 것

| 항목 | 예산 초과 시 | 근거 |
|---|---|---|
| 인스턴스 수 하위 타입 전체 | **잘림** | [agent/mod.rs:65](../../engine/src/agent/mod.rs) `.take(cap)` |
| 남은 타입의 프로퍼티 목록 | **전부 남음 (상한 없음)** | [agent/mod.rs:66-67, 130-152](../../engine/src/agent/mod.rs) |
| 남은 관계 타입의 role 목록 | **전부 남음 (상한 없음)** | [agent/mod.rs:74, 154-183](../../engine/src/agent/mod.rs) |
| 부모 타입(`extends`) 목록 | **전부 남음 (상한 없음)** | [agent/mod.rs:66, 115-128](../../engine/src/agent/mod.rs) |
| `notes` 배열 3줄 | 항상 포함 | [agent/mod.rs:93-100](../../engine/src/agent/mod.rs) |
| `schema_version` | 항상 포함 | [agent/mod.rs:24-30, 90](../../engine/src/agent/mod.rs) |
| `truncated` 객체 | 절단이 일어났을 때만 | [agent/mod.rs:101-111](../../engine/src/agent/mod.rs) |

**따라서 예산은 상한이 아니다.** 프로퍼티가 200개인 타입 하나가 예산을 통째로 넘길 수
있다. spec 008 SC-003("타입 1,000개 스키마가 4,000 토큰 이내로 요약")을 강제하는 코드는
없다 — 저장소에 그 측정을 수행하는 하네스도 없다(미확인이 아니라 부재).

### 1.4 조용히 사라지는 세 번째 종류

```rust
// engine/src/agent/mod.rs:68-85
if kind == "r" { relations.push(...) }
else if kind == "e" { entities.push(...) }
```

`kind` 가 `'r'`(relation)도 `'e'`(entity)도 아닌 타입 — 예를 들어 TypeQL이 만드는
attribute 타입(`'a'`, 저장 테이블 `og_data.a_*`,
[engine/src/cypher/views.rs:67-68](../../engine/src/cypher/views.rs) 참조) — 은
**`take(cap)` 의 정원을 소비하지만 두 배열 중 어디에도 담기지 않는다.**
그래프에 attribute 타입이 많으면 예산이 보이지 않는 곳으로 새어 나간다.

### 1.5 응답 형태

```json
{
  "graph": "meeting",
  "schema_version": 42,
  "entity_types": [
    { "name": "MeetingRoom", "abstract": false, "extends": [], "instances": 5,
      "properties": [
        { "name": "name", "type": "text", "required": true, "key": true },
        { "name": "seats", "type": "int4", "required": false, "key": false }
      ] }
  ],
  "relation_types": [
    { "name": "FOR_ROOM", "abstract": false, "extends": [], "instances": 8,
      "roles": [
        { "name": "reservation", "player_type": "Reservation", "position": "source", "min": 0, "max": null },
        { "name": "room", "player_type": "MeetingRoom", "position": "target", "min": 0, "max": null }
      ],
      "properties": [] }
  ],
  "notes": [
    "A label matches all of its subtypes: MATCH (v:Vehicle) also returns Car and EV.",
    "Relationship direction matters. Check `roles` for which type sits at each end.",
    "Parameters use $name and are passed as the third argument to og_cypher."
  ],
  "truncated": {
    "shown": 133, "total": 1042,
    "ordered_by": "instance count, descending",
    "hint": "call og_schema_for(graph, question) for the types relevant to one question"
  }
}
```

필드별 생성 위치: `properties` 는 `og_catalog.property` 의
`(name, data_type, required, is_key)`
([agent/mod.rs:134-137](../../engine/src/agent/mod.rs)),
`roles` 는 `og_catalog.role` 의 `(name, player_type_id→name, ordinal, card_min, card_max)`
([agent/mod.rs:158-161](../../engine/src/agent/mod.rs)).
`position` 은 `ordinal` 을 매핑한 것: `0 → "source"`, `1 → "target"`, 그 외 `"additional"`
([agent/mod.rs:170-174](../../engine/src/agent/mod.rs)).

### 1.6 스펙 대비 미구현

spec 008 FR-003은 "타입별 통계(인스턴스 수, **프로퍼티 값 분포, 카디널리티, 대표 값 예시**)"
를 요구한다([specs/008-agent-native-interface/spec.md:214-215](../../specs/008-agent-native-interface/spec.md)).
구현된 것은 인스턴스 수뿐이다. role의 `min`/`max` 는 카디널리티에 해당하지만 프로퍼티
카디널리티(`og_catalog.property.card_min/card_max`,
[engine/sql/bootstrap.sql](../../engine/sql/bootstrap.sql) `property` 테이블 정의)는 응답에
포함되지 않는다. 값 분포와 예시 값은 없다.

FR-006("자주 쓰이는 질의 패턴 예시")도 미구현이다.

---

## 2. 사실 — `og_schema_for(graph, question)` 의 실제 로직

정의: [engine/src/agent/mod.rs:189-258](../../engine/src/agent/mod.rs). `STABLE`.
모듈 주석이 스스로 한계를 명시한다
([agent/mod.rs:187-188](../../engine/src/agent/mod.rs)):
"Lexical overlap only. It is not trying to be clever."

### 2.1 토크나이즈

```rust
// engine/src/agent/mod.rs:192-196
question.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
```

- 분할 기준은 **비영숫자**. 한글은 `is_alphanumeric()` 이 `true` 이므로 한 어절이 하나의
  토큰이 된다. `"라일락" 회의실을 어제 예약했던 사람 목록` →
  `["라일락", "회의실을", "어제", "예약했던", "사람", "목록"]`.
- `w.len() >= 3` 은 **바이트 길이**다(`str::len`). 한글 1글자 = UTF-8 3바이트이므로
  한글은 1글자부터 통과하고, 영어는 3글자부터 통과한다.
- `to_ascii_lowercase()` 는 ASCII만 접는다. 비ASCII는 그대로.

### 2.2 점수 계산

```rust
// engine/src/agent/mod.rs:216-228
for w in &words {
    if lname.contains(w) || w.contains(&lname) { score += 10; }
    else if types::edit_distance(&lname, w) <= 2 { score += 4; }
}
for p in property_list(tid) {
    if words.iter().any(|w| pn.contains(w) || w.contains(&pn)) { score += 3; }
}
```

| 신호 | 점수 |
|---|---|
| 타입 이름과 단어가 서로를 부분문자열로 포함 | +10 (단어당) |
| 타입 이름과 단어의 편집거리 ≤ 2 | +4 (단어당) |
| 프로퍼티 이름과 단어가 서로를 부분문자열로 포함 | +3 (프로퍼티당) |

`score > 0` 인 타입만 후보에 들어가고([agent/mod.rs:229-231](../../engine/src/agent/mod.rs)),
점수 내림차순 정렬 후 **상위 12개**로 자른다
([agent/mod.rs:233-234](../../engine/src/agent/mod.rs)).

**주의**: `w.contains(&lname)` 방향이 있으므로 타입 이름이 짧으면 오탐이 잦다.
`Employee` 라는 타입과 `EV` 라는 타입이 함께 있는 그래프에서 단어 `"reviewer"` 는
`lname="ev"` 를 포함하므로 `EV` 에 +10을 준다.

### 2.3 응답 형태

```json
{
  "graph": "meeting",
  "question": "라일락 회의실을 어제 예약했던 사람 목록",
  "matched_types": [
    { "name": "MeetingRoom", "kind": "entity", "relevance": 13,
      "extends": [], "properties": [...], "roles": null },
    { "name": "RESERVED_BY", "kind": "relation", "relevance": 10,
      "extends": [], "properties": [], "roles": [...] }
  ],
  "fallback": null
}
```

매칭이 하나도 없으면 `fallback` 에 안내가 들어간다
([agent/mod.rs:254-256](../../engine/src/agent/mod.rs)):
`"no lexical match; call og_schema(graph) for the full schema"`.

### 2.4 og_schema_for 에는 토큰 예산이 없다

시그니처가 `(graph, question)` 뿐이다([agent/mod.rs:190](../../engine/src/agent/mod.rs)).
상한은 하드코딩된 12개 타입이고, 각 타입의 프로퍼티/role은 전량 포함된다. 프로퍼티가
많은 타입 12개는 여전히 예산을 넘길 수 있다.

### 2.5 성능 특성 — N+1

`og_schema_for` 는 **그래프의 모든 타입에 대해** `property_list(tid)` 를 호출한다
([agent/mod.rs:213, 223](../../engine/src/agent/mod.rs)). `property_list` 는 SPI 질의 1건이므로
([agent/mod.rs:130-152](../../engine/src/agent/mod.rs)), 타입이 N개면 점수 계산만으로 SPI
질의가 N건 발생한다. 상위 12개를 확정한 뒤 다시 `parent_names` + `property_list` +
`role_list` 를 호출하므로([agent/mod.rs:243-245](../../engine/src/agent/mod.rs)) 추가 36건이
더 붙는다.

`og_schema` 도 마찬가지로 반환되는 타입마다 SPI 질의 2~3건을 발생시킨다
([agent/mod.rs:66-74](../../engine/src/agent/mod.rs)). 여기에 1.1절의 상관 서브쿼리 카운트가
타입당 2회 붙는다.

---

## 3. 결정(Decision)

| ID | 결정 | 근거 |
|---|---|---|
| D-1 | 중요도 기준은 **인스턴스 수** — 사용 빈도나 중심성이 아니다 | [agent/mod.rs:38-42](../../engine/src/agent/mod.rs) |
| D-2 | 절단은 **타입 단위**로만 일어난다. 프로퍼티/role 목록은 자르지 않는다 | [agent/mod.rs:65-86](../../engine/src/agent/mod.rs) |
| D-3 | 토큰 환산은 타입당 30으로 고정 — 의도적 과소추정 | [agent/mod.rs:60-62](../../engine/src/agent/mod.rs) 주석 |
| D-4 | 최소 8개 타입은 예산과 무관하게 항상 반환 | [agent/mod.rs:62](../../engine/src/agent/mod.rs) `.max(8)` |
| D-5 | 관련성 판정은 **어휘 중첩만**. 이 DB가 벡터 검색을 갖고 있음에도 사용하지 않는다 | [agent/mod.rs:187-188, 216-228](../../engine/src/agent/mod.rs) |

---

## 4. 필수(Required) / 금지(Forbidden)

**필수**

- `schema_version` 을 캐시 무효화 키로 쓸 것. 이 값은 `og_catalog.schema_version` 의
  `max(version)` 이며([agent/mod.rs:24-27](../../engine/src/agent/mod.rs)),
  `bump_schema_version` 이 타입/프로퍼티/role/rule 변경 시 올린다
  (예: [engine/src/catalog/types.rs:599, 653, 681](../../engine/src/catalog/types.rs)).
- 예산을 지정할 때는 `truncated` 유무를 **반드시 확인하고 프롬프트에 전달**할 것.
  잘렸다는 사실을 모르는 모델은 없는 레이블을 지어낸다.
- 큰 온톨로지에서는 `og_schema_for` 를 먼저 시도하고, `fallback` 이 non-null이면
  `og_schema(graph, budget)` 으로 내려갈 것.

**금지**

- `token_budget` 을 출력 크기의 상한으로 간주하지 말 것 (1.3절).
- `og_schema_for` 의 `relevance` 점수를 의미적 유사도로 해석하지 말 것. 편집거리와
  부분문자열 매칭의 가중합이다.
- 타입 수가 수천 개인 그래프에서 `og_schema(graph, NULL)` 을 에이전트 경로에서
  호출하지 말 것. 절단 없이 전량 반환하며, 타입당 SPI 질의가 선형으로 발생한다 (2.5절).
- attribute 타입(`kind='a'`)이 있는 TypeQL 그래프에서 `token_budget` 계산을 신뢰하지
  말 것 (1.4절).

---

## 5. 참고

- 원문: [docs/agents.md:26-67](../../docs/agents.md) "Give it the schema, not the DDL"
- 함수 계약: [docs/api.md:173-174](../../docs/api.md)
- 스펙: FR-001~FR-006 ([specs/008-agent-native-interface/spec.md:208-220](../../specs/008-agent-native-interface/spec.md)),
  SC-001/SC-003 ([같은 파일:300-304](../../specs/008-agent-native-interface/spec.md))
- 개선 제안: [10_improvements_llm.md](10_improvements_llm.md) LLM-02, LLM-09

<!-- affects: llm, api, backend -->
<!-- requires-update: 02_api/00_index.md -->
