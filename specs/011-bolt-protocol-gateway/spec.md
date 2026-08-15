# Feature Specification: Bolt 프로토콜 게이트웨이 (Bolt Protocol Gateway)

**Feature Branch**: `011-bolt-protocol-gateway`

**Created**: 2026-08-06

**Status**: Draft

**Input**: User description: "neo4j 프로토콜을 지원하는지 테스트해봐" → "기존 neo4j
샘플애플리케이션을 하나 가져와서 동작하는지 확인해봐" → "neo4j bolt 도 추가적으로 지원을 하자."

## 개요

스펙 003은 FR-024로 "Cypher는 기존 PostgreSQL 드라이버로 실행 가능해야 한다"를 못박았고,
그 귀결로 FR-024a에서 **Bolt를 비목표로 선언**했다. 그 선언의 근거는 비용이었다:
두 번째 프로토콜 리스너는 두 번째 인증·권한·감사 경로를 뜻한다.

`tests/neo4j-movies/` 가 그 선언의 실제 값을 측정했다. 공식 Neo4j Movie Graph 샘플의
데이터셋 39개 구문과 가이드 질의 24개는 **한 줄도 고치지 않고** 동작하며 Neo4j와 같은
결과를 낸다. 넘어오지 않는 것은 Cypher가 아니라 **연결 계층 하나**뿐이었다.

이 스펙은 그 하나를 없앤다. **Bolt를 엣지의 어댑터로 추가**하여, Neo4j 애플리케이션이
드라이버·세션·결과 매핑 코드를 그대로 둔 채 URI만 바꿔 접속할 수 있게 한다.

