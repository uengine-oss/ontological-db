# Implementation Plan: 시맨틱 웹 어댑터

**Branch**: `006-semantic-web-adapters` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

RDF/OWL을 **002의 타입 시스템으로 매핑**하고, SPARQL을 **003의 컴파일러로 컴파일**한다.
트리플 스토어를 따로 두지 않는다(헌법 VI).

**매핑 규칙**

| RDF/OWL | Ontological |
|---------|-------------|
| `rdfs:Class` / `owl:Class` | entity type |
| `rdfs:subClassOf` | `og_catalog.type_parent` → 구간 라벨 |
| `owl:ObjectProperty` | relation type (+ role `subject`/`object`) |
| `owl:DatatypeProperty` | attribute (타입 테이블 컬럼) |
| `rdfs:domain` / `rdfs:range` | role player 타입 제약 |
| `owl:FunctionalProperty` | `card_max = 1` |
| `owl:TransitiveProperty` 등 | `og_catalog.rule` |
| IRI | `type.iri` / `og_data.og_iri` |
| 언어 태그 / `xsd:` 타입 | `og_data.og_literal` 보조 컬럼 |

매핑 불가 구문은 **버리지 않고** `og_data.og_triple_overflow` 에 원형 보존하고 보고한다
(FR-010). 이것이 round-trip 손실률을 측정 가능하게 만든다.

## Constitution Check

| 원칙 | 상태 |
|------|------|
| **VI** | ✅ 어댑터는 코어 의미론을 바꾸지 않는다. 저장은 하나 |
| II | ✅ SPARQL은 Cypher와 동일한 컴파일러·연산자로 내려간다 |
| X | ✅ SPARQL 통과율이 009의 게이트 대상 |

## Architecture

```
adapters/
├── rdf/
│   ├── parse.rs      # Turtle / N-Triples / N-Quads
│   ├── serialize.rs  # 내보내기 (round-trip)
│   └── map.rs        # RDF → 타입 시스템 매핑 + 보고서
└── sparql/
    ├── parser.rs     # SPARQL 1.1 SELECT/ASK/CONSTRUCT 부분집합
    └── lower.rs      # SPARQL AST → cypher::ast (공용 논리 계획)
```

**핵심 결정**: SPARQL 파서는 자체 AST를 만들지 않고 **`cypher::ast::Query` 로 lower** 한다.
그러면 003의 컴파일러·옵티마이저·연산자를 그대로 재사용하며, SC-005("동일 물리 연산자")가
설계상 자동 충족된다.

## Phasing

| Phase | 내용 |
|-------|------|
| P0 | Turtle/N-Triples 적재 + IRI 레지스트리 + 매핑 보고서 |
| P1 | RDFS/OWL 클래스·프로퍼티 → 타입 계층 |
| P2 | 내보내기 + round-trip 손실 측정 |
| P3 | SPARQL SELECT/ASK → cypher AST lowering |
| P4 | OWL 2 RL 부분집합 추론, SHACL 검증 |
| P5 | GraphQL 스키마 생성 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| `og_triple_overflow` 보조 테이블 | reification·복잡 blank node는 속성 그래프로 무손실 매핑 불가. 조용히 버리는 것이 스펙 FR-010 위반이므로 원형 보존이 필요 | 별도 트리플 스토어는 원칙 VI 위반. 이 테이블은 **대체 질의 경로가 아니라 손실 방지 기록**이다 |
