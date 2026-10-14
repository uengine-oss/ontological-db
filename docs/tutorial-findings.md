# 튜토리얼을 실행하며 발견한 것

> **이 문서가 답하는 질문**
> - 일곱 편의 튜토리얼을 실제로 실행하는 동안, 제품이 문서나 스펙과 다르게 동작한 곳은 어디인가?
> - 그중 무엇을 고쳤고, 무엇이 남았는가?

튜토리얼의 예제는 손으로 적은 출력이 아니라 실행 결과입니다. 그 과정에서 드러난 차이를 기능별로 모았습니다.
각 항목의 재현 예제는 해당 튜토리얼에 있습니다. 2026-10-13 기준입니다.

## 고친 것

| 무엇 | 증상 | 조치 |
|---|---|---|
| `=` 가 `IS NOT DISTINCT FROM` 으로 컴파일 | 원천·로컬 모두 B-tree를 못 씀. `null = 'x'` 가 Cypher와 다르게 `false` | 한쪽이 상수·파라미터이면 `=` / `<>` |
| `x IN [리터럴…]` 이 jsonb 포함으로 컴파일 | 인덱스를 못 쓰고 원천으로 내려가지 않음 | SQL `IN (…)` |
| `OPTIONAL MATCH` 로 매칭되지 않은 노드가 `{}` | `count(n)` 이 0이 아니라 1. 안티 조인이 항상 빈 결과 | 별칭의 id가 NULL이면 NULL |
| `og_cypher_explain` 의 `plan` 이 항상 `null`, `og_estimate` 의 추정치가 항상 0 | `EXPLAIN (FORMAT JSON)` 의 `json` 을 `jsonb` 로 읽고 실패를 삼킴 | `json` 으로 읽음 |
| 가상 노드에 `SET`·`DELETE` 가 원천까지 내려감 | 원천 계정이 읽기 전용이라 막혔을 뿐 | 엔진이 먼저 거부. 데이터소스 테이블에는 쓰기 거부 트리거 |
| Studio가 DB 연결이 끊기면 종료 | 연결 풀의 미처리 오류 | 오류 처리기 |
| Studio 결과의 컬럼 순서가 키 정렬 순서 | `RETURN` 순서가 아님 | `og_cypher_columns` 사용 |
| Studio에 파라미터 입력이 없음 | 비밀이 질의문과 브라우저 기록에 남음 | `:param 이름 => 값` (메모리에만) |
| Bolt 게이트웨이 바이너리가 소스보다 오래됨 | `tx_metadata` 의 시점·행위자를 조용히 무시 | 다시 빌드 |
| 소개 페이지에 `doctype` 과 문자 인코딩 선언이 없음 | 서버가 인코딩을 알리지 않으면 한글이 깨짐 | 선언 추가 |

## 남은 것

### Cypher 엔진
- 첫 절이 `OPTIONAL MATCH` 이고 첫 노드에 조건이 있으면 잘못된 SQL이 만들어진다 (`… FROM v_3 n1 AND (…)`).
- `OPTIONAL MATCH (v)-[:R]->(a:Label)` 에서 라벨 조건이 두 번째 LEFT JOIN이 되어, 다른 간선이 있는 행마다 null 행이 더 나온다.
- `RETURN x AS po ORDER BY po` 처럼 별칭이 변수 이름과 같으면 별칭이 아니라 변수로 정렬한다.
- 새로 만든 빈 그래프에서 `MATCH (n) RETURN count(n)` 이 같은 DB의 다른 그래프 노드까지 센다.
- `toInteger('abc')` 가 null이 아니라 오류. `date()` 를 `SET` 에 쓰면 오류 없이 null이 들어간다. `round(x, 3)` 이 자릿수를 무시한다.
- `labels(v)` 의 순서와 범위가 문장 형태에 따라 다르다.
- `CREATE … WITH t MATCH … CREATE (t)-[:R]->(a)` 가 "CREATE requires a label per new node" 로 실패한다.

