# Tasks: TypeQL 질의 표면

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
체크되지 않은 항목은 착수하지 않았거나 부분 구현이며, 그 이유는 plan.md 의
Phasing / Complexity Tracking 에 있다.

검증 명령: `python3 tests/typeql/run.py` (TypeDB 공식 bookstore 예제로 28개 검사)

## Phase 0 — 언어 표면
- [x] T001 렉서: 주석, 문자열/수/불리언/날짜시간 리터럴, 변수(`$x`), 애너테이션(`@k(..)`), **하이픈 포함 라벨**
- [x] T002 AST: 파이프라인 = 스테이지 목록
- [x] T003 파서: `define` 블록
- [x] T004 파서: `match` 패턴(`isa`/`isa!`/`has`/역할 패턴/`links`/비교/`or`/`not`/`let`/`sub`)
- [x] T005 파서: `insert` / `put` / `delete` / `update`
- [x] T006 파서: 성형 스테이지 `select`/`sort`/`limit`/`offset`/`distinct`/`reduce`
- [x] T007 파서: `fetch` 문서(중첩 객체, 서브페치 목록, 표현식)
- [x] T008 파서: `fun` 선언 보존
- [x] T009 스크립트 분할: 블록 구분자(`end;`)와 다중 질의

## Phase 1 — define
- [x] T010 `og_catalog.role.parent_role_id` 추가 (역할 특수화)
- [x] T011 `og_catalog.typeql_function` 카탈로그 테이블
- [x] T012 attribute 타입 → 값 테이블 `og_data.a_<tid> (id, val UNIQUE)`
- [x] T013 entity/relation 타입 생성 + `sub` + 구간 라벨 재계산
- [x] T014 `relates` / `relates .. as ..` → `og_catalog.role`
- [x] T015 `owns` / `plays` → `og_catalog.og_constraint`
- [x] T016 애너테이션 보존: `@abstract`/`@key`/`@unique`/`@card`/`@values`/`@range`
- [x] T017 멱등 `define` (재실행 무해)
- [x] T018 그래프당 내부 관계 타입 `$has` 부트스트랩

## Phase 2 — insert / put
- [x] T019 속성 인스턴스: 값 기준 조회-후-생성(중복 제거)
- [x] T020 엔티티 인스턴스 생성 + `has` 소유 간선
- [x] T021 관계 인스턴스(물화 노드) + `og_role_player` 역할 배정
- [x] T022 3항 이상 관계, 관계의 속성 소유
- [x] T023 `match ... insert` — 바인딩마다 실행
- [x] T024 `put` 의미론(있으면 재사용)
- [x] T025 제약 강제: `@key`/`@unique`/`@card` 상한/`@values`/`@range`/추상 타입 인스턴스화 거부
  - `@card` **하한**(커밋 시점 검사)은 미구현. 상한·키·유일성·허용값·범위는 적재 시점에 강제된다.

## Phase 3 — match → SQL
- [x] T026 `isa` 다형성 — `og_subtypes()` 기반, 재귀 순회 없음
- [x] T027 `isa!` 정확 타입
- [x] T028 `has` — 값/변수/비교 술어, 속성 타입 상속 포함
- [x] T029 역할 명시 관계 패턴
- [x] T030 역할 생략 관계 패턴(가능한 배정 전개, 플레이어 타입으로 가지치기)
- [x] T031 역할 특수화를 통한 상위 역할 매칭
- [x] T032 비교/문자열 술어(`==`,`!=`,`<`,`<=`,`>`,`>=`,`like`,`contains`)
- [x] T033 `{...} or {...}` / `not {...}` — 바깥 변수에 상관된 EXISTS 로 컴파일
  - 분기가 **새 변수를 바깥으로 내보내는** 형태는 미지원(상관 부분질의로 표현되지 않음).
- [x] T034 `let $x = <표현식>`
- [x] T035 타입 변수(`$t sub X`, `$t label n`)
- [x] T036 단일 SQL 컴파일 + 컴파일 결과 열람(`og_typeql_sql`)
- [x] **T036a 독립 패턴 그룹 분리** — 변수를 공유하지 않는 패턴은 각각의 부분질의로 컴파일한다.
  실측: 16개 독립 변수를 가진 블록이 >14분 → 0.6초(전체 파일).

## Phase 4 — 파이프라인 · fetch
- [x] T037 `select`/`distinct`/`limit`/`offset`
- [x] T038 `sort` (asc/desc, 다중 키)
- [x] T039 `reduce` + `groupby` + 집계(count/sum/max/min/mean/median/std/list)
- [x] T040 `fetch` 스칼라 투영 `$x.attr`
- [x] T041 `fetch` 중첩 객체
- [x] T042 `fetch` 서브페치 목록 — 결과 행을 제거하지 않음(상관 집계로 컴파일)
- [x] T043 표현식(산술, `round`/`abs`/`floor`/`ceil`/`length`)
- [x] T044 스테이지 순서 의미론 — 각 스테이지가 이전 질의를 감싼다

## Phase 5 — 수정 · 통합
- [x] T045 `match ... delete` (인스턴스/소유/역할 배정)
- [x] T046 `match ... update`
- [x] T047 스키마 덤프 → TypeQL (`og_typeql_schema`, 왕복 검증됨)
- [x] T048 감사 로그 `lang='typeql'`
- [x] T049 교정 가능한 오류(위치 + 원인 + 힌트)
- [x] T050 Cypher ↔ TypeQL 상호 가시성 (`og_typeql_attribute`, `og_typeql_role` 뷰)

## Phase 6 — 함수
- [x] T051 `fun` 저장/덤프
- [ ] T052 비재귀 단일값 함수 평가
- [ ] T053 비재귀 스트림 함수 평가
- [x] T054 함수 호출 — 명시적 미지원 오류(조용한 오답 없음)

## Phase 7 — 검증 (TypeDB 공식 예제)
- [x] T055 bookstore `schema.tql` 무수정 적재 (entity 19 / relation 12 / attribute 25, 특수화 역할 3, 함수 7)
- [x] T056 bookstore `data.tql` 무수정 적재 (31 블록, 0.6초)
- [x] T057 문서 질의 1(장르 + 서브페치 저자 + 가격) 결과 대조 — 일치
- [x] T058 문서 질의 2(프로모션 기간 + 할인가 계산) 결과 대조 — 일치
- [x] T059 다형성 검증(추상 `book` 21 = hardback 3 + paperback 12 + ebook 6)
- [x] T060 Cypher 교차 검증(같은 사실 집합)
- [x] T061 단일 명령 회귀 스위트 `python3 tests/typeql/run.py` + 미지원 항목 표기

## 남은 것 (정직하게)

- **사용자 정의 함수 평가(T052/T053)**. 선언은 보존·덤프되지만 호출은 실행되지 않고
  명시적 오류를 낸다. 예제 문서의 질의 4개 중 2개가 함수를 쓰므로, 문서 질의 적합성은
  **함수를 쓰지 않는 2개에 대해 100%**이며 나머지 2개는 미지원으로 표기한다.
- **`@card` 하한 검사**. 커밋 시점 검증 훅이 필요하다.
- **변수를 내보내는 분기(`or`)**. 현재 상관 EXISTS 로만 컴파일된다.
- **역할 배정에 인접 세그먼트 미사용**. `og_role_player` 인덱스 조인을 쓴다.
  이식성이 성립한 뒤의 성능 과제이며 plan.md 의 Complexity Tracking 에 기록되어 있다.
- **`undefine` / `redefine`**. 파싱은 되지만 `undefine` 은 실행 시 미지원 오류를 낸다.
