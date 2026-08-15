# Implementation Plan: TypeQL 질의 표면

**Branch**: `010-typeql-query-surface` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

TypeQL(TypeDB 3.x)을 두 번째 1급 질의 표면으로 추가한다. Cypher와 **같은 카탈로그, 같은
저장 엔진, 같은 트랜잭션**을 쓰되, 데이터 모델의 차이는 파서가 아니라 **매핑 계층**에서 흡수한다.

**핵심 설계 결정**

1. **TypeQL은 Cypher 위에 얹지 않는다.** 별도의 렉서·파서·컴파일러를 둔다. TypeQL을 Cypher로
   번역하는 경로는 관계의 1급성·역할·속성 공유 의미론에서 반드시 새기 때문이다. 공유하는 것은
   AST가 아니라 **카탈로그와 저장 구조**다.
2. **관계는 노드로 물화(reify)한다.** TypeDB 관계는 3항 이상일 수 있고, 속성을 소유하며,
   다른 관계의 역할을 수행한다. 간선(src,dst)으로는 이 셋 중 어느 것도 온전히 표현되지 않는다.
   역할 배정은 스펙 002가 바로 이 목적으로 만든 `og_data.og_role_player`에 저장한다.
3. **속성은 값으로 중복 제거된 인스턴스다.** TypeDB 의미론의 핵심이며, 이것이 있어야
   "두 책이 같은 장르를 공유한다"가 그래프 탐색으로 답해진다. 소유는 인접 세그먼트(`og_adj`)
   위의 간선이므로 확장이 순차 읽기가 된다.
4. **다형성은 구간 라벨링 인덱스로만 푼다.** `$b isa book`이 hardback/paperback/ebook을
   모두 답하는 것은 스펙 002의 `og_subtypes()` 한 번이지 재귀 순회가 아니다. 속성 타입 상속
   (`isbn-13 sub isbn`)도 같은 인덱스로 푼다.
5. **읽기는 하나의 SQL로 컴파일한다.** Cypher 경로와 같은 원칙 — 계획기가 조인 순서를 갖는다.
   쓰기는 바인딩마다 절차적으로 실행한다.

## Data model mapping

이것이 이 스펙의 실질이다. FR-043이 요구하는 문서화된 대응이다.

| TypeQL 개념 | 저장 |
|---|---|
| entity 타입 | `og_catalog.type` kind `e` + 노드 테이블 `og_data.n_<tid>` |
| relation 타입 | `og_catalog.type` kind `r` + **노드** 테이블 `og_data.n_<tid>` (물화) |
| attribute 타입 | `og_catalog.type` kind `a` + 값 테이블 `og_data.a_<tid> (id, val UNIQUE)` |
| `sub` | `og_catalog.type_parent` + 구간 라벨(`og_catalog.type_label`) |
| `relates r` | `og_catalog.role` (rel_type_id, name, ordinal=선언 순서) |
| `relates r as p` | `og_catalog.role.parent_role_id` |
| `plays R:r` | `og_catalog.og_constraint` kind `plays`, target `R:r` |
| `owns A @ann` | `og_catalog.og_constraint` kind `owns`, target `A`, params=애너테이션 |
| entity/relation 인스턴스 | `og_data.og_node` + 타입 테이블 1행 |
| attribute 인스턴스 | `og_data.og_node` + `og_data.a_<tid>` 1행 (값 UNIQUE → 자동 공유) |
| 소유 인스턴스 (`has`) | 그래프당 하나의 내부 관계 타입 `$has`의 간선. src=소유자, dst=속성 |
| 역할 배정 | `og_data.og_role_player (edge_id=관계노드, role_id, player_id)` |
| `fun` 선언 | `og_catalog.typeql_function` (신규 카탈로그 테이블) |

**속성 타입 필터가 공짜인 이유**: 식별자의 18비트에 type_id가 박혀 있으므로(스펙 001),
`has genre $g`의 장르 필터는 이웃 id에 대한 시프트-마스크다. 카탈로그 조인도, 별도 인덱스도
필요 없다. 상위 속성 타입(`has isbn $x`)은 `og_subtypes()`가 돌려준 집합에 대한 `= ANY`가 된다.

**값 타입 대응**: string→text, integer→int8, double→float8, boolean→bool, datetime→timestamp,
date→date, duration→interval, decimal→numeric.

**Cypher에서 본 모습**: TypeQL로 적재한 그래프를 Cypher로 보면 엔티티 노드, 물화된 관계 노드,
속성 노드, `$has` 간선이 보인다. 이는 숨겨진 사실이 아니라 **문서화된 투영**이다(FR-040/043).
`og_typeql_view()`가 이 대응을 설명하는 뷰를 제공한다.