### 세 언어 (OWL · TypeQL)
- 숫자형 RDF 리터럴(`"5000"^^xsd:integer`)이 `text` 프로퍼티가 되어 비교가 PostgreSQL 오류로 실패한다.
- RDF 덤프는 무손실 왕복이 아니다. 프로퍼티 IRI가 `urn:og:prop:<이름>` 으로 바뀌고 자료형·언어 태그가 빠지며, TypeQL 관계 인스턴스가 든 그래프의 덤프는 다시 적재되지 않는다.
- `og_data.og_iri` 가 그래프를 넘어 IRI로 유일해서, 같은 IRI를 두 번째 그래프에 적재하면 조용히 건너뛴다.
- Turtle의 `[ … ]` 는 오버플로로 가지 않고 적재 전체를 중단시킨다.
- TypeQL `define` 묶음에 `sub` 가 하나도 없으면 새 타입의 구간 라벨이 비어, 뒤따르는 `plays`·`owns` 가 실패한다.
- TypeQL의 역할 매칭은 RDF·Cypher가 만든 간선에 대해 오류 없이 0행을 돌려준다.
- TypeQL `redefine` 이 `define` 으로 처리되어 스키마 변경 절차와 무관하게 별도의 속성 타입을 만든다.

### 이중 시간
- 미래 날짜의 변경이 **라벨 없는 패턴**에서는 시점을 정하지 않아도 보인다 (`(n {name:'bob'})`, `(c)<-[:WORKS_AT]-(n)`).
- Studio의 "as known at" 이 한 건 앞선 쓰기를 가리킨다. 감사 시각은 밀리초, 시스템 시간은 마이크로초이기 때문이다.

### 보안
- 행 수준 보안은 패턴이 보호된 타입에 **라벨을 붙였을 때만** 경로를 지운다. 라벨 없는 가운데 노드로는 경로가 남고 `labels(p)` 가 타입을 알려 준다.
- 거부된 문장과 원시 SQL은 `og_audit` 에 남지 않는다.
- TypeQL 행위에 `params` 를 주면 검증만 하고 본문에 전달하지 않는다.

### 에이전트 인터페이스
- 없는 라벨은 오류가 아니다. `og_explain_error` 가 `{"ok": true}` 를 돌려주고 후보는 NOTICE로만 나가며, Studio는 그 NOTICE를 버린다.
- 카티션 곱 검사는 관계가 하나도 없는 패턴만 잡는다.
- 역할의 `read_only: true` 는 감사 기록 때문에 읽기까지 막고, `max_rows` 는 적용되지 않는다.
- 스키마 예산은 토큰이 아니라 타입 개수(`max(예산/30, 8)`)로 줄인다.
- 옳은 쓰기 질의를 `og_explain_error` 가 `COMPILE_ERROR` 로 보고한다.
- Studio의 "did you mean" 칩이 `:sql` 접두어 없이 다시 실행되어 실패한다.

### 벡터 검색
- `vector.similarity(a.emb, [1,0,0])` 처럼 리스트 리터럴을 주면 실패한다 (파라미터나 문자열은 된다).
- `og_add_embedding` 이 멱등이 아니다. `og_hybrid_search` 는 관계 타입에 대해 오류 없이 0행이다.
- `l2` 에서는 "높을수록 좋은 점수"가 아니라 거리 그대로 나온다. `ORDER BY vector.similarity(…) DESC LIMIT k` 는 HNSW 인덱스를 쓰지 않는다.
- `og_vector_search` 의 `filter` 인자가 SQL 조각으로 그대로 붙는다. 사용자 입력으로 만들면 주입 위험이 있다.
- 유사도가 null인 행이 내림차순 정렬에서 맨 앞에 온다.

### 스키마 진화
- 유니크 제약이 계층 전체가 아니라 테이블 단위다. 하위 타입에 같은 값이 들어간다.
- Studio 디자이너의 "rows rewritten" 이 변경 중에 새로 들어온 행까지 센다. 디자이너가 만든 상위 타입은 추상이 아니다.

### 가상 패브릭
- 문자열 키는 36비트로 해시된다. `id(n)` 조건은 원천으로 내려가지 않는다.
- 가상 관계는 가변 길이로 걸을 수 없고, 라벨 없는 스캔에 가상 노드가 나오지 않는다.
- Studio의 스키마 패널이 가상 노드를 인스턴스 0개로만 보여 준다.