FR-024a는 이 스펙으로 **대체**된다. 다만 그 조항이 지키려던 것 — 코어에 두 번째 진실의
원천을 만들지 않는다 — 은 그대로 유지된다. Bolt는 **코어 밖의 게이트웨이 프로세스**이며,
질의는 여전히 `og_cypher()` 한 경로로만 실행된다. 헌법 원칙 VI("코어는 하나, 표준은
어댑터로")의 프로토콜 판(版)이다.

이것은 PostgreSQL 프로토콜 경로를 대체하지 않는다. 두 경로는 같은 그래프, 같은 카탈로그,
같은 트랜잭션 의미론을 공유하며, 어느 쪽으로 들어와도 같은 컴파일러가 같은 SQL을 만든다.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Neo4j 애플리케이션이 URI만 바꿔 접속한다 (Priority: P1)

Neo4j로 작성된 애플리케이션을 가진 팀이 접속 문자열의 호스트·포트만 바꾼다. 드라이버 코드,
세션 관리, 결과 매핑, Cypher 문자열은 그대로다. 애플리케이션은 자신이 Neo4j가 아닌 것에
접속했다는 사실을 알 필요가 없다.

**Why this priority**: 이 스펙의 존재 이유다. 이것이 안 되면 나머지는 의미가 없다.

**Independent Test**: 공식 Movie Graph 샘플 앱(`code/python/example.py`)의 접속 URI만
바꿔 실행하고, 결과가 같은 데이터를 적재한 Neo4j와 일치하는지 확인한다.

**Acceptance Scenarios**:

1. **Given** 게이트웨이가 떠 있을 때, **When** Neo4j 공식 드라이버가 `bolt://host:7687`
   으로 접속하면, **Then** 핸드셰이크가 성공하고 드라이버의 `verify_connectivity()`가 통과한다.
2. **Given** 접속된 세션에서, **When** 파라미터를 포함한 Cypher를 실행하면, **Then**
   레코드가 스트리밍되고 필드 이름은 질의의 `RETURN` 순서를 따른다.
3. **Given** 노드를 반환하는 질의에서, **When** 드라이버가 결과를 받으면, **Then**
   `Node` 객체로 역직렬화되며 `.labels`·`.items()`·`.id`가 채워져 있다.
4. **Given** 잘못된 Cypher에서, **When** 실행하면, **Then** 드라이버는 `Neo4jError`를
   받고 메시지에 컴파일러의 교정 제안이 담긴다.
5. **Given** 실패한 세션에서, **When** 드라이버가 `RESET`을 보내면, **Then** 세션이
   정상 상태로 돌아와 다음 질의를 받는다.

---

### User Story 2 - 세션과 트랜잭션이 드라이버 규약대로 동작한다 (Priority: P1)

애플리케이션이 `session.begin_transaction()` / `read_transaction()` / `write_transaction()`
을 쓴다. 커밋과 롤백이 PostgreSQL 트랜잭션에 그대로 대응되어야 하며, 롤백된 쓰기는
PostgreSQL 프로토콜 경로에서도 보이지 않아야 한다.

**Why this priority**: 자동커밋만 되는 게이트웨이는 실제 애플리케이션을 받지 못한다.
헌법 원칙 IX(ACID)는 프로토콜이 둘이라고 완화되지 않는다.

**Independent Test**: Bolt로 명시적 트랜잭션을 열어 노드를 만들고 롤백한 뒤, psql로
그 노드가 없음을 확인한다.

**Acceptance Scenarios**:

1. **Given** 명시적 트랜잭션에서 노드를 만들고, **When** 커밋하면, **Then** psql에서 보인다.
2. **Given** 명시적 트랜잭션에서 노드를 만들고, **When** 롤백하면, **Then** psql에서 보이지 않는다.
3. **Given** 트랜잭션 중 질의가 실패하면, **When** 드라이버가 롤백하면, **Then** 같은
   트랜잭션의 앞선 쓰기도 함께 취소된다.
4. **Given** 자동커밋 질의는, **When** 성공하면, **Then** 별도 커밋 없이 즉시 반영된다.

---

### User Story 3 - 데이터베이스 선택이 그래프 선택이 된다 (Priority: P2)

Neo4j 드라이버는 세션마다 `database="..."` 를 지정한다. 이 값이 그래프 이름이 되어,
하나의 게이트웨이가 여러 그래프를 서비스한다.

**Why this priority**: 이 데이터베이스의 그래프 개념과 Neo4j의 데이터베이스 개념을 잇는
자연스러운 대응이며, 없으면 게이트웨이 인스턴스를 그래프마다 띄워야 한다.

**Independent Test**: 그래프 두 개를 만들고, `database=` 만 바꾼 두 세션이 각각의
그래프를 보는지 확인한다.

**Acceptance Scenarios**:

1. **Given** 그래프 `movies`가 있을 때, **When** `database="movies"` 세션에서 질의하면,
   **Then** 해당 그래프의 데이터가 반환된다.
2. **Given** `database`를 지정하지 않으면, **When** 질의하면, **Then** 게이트웨이의
   기본 그래프가 사용된다.
3. **Given** 없는 그래프를 지정하면, **When** 질의하면, **Then** 그래프 이름을 담은
   오류가 반환되며 연결은 유지된다.

---

### User Story 4 - 인증이 PostgreSQL 역할에 그대로 대응된다 (Priority: P2)

Bolt `HELLO`가 나르는 사용자/비밀번호가 PostgreSQL 역할의 그것이다. 게이트웨이는 자체
사용자 저장소를 갖지 않는다.

**Why this priority**: 두 번째 인증 경로를 만들지 않는다는 것이 FR-024a의 원래 우려였고,
이 대응이 그 우려에 대한 답이다. 권한·RLS·감사가 전부 PostgreSQL 것 하나로 유지된다.

**Independent Test**: 권한이 없는 역할로 Bolt 접속해 질의하면 PostgreSQL의 권한 오류가
드라이버까지 전달되는지 확인한다.

**Acceptance Scenarios**:

1. **Given** 유효한 PostgreSQL 역할과 비밀번호로, **When** Bolt 접속하면, **Then** 성공한다.
2. **Given** 잘못된 비밀번호로, **When** 접속하면, **Then** `Neo.ClientError.Security.Unauthorized`
   가 반환된다.
3. **Given** RLS가 걸린 타입에서, **When** 제한된 역할로 질의하면, **Then** PostgreSQL
   프로토콜 경로와 **같은 행 집합**이 반환된다.

---

### Edge Cases

- 드라이버가 지원 범위 밖의 Bolt 버전만 제안하면? → 핸드셰이크에서 `0x00000000`을 돌려주고
  연결을 닫는다. 드라이버는 명확한 버전 협상 실패로 인식한다.
- 미지원 Cypher(`WITH`, `UNION`, `shortestPath`)를 보내면? → 컴파일러 오류가 그대로
  `FAILURE`로 전달된다. 프로토콜이 의미를 바꾸지 않는다.
- 클라이언트가 라우팅 테이블(`ROUTE`)을 요청하면? → 단일 서버임을 알리는 응답을 돌려주어
  `neo4j://` 스킴도 동작하게 한다.
- 결과를 다 읽지 않고 `RESET`하면? → 남은 레코드를 버리고 세션을 정상 상태로 되돌린다.
- 청크 경계가 메시지 중간에 걸리면? → 재조립 후 처리한다. 청크 크기는 메시지 크기와 무관하다.

## Requirements *(mandatory)*

### Functional Requirements

**프로토콜**

- **FR-001**: 게이트웨이는 Bolt 핸드셰이크(매직 `0x6060B017` + 버전 제안 4개)를 처리하고
  지원 버전 중 가장 높은 것을 선택해야 한다.
- **FR-002**: 게이트웨이는 Bolt 4.4를 지원해야 한다. 그 외 버전은 협상 실패로 명확히 거절한다.
- **FR-003**: 게이트웨이는 청크 프레이밍(2바이트 길이 + 페이로드, `0x0000` 종료)을
  구현해야 하며, 메시지 크기와 청크 크기는 독립이어야 한다.
- **FR-004**: 게이트웨이는 PackStream v2로 값을 인코딩·디코딩해야 한다:
  null, bool, int(모든 폭), float, string, list, dictionary, structure.
- **FR-005**: 게이트웨이는 `HELLO`, `RUN`, `PULL`, `DISCARD`, `BEGIN`, `COMMIT`,
  `ROLLBACK`, `RESET`, `GOODBYE`, `ROUTE` 메시지를 처리해야 한다.
- **FR-006**: 게이트웨이는 `SUCCESS`, `RECORD`, `FAILURE`, `IGNORED` 를 규약대로 보내야 한다.
- **FR-007**: 실패한 세션은 `RESET` 전까지 들어오는 모든 메시지에 `IGNORED`로 응답해야 한다.

**질의 실행**

- **FR-008**: 게이트웨이는 Cypher를 자체 해석하지 않으며, `og_cypher()` 로만 실행해야 한다.
  프로토콜 계층은 의미론을 갖지 않는다.
- **FR-009**: 게이트웨이는 `RUN`의 파라미터 딕셔너리를 `og_cypher()`의 파라미터로 전달해야
  하며, 질의 문자열에 값을 보간해서는 안 된다(주입 방지, 003 FR-026과 동일 보장).
- **FR-010**: `SUCCESS`의 `fields`는 질의의 `RETURN` 절 순서를 따라야 한다. jsonb 키 정렬
  순서가 아니다.
- **FR-011**: 노드는 Bolt `Node` 구조체로, 관계는 `Relationship` 구조체로 인코딩되어야
  하며, 드라이버가 각각 `Node`/`Relationship` 객체로 역직렬화할 수 있어야 한다.
- **FR-012**: 게이트웨이는 `PULL n` 의 요청 개수를 존중해야 하며, `has_more` 를 통해
  후속 `PULL`을 지원해야 한다.

**세션**

- **FR-013**: Bolt 연결 하나는 PostgreSQL 연결 하나에 대응되어야 하며, 연결 수명 동안 유지된다.
- **FR-014**: `BEGIN`/`COMMIT`/`ROLLBACK`은 PostgreSQL 트랜잭션에 직접 대응되어야 한다.
- **FR-015**: `HELLO`의 인증 정보는 PostgreSQL 역할 인증에 그대로 사용되어야 하며,
  게이트웨이는 자체 사용자 저장소를 가져서는 안 된다.
- **FR-016**: `RUN`/`BEGIN`의 `db` 필드는 그래프 이름으로 해석되어야 한다. 없으면 기본 그래프.
- **FR-017**: 게이트웨이는 연결마다 독립적으로 동작해야 하며, 한 세션의 질의가 다른 세션을
  차단해서는 안 된다.

**운영**

- **FR-018**: 게이트웨이는 코어 확장과 **별도 프로세스**여야 하며, 켜지 않아도 PostgreSQL
  경로는 아무 영향을 받지 않아야 한다.
- **FR-019**: 오류는 Neo4j 오류 코드 체계(`Neo.ClientError.*`)로 매핑되어 전달되어야 하며,
  원본 PostgreSQL 메시지를 보존해야 한다.
- **FR-020**: 게이트웨이는 지원 범위(버전, 메시지, 타입)를 문서화된 매트릭스로 제공해야 한다 —
  "지원함"이라는 모호한 표현 금지(헌법 기술 제약).

### Key Entities

- **Bolt 연결**: TCP 연결 하나. 협상된 버전, 인증 주체, PostgreSQL 연결 하나를 갖는다.
- **세션 상태**: `연결됨 → 준비 → 스트리밍 → 실패` 상태 기계. `RESET`이 준비로 되돌린다.
- **PackStream 값**: 프로토콜의 값 표현. 그래프 값(`Node`/`Relationship`)은 구조체로 실린다.
- **게이트웨이 프로세스**: Bolt를 말하고 `og_cypher()`를 부르는 것 외에 아무 상태도 갖지 않는다.

## Success Criteria *(mandatory)*

- **SC-001**: Neo4j 공식 Python 드라이버로 접속·질의·결과 수신이 **드라이버 코드 수정 없이**
  가능하다.
- **SC-002**: `tests/neo4j-movies/` 의 가이드 질의 24개를 Bolt 경로로 실행한 결과가
  PostgreSQL 경로 및 Neo4j와 **모두 동일한 행 수**를 낸다.
- **SC-003**: 공식 Movie 샘플 앱(`code/python/example.py`)이 **URI 한 줄만 바뀐 채**
  실행되어 Neo4j와 같은 결과를 낸다.
- **SC-004**: 노드를 반환하는 질의의 결과가 드라이버에서 `Node` 객체로 역직렬화되며
  `labels`·프로퍼티가 Neo4j와 일치한다.
- **SC-005**: 명시적 트랜잭션의 롤백이 PostgreSQL 경로에서도 관측되지 않는다.
- **SC-006**: 게이트웨이를 끄면 PostgreSQL 경로의 회귀 스위트가 **변함없이 통과**한다.
- **SC-007**: 동시 세션 8개가 서로를 차단하지 않는다.

## Assumptions

- Cypher 의미론·컴파일·오류 메시지는 전부 스펙 **003**의 것을 그대로 쓴다. 이 스펙은
  프로토콜만 다룬다.
- 그래프·타입 선언은 스펙 **002**의 카탈로그를 쓴다. Bolt로 스키마를 정의하는 경로는
  Cypher가 지원하는 범위까지만이다.
- 권한·RLS·감사는 스펙 **005**의 PostgreSQL 메커니즘을 그대로 상속한다.
- 클러스터 라우팅(진짜 라우팅 테이블 반환)은 스펙 **007**의 범위이며, 여기서는 단일 서버
  응답만 돌려준다.

## Out of Scope

- Bolt 5.x 및 그 신규 타입(시공간·공간 타입, element_id 문자열 식별자).
- `CALL {}` 서브질의, APOC/GDS 프로시저 — 003이 지원하지 않는 것은 여기서도 지원하지 않는다.
- Neo4j Browser의 시스템 질의(`SHOW DATABASES`, `CALL dbms.*`) 전체 호환. 이 스펙은
  **드라이버 호환**을 목표로 하며, Browser 호환은 별도 판단이다.
- 클러스터 라우팅, 북마크 기반 인과 일관성(형식만 채우고 단일 서버로 동작).
- TLS 종단(운영에서는 앞단 프록시로 처리한다고 가정).
