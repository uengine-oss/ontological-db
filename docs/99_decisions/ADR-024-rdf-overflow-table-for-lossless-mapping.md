# ADR-024: RDF 매핑 불가 구문을 버리지 않고 `og_triple_overflow`에 원형 보존한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/006-semantic-web-adapters/plan.md` 기준일) |
| 영향 범위 | adapters, data, catalog |
| 근거 | `specs/006-semantic-web-adapters/plan.md` "매핑 규칙"·Complexity Tracking, `engine/src/adapters/mod.rs:5`, `engine/src/adapters/rdf.rs:5-6`·`:527`·`:559`·`:581`·`:598`·`:821-838`, `engine/sql/bootstrap.sql:350-358`, `.specify/memory/constitution.md` 원칙 VI |

> **이 문서가 답하는 질문**
> - reification이나 복잡한 blank node를 담은 RDF를 적재하면 어떻게 되는가?
> - 그것을 담는 테이블이 두 번째 저장소 아닌가?

## 배경

헌법 원칙 VI는 **트리플 스토어를 따로 두는 이중 저장 구조를 금지**한다. RDF/OWL은
어댑터 계층에서 spec 002의 타입 시스템으로 **매핑**되어야 한다.

문제는 RDF의 일부 구문 — reification, 복잡한 blank node — 이 속성 그래프로 **무손실 매핑이
불가능**하다는 것이다. spec 006 FR-010은 그것을 조용히 버리는 것을 금지한다.

## 고려한 선택지

1. **매핑 불가 구문을 버림** — spec FR-010 위반. round-trip이 조용히 깨진다.
2. **별도 트리플 스토어 병행 운영** — 원칙 VI 위반. 질의 경로가 둘로 갈라진다.
3. **원형을 보조 테이블에 보존하고 보고**

## 결정

**3안.** 매핑 불가 구문은 `og_data.og_triple_overflow`에 원형 그대로 보존하고 보고한다
(`specs/006-.../plan.md`): *"매핑 불가 구문은 **버리지 않고** `og_data.og_triple_overflow`
에 원형 보존하고 보고한다(FR-010). 이것이 round-trip 손실률을 측정 가능하게 만든다."*

## 근거

- Complexity Tracking이 이 테이블의 성격을 명확히 규정한다:
  > `og_triple_overflow` 보조 테이블 | reification·복잡 blank node는 속성 그래프로 무손실
  > 매핑 불가. 조용히 버리는 것이 스펙 FR-010 위반이므로 원형 보존이 필요 | 별도 트리플
  > 스토어는 원칙 VI 위반. 이 테이블은 **대체 질의 경로가 아니라 손실 방지 기록**이다
- 이 구분이 결정의 핵심이다. **질의 경로가 아니므로 이중 저장이 아니다.** 어댑터는 여전히
  하나의 저장 엔진과 하나의 타입 시스템 위에서 동작한다.
- 구현이 이를 그대로 따른다 (`engine/src/adapters/mod.rs:5`):
  *"that will not map is preserved verbatim in `og_triple_overflow` and reported"*.
  `engine/src/adapters/rdf.rs:5-6`: *"Anything else is preserved in the overflow table
  and counted in the report rather than quietly discarded."*
- **무엇이 overflow로 가는지가 사유와 함께 기록된다** (`record_overflow`,
  `engine/src/adapters/rdf.rs:603-605`). 현재 기록되는 사유는 세 가지다:
  `"non-IRI subject or predicate"` (`:527`), `"object IRI is not an instance in this
  document"` (`:559`), `"blank node object"` (`:581`).
- 적재 결과가 조용하지 않다 — 보고 메시지가 사용자를 보고 함수로 안내한다 (`:598`):
  *"unmapped triples were preserved verbatim — see og_mapping_report(graph)"*.
  보고 함수는 `og_mapping_report(graph)`이며 사유별 트리플과 총계를 돌려준다
  (`engine/src/adapters/mod.rs:52-81`).
- 나머지 RDF/OWL 구문은 전부 타입 시스템으로 매핑된다
  (`specs/006-.../plan.md` "매핑 규칙" 표): `rdfs:subClassOf` → `type_parent` + 구간 라벨
  (ADR-007), `owl:ObjectProperty` → relation type + role, `owl:DatatypeProperty` →
  attribute(타입 테이블 컬럼, ADR-005) 등.
- 같은 spec의 SPARQL 결정도 원칙 VI를 따른다: 자체 AST를 만들지 않고
  `cypher::ast::Query`로 lower 해 003의 컴파일러·연산자를 재사용한다.
  (**단, SPARQL은 현재 미구현이다** — README 상태표: `006 | partial — SPARQL not yet`.)

## 결과

**긍정적**
- **RDF → 저장 → RDF 왕복이 실제로 무손실이다.** `og_dump_rdf`가 overflow 테이블을 다시
  읽어 원형 트리플을 함께 내보낸다 (`engine/src/adapters/rdf.rs:821`, `:838`).
- round-trip 손실률이 **측정 가능한 수치**가 된다. 손실이 0이라고 주장하는 대신 얼마인지,
  그리고 왜 그런지(사유별로) 말할 수 있다 — `og_mapping_report(graph)`.
- 원칙 VI가 지켜진다 — 질의 경로는 하나뿐이다.

**부정적 / 감수한 대가**
- overflow에 들어간 트리플은 **질의할 수 없다.** 보존되고 덤프에서 복원되지만, 그래프의
  일부로 참여하지 않는다 — Cypher/TypeQL로 볼 수 없다.
- 사용자가 "적재는 성공했는데 질의에 안 나온다"는 상황을 겪을 수 있다.
  `og_mapping_report(graph)`가 이를 완화하는 유일한 수단이다.
- 테이블 하나가 늘어난다 (`engine/sql/bootstrap.sql:350-358`). `pg_extension_config_dump`에
  등록되어 있어 `pg_dump`에 함께 실린다 (`:434`).

## 재검토 조건

- overflow 비율이 실제 온톨로지에서 유의미하게 높게 나오면, 해당 구문(특히 reification)을
  타입 시스템으로 매핑하는 방법을 다시 찾아야 한다. **TypeQL 쪽에서 관계를 노드로 물화하는
  결정(ADR-016)이 reification과 구조적으로 같은 문제를 이미 풀고 있으므로**, 그 매핑을
  RDF에 재사용할 수 있는지가 1순위 검토 대상이다.
- SPARQL이 구현되면 overflow 트리플에 대한 질의 요구가 생길 것이다. 그 시점에 "질의 경로가
  아니다"라는 현재의 규정이 유지 가능한지 재평가한다.

<!-- affects: adapters, data, catalog -->
<!-- requires-update: docs/99_decisions/ADR-016-typeql-relations-reified-as-nodes.md -->
