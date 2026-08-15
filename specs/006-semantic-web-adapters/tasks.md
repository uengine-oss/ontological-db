# Tasks: 시맨틱 웹 어댑터

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — RDF ingest
- [x] T001 Turtle / N-Triples 파서 — `adapters/rdf.rs`
- [x] T002 접두어, IRI 해석, 언어 태그, `xsd:` 데이터타입 보존
- [x] T003 IRI 레지스트리 (`og_iri`, `og_catalog.prefix`)
- [x] T004 매핑 불가 트리플을 `og_triple_overflow`에 원형 보존 (FR-010)
- [x] T005 적재 보고서 (`og_load_rdf` 반환값, `og_mapping_report`)

## Phase 1 — ontology mapping
- [x] T006 `owl:Class`/`rdfs:Class` → entity 타입
- [x] T007 `rdfs:subClassOf` → 상속 + 구간 라벨 재계산
- [x] T008 `owl:ObjectProperty` → 관계 타입
- [x] T009 `rdfs:domain`/`range` → role player 제약
- [x] T010 `owl:TransitiveProperty`/`SymmetricProperty` → `og_catalog.rule`
- [x] T011 적재된 온톨로지를 Cypher로 질의 (상속 포함)

## Phase 2 — export
- [x] T012 `og_dump_rdf` — TBox + ABox + overflow 재발행
- [ ] T013 round-trip 손실률 자동 측정

## Phase 3 — SPARQL (not started)
- [ ] T014 SPARQL 1.1 SELECT/ASK 파서
- [ ] T015 SPARQL AST → `cypher::ast` lowering (동일 컴파일러 재사용)
- [ ] T016 SPARQL 프로토콜 엔드포인트
- [ ] T017 SPARQL 테스트 스위트 통과율

## Phase 4 — reasoning & validation (not started)
- [ ] T018 OWL 2 RL 부분집합 추론
- [ ] T019 SHACL Core 검증 + 스키마 제약 승격
- [ ] T020 GraphQL 스키마 생성
