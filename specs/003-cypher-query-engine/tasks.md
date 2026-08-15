# Tasks: 네이티브 Cypher 질의 엔진

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그렇게 표기한 이유는
plan.md 의 Phasing / Complexity Tracking 에 있다.

## Phase 0 — front end
- [x] T001 렉서: 유니코드 문자열, 주석, 파라미터, 백틱 식별자 — `cypher/lexer.rs`
- [x] T002 AST + 집계 판별 + 컬럼 이름 규칙 — `cypher/ast.rs`
- [x] T003 재귀 하강 파서 — `cypher/parser.rs`
- [x] T004 미지원 구문은 대안을 명시한 오류 (FR-008)
- [x] T005 파서 단위 테스트 (`cargo test`)

## Phase 1 — read compilation
- [x] T006 타입 → 구체 테이블 UNION 뷰 — `cypher/views.rs`
- [x] T007 라벨을 컴파일 타임에 해소 (실행 시 계층 비용 0)
- [x] T008 MATCH / WHERE / RETURN / ORDER BY / SKIP / LIMIT
- [x] T009 타입 힌트 기반 파라미터 캐스팅 (인덱스 유지 + 주입 방지)
- [x] T010 ORDER BY가 RETURN 별칭 참조 지원

## Phase 2 — patterns
- [x] T011 관계 패턴, 방향, 타입 대안
- [x] T012 가변 길이 경로 — `og_vlp`, trail semantics
- [x] T013 경로 변수
- [x] T014 OPTIONAL MATCH → LEFT JOIN LATERAL

## Phase 3 — projection
- [x] T015 집계 + 자동 GROUP BY 도출
- [x] T016 DISTINCT
- [x] T017 노드/관계 전체 투영 (jsonb)
- [x] T018 함수 라이브러리 (문자열/수학/변환/그래프/벡터)

## Phase 4 — writes
- [x] T019 CREATE (노드 + 관계)
- [x] T020 MERGE + ON CREATE/ON MATCH
- [x] T021 SET / REMOVE
- [x] T022 DELETE / DETACH DELETE (미분리 삭제는 거부)
- [x] T023 쓰기 절 표현식 평가기 — `cypher/eval.rs`

## Phase 5 — surface
- [x] T024 `og_cypher_sql` — 컴파일된 SQL 노출 (SQL 임베딩 경로)
- [x] T025 `og_cypher_explain` — SQL + PostgreSQL 계획
- [x] T026 컴파일 캐시 (파싱·컴파일 재사용)
- [x] T027 감사 로그 기록

## Phase 6 — not yet
- [ ] T028 WITH 체이닝 (현재 명시적 오류)
- [ ] T029 UNION
- [ ] T030 UNWIND (읽기 전용 경로만 부분 지원)
- [ ] T031 리소스 상한을 컴파일된 SQL에 주입
- [ ] T032 Custom Scan 전환 (v2)

## Phase 7 — Neo4j 샘플 적합성
공식 Movie Graph 샘플을 고쳐 쓰지 않고 그대로 돌린 결과. 데이터셋 39/39 구문,
가이드 질의 24/24 가 같은 데이터를 적재한 Neo4j 5와 동일한 행 수를 낸다 (SC-011).
아래 네 건은 회귀 스위트가 통과하는 동안 살아 있던 결함이며, 이 실행이 드러냈다.
- [x] T033 Movie Graph 샘플 하네스 — 데이터셋·가이드 질의·Neo4j 결과 대조 — `tests/neo4j-movies/`
- [x] T034 Bolt 미지원을 주장이 아니라 실제 핸드셰이크로 확인 (FR-024a)
- [x] T035 리스트 리터럴을 타입 있는 값으로 컴파일 — `['Neo']` 가 붙은 MERGE 는 전부 실패했다
- [x] T036 리스트 ↔ 배열 컬럼 비교를 `ARRAY[...]::text[]` 로 컴파일 (`text[] = jsonb` 오류)
- [x] T037 MERGE가 이미 바인딩된 끝점을 고정 — 그러지 않으면 그래프의 아무 간선이나 찾아
      두 번째 간선을 만들지 않는다 (DIRECTED 44개가 1개로 들어왔다)
- [x] T038 관계 유일성(isomorphism)을 MATCH 절 범위로 적용 (FR-006a)