## Architecture

```
engine/src/typeql/
├── mod.rs        # 진입점 og_typeql / og_typeql_check / og_typeql_sql / og_typeql_schema
├── lexer.rs      # 토큰화 (주석, 리터럴, 애너테이션, 변수, 라벨의 하이픈 허용)
├── ast.rs        # 질의 = 스테이지의 파이프라인
├── parser.rs     # define / insert / put / match / delete / update / 성형 스테이지 / fetch
├── schema.rs     # define 실행 → og_catalog (멱등)
├── compile.rs    # match 패턴 → 단일 SQL
├── write.rs      # insert / put / delete / update 실행
├── fetch.rs      # fetch 문서 구성 (중첩 서브페치 포함)
└── dump.rs       # 스키마 → TypeQL 역직렬화 (왕복 검증용)
```

**렉서에서 주의할 것**: TypeQL 라벨은 하이픈을 포함한다(`isbn-13`, `order-line`,
`start-timestamp`). `a-b`가 뺄셈이 아니라 하나의 라벨이므로, 식별자 규칙이 Cypher와 다르다.
표현식 문맥에서만 `-`를 연산자로 읽는다.

## Constitution Check

| 원칙 | 상태 |
|------|------|
| I (unpatched PostgreSQL) | ✅ 확장 안에서 전부 해결 |
| II (Cypher-native) | ⚠️ 두 번째 언어를 추가한다 — Complexity Tracking 참조 |
| III (타입 시스템 1급) | ✅ 이 스펙이 002의 카탈로그를 실제로 쓰는 첫 소비자 |
| VI (에이전트 친화 오류) | ✅ FR-042 — 008의 교정 가능한 오류 형식 재사용 |
| IX (호스트 MVCC 그대로) | ✅ TypeDB의 트랜잭션 종류 구분을 도입하지 않음 |
| X (측정되지 않으면 없는 것) | ✅ SC 전부가 외부 예제 애플리케이션으로 검증됨 |

## Complexity Tracking

| 편차 | 이유 | 대안을 버린 근거 |
|---|---|---|
| 두 번째 질의 언어 추가 (원칙 II) | 002가 TypeDB 개념 모델을 채택했으면서 그 언어를 거부한 것은 이식성 측면에서 절반의 결정이었다 | TypeQL을 Cypher로 번역하는 대안은 관계 1급성·역할·속성 공유에서 반드시 새며, 조용한 오답을 만든다 |
| 관계를 노드로 물화 | 3항 관계 + 관계의 속성 소유 + 관계의 역할 수행을 (src,dst) 간선으로 표현할 수 없다 | 간선 + `og_role_player` 혼합안은 "관계가 속성을 소유"를 표현하지 못한다 |
| 역할 배정에 인접 세그먼트를 쓰지 않음 | `og_role_player`가 스펙 002가 이 목적으로 만든 구조이며 PK/역인덱스가 이미 있다 | CSR 세그먼트로 옮기는 최적화는 이식성이 성립한 뒤의 과제로 남긴다 — **미구현임을 명시** |
| 신규 카탈로그 테이블 1개(`typeql_function`) | 함수 본문은 기존 어느 카탈로그 테이블에도 맞지 않는다 | 스펙의 "새 저장 구조 없음" 가정은 **데이터** 저장에 대한 것이며, 스키마 카탈로그 확장은 별개 |

## Phasing

| Phase | 내용 | 대응 |
|-------|------|------|
| P0 | 렉서 · 파서 · AST | FR-001..005 |
| P1 | `define` 실행 → 카탈로그 (멱등) | US1, FR-006..014 |
| P2 | `insert` / `put` / `match ... insert` | US2, FR-015..021 |
| P3 | `match` 컴파일 → SQL (isa/has/역할/술어/or/not/let) | US3, FR-022..031 |
| P4 | 성형 스테이지 + `fetch` | US3/US4, FR-032..036 |
| P5 | `delete` / `update`, 스키마 덤프, 감사·오류 통합 | US4/US5, FR-037..043 |
| P6 | 비재귀 함수 | US6, FR-044..046 |

## Verification

수용은 주장이 아니라 **외부 산출물**로 한다: TypeDB 공식 예제 저장소의 bookstore
(`schema.tql`, `data.tql`, README에 결과가 함께 실린 질의)를 그대로 가져와 적재·실행하고
문서의 결과와 대조한다. 회귀 스위트는 `tests/typeql/`에 두고 단일 명령으로 실행한다.
미지원 항목은 통과율에 포함하지 않고 **미지원으로 표기**한다(SC-010).
