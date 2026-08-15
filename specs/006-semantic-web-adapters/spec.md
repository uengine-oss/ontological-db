# Feature Specification: 시맨틱 웹 어댑터 (RDF / OWL / SPARQL / SHACL Adapters)

**Feature Branch**: `006-semantic-web-adapters`

**Created**: 2026-08-05

**Status**: Draft

**Input**: User description: "온톨로지와 관련된 스펙들도 교차 지원하면 좋겠다. RDF, OWL, SPARQL 같은 것들을 어댑터를 가지고 수행하면 될 것 같고, 기본적으로는 사이퍼 쿼리가 중심"

## 개요

시맨틱 웹 표준(RDF/OWL/SPARQL/SHACL)은 온톨로지 자산이 축적된 곳이다. 공개 온톨로지(FOAF,
schema.org, SKOS, 산업 표준 온톨로지)를 그대로 가져오고, 기존 SPARQL 도구가 붙을 수 있어야
이 제품이 온톨로지 DB로서 의미를 갖는다.

동시에 헌법 원칙 VI가 분명히 한다: **코어는 하나다.** 트리플 스토어를 따로 두지 않는다.
RDF는 스펙 002의 타입 시스템과 스펙 001의 저장 구조로 **매핑**되고, SPARQL은 스펙 003의
논리 계획으로 **컴파일**된다. 중심 언어는 언제나 Cypher다.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 기존 온톨로지를 가져와 Cypher로 질의한다 (Priority: P1)

지식 엔지니어가 산업 표준 OWL 온톨로지(TTL/RDF-XML)를 적재한다. OWL 클래스 계층이 타입
계층으로, 프로퍼티가 관계·attribute 타입으로 변환되어 Cypher로 바로 질의된다.

**Why this priority**: 온톨로지 자산 재사용이 이 스펙의 첫 번째 가치다.

**Independent Test**: 공개 온톨로지(예: schema.org, FOAF, SKOS)를 적재하고 클래스 계층이
타입 계층으로 정확히 반영되었는지, Cypher 상위 타입 질의가 하위 클래스 인스턴스를 잡는지
검증한다.

**Acceptance Scenarios**:

1. **Given** OWL/RDFS 온톨로지 파일이 주어지면, **When** 적재하면, **Then** `rdfs:subClassOf`
   계층이 타입 상속 계층(002)으로 변환되고 상속 인덱스가 구축된다.
2. **Given** 적재된 온톨로지에서, **When** Cypher로 상위 클래스를 질의하면, **Then** 하위
   클래스 인스턴스가 모두 반환된다.
3. **Given** `owl:ObjectProperty` 와 `owl:DatatypeProperty` 가 정의된 온톨로지에서,
   **When** 적재하면, **Then** 각각 관계 타입과 attribute 타입으로 매핑된다.
4. **Given** 매핑 불가능한 OWL 구성요소가 있을 때, **When** 적재하면, **Then** 무시되지 않고
   경고 목록으로 보고되며 원본 트리플이 보존된다.
5. **Given** 적재된 데이터에서, **When** RDF로 다시 내보내면, **Then** 원본과 의미적으로
   동등한 그래프가 생성된다(round-trip).

---

### User Story 2 - 기존 SPARQL 도구가 그대로 붙는다 (Priority: P1)

시맨틱 웹 팀이 기존 SPARQL 질의와 클라이언트를 수정 없이 사용한다. SPARQL 엔드포인트로
접속해 같은 데이터를 조회한다.

**Why this priority**: 어댑터의 존재 이유. 마이그레이션 비용 없이 도입할 수 있어야 한다.

**Independent Test**: SPARQL 1.1 테스트 스위트를 실행하고 통과율을 측정한다. 기존 SPARQL
클라이언트로 엔드포인트에 접속해 표준 결과 형식을 받는지 확인한다.

**Acceptance Scenarios**:

1. **Given** SPARQL SELECT 질의가 주어지면, **When** 실행하면, **Then** 표준 결과 형식
   (SPARQL Results JSON/XML)으로 반환된다.
2. **Given** 동일한 의미의 SPARQL 질의와 Cypher 질의가 있을 때, **When** 각각 실행하면,
   **Then** 동일한 결과 집합을 반환한다.
3. **Given** SPARQL 질의가 실행될 때, **When** 실행 계획을 확인하면, **Then** Cypher와 동일한
   논리 계획·물리 연산자로 컴파일되었음이 확인된다(별도 실행 엔진 없음).
4. **Given** ASK, CONSTRUCT, DESCRIBE 질의 형태에서, **When** 실행하면, **Then** 각 형태의
   표준 결과가 반환된다.
5. **Given** 미지원 SPARQL 기능이 사용되면, **When** 실행하면, **Then** 어느 기능이 미지원
   인지 명시한 오류가 반환된다.

---

### User Story 3 - 식별자와 데이터타입이 표준과 정합한다 (Priority: P1)

데이터 통합 담당자가 외부 시스템과 데이터를 주고받는다. IRI, 접두어(prefix), 리터럴
데이터타입, 언어 태그가 손실 없이 유지된다.

**Why this priority**: 여기가 어긋나면 상호운용 자체가 성립하지 않는다. 조용한 데이터 손실이
가장 위험한 실패 모드다.

**Independent Test**: 다양한 데이터타입·언어 태그·blank node를 포함한 RDF를 적재 후 내보내
바이트 수준이 아닌 의미 수준에서 동등한지 검증한다.

**Acceptance Scenarios**:

1. **Given** IRI로 식별되는 리소스가 적재되면, **When** Cypher로 조회하면, **Then** 원본
   IRI가 보존되고 접두어 축약 형태로도 조회 가능하다.
2. **Given** 언어 태그가 붙은 리터럴(`"색"@ko`, `"color"@en`)이 있을 때, **When** 적재 후
   조회하면, **Then** 언어 태그가 보존되고 언어별 필터가 가능하다.
3. **Given** `xsd:` 데이터타입이 명시된 리터럴이 있을 때, **When** 적재하면, **Then** 대응
   PostgreSQL 타입으로 매핑되고 원 데이터타입 IRI가 보존된다.
4. **Given** blank node가 포함된 RDF에서, **When** 적재 후 내보내면, **Then** blank node
   구조가 의미적으로 보존된다.
5. **Given** 명명된 그래프(named graph)가 포함된 데이터셋에서, **When** 적재하면, **Then**
   그래프 구분이 보존되고 SPARQL `GRAPH` 절로 질의 가능하다.

---

### User Story 4 - SHACL로 데이터 품질을 검증한다 (Priority: P2)

데이터 품질 담당자가 SHACL 형태(shape) 정의로 그래프를 검증하고 위반 보고서를 받는다.
가능한 제약은 스키마 제약(002)으로 승격해 애초에 위반을 막는다.

**Why this priority**: 온톨로지 운영에서 중요하지만, 적재와 질의가 먼저 동작해야 한다.

**Independent Test**: SHACL 테스트 스위트로 검증 결과를 대조하고, 스키마 제약으로 승격 가능한
shape의 비율을 측정한다.

**Acceptance Scenarios**:

1. **Given** SHACL shape 정의가 주어지면, **When** 그래프를 검증하면, **Then** 표준 형식의
   위반 보고서가 생성된다.
2. **Given** 스키마 제약으로 표현 가능한 shape(필수 여부, 카디널리티, 데이터타입, 값 범위)이
   있을 때, **When** 승격을 요청하면, **Then** 002의 제약으로 등록되어 쓰기 시점에 강제된다.
3. **Given** 승격 불가능한 shape가 있을 때, **When** 승격을 시도하면, **Then** 이유가 보고되고
   검증 시점 확인으로 남는다.

---

### User Story 5 - OWL 추론이 질의에 반영된다 (Priority: P2)

온톨로지 엔지니어가 `owl:TransitiveProperty`, `owl:inverseOf`, `owl:equivalentClass` 같은
공리를 선언하면, 명시되지 않은 사실도 질의 결과에 나타난다.

**Why this priority**: 온톨로지 DB의 핵심 가치이지만, 002의 관계 특성 추론 위에서 확장되는
기능이다.

**Independent Test**: OWL 2 RL 프로파일의 규칙별 테스트 케이스를 실행해 추론 정확성을
검증한다.

**Acceptance Scenarios**:

1. **Given** `owl:TransitiveProperty` 로 선언된 프로퍼티의 체인이 있을 때, **When** 간접
   관계를 질의하면, **Then** 결과가 반환되며 추론된 사실로 표시된다.
2. **Given** `owl:inverseOf` 가 선언되면, **When** 역방향으로 질의하면, **Then** 명시 저장
   없이도 결과가 반환된다.
3. **Given** `owl:equivalentClass` 가 선언되면, **When** 어느 클래스로 질의해도, **Then**
   동일한 인스턴스 집합이 반환된다.
4. **Given** 지원 프로파일(OWL 2 RL) 밖의 공리가 있을 때, **When** 적재하면, **Then**
   미지원 공리가 명시적으로 보고된다.
5. **Given** 추론이 활성화된 질의에서, **When** 성능을 측정하면, **Then** 추론 비용이 문서화된
   범위 내이며 추론을 끌 수 있다.

---

### User Story 6 - GraphQL로 그래프를 노출한다 (Priority: P3)

프론트엔드 개발자가 타입 카탈로그에서 자동 생성된 GraphQL 스키마로 그래프를 조회한다.

**Why this priority**: 애플리케이션 개발 편의를 크게 높이지만, 핵심 온톨로지 기능은 아니다.

**Independent Test**: 타입 카탈로그로부터 GraphQL 스키마를 생성하고, 표준 GraphQL 클라이언트로
중첩 질의를 실행한다.

**Acceptance Scenarios**:

1. **Given** 타입 카탈로그가 정의되어 있을 때, **When** GraphQL 스키마를 생성하면, **Then**
   타입 계층과 관계가 GraphQL 타입·필드로 반영된다.
2. **Given** 중첩된 GraphQL 질의가 주어지면, **When** 실행하면, **Then** N+1 질의가 아닌 단일
   그래프 질의로 처리된다.
3. **Given** 스키마가 변경되면, **When** GraphQL 스키마를 재생성하면, **Then** 변경이 반영된다.

---

### Edge Cases

- **매핑 손실**: RDF의 모든 구성요소가 속성 그래프로 무손실 매핑되지는 않는다(reification,
  복잡한 blank node 구조). 무엇이 손실되는지 명시적으로 문서화·보고되어야 한다.
- **IRI 충돌**: 서로 다른 접두어가 같은 로컬명을 가질 때 타입 이름 충돌 처리.
- **매우 큰 온톨로지**: 수십만 클래스를 가진 온톨로지 적재 시 상속 인덱스 구축 비용.
- **추론 폭발**: 이행적 프로퍼티가 조밀한 그래프에서 추론 결과가 폭증할 때의 상한.
- **순환 공리**: `equivalentClass` 순환 등이 추론을 무한 루프에 빠뜨리지 않아야 한다.
- **동시 적재**: 대용량 RDF 적재 중 다른 질의가 일관된 스냅샷을 봐야 한다.
- **SPARQL UPDATE**: 읽기만 지원할지, 쓰기까지 지원할지 명확히 구분되어야 한다.
- **접두어 미선언**: 축약 IRI의 접두어가 등록되지 않았을 때 명확한 오류.

## Requirements *(mandatory)*

### Functional Requirements

**아키텍처 제약**

- **FR-001**: 어댑터는 스펙 001의 저장 엔진과 스펙 002의 타입 시스템 위에서 동작해야 하며,
  별도의 트리플 저장 구조를 만들어서는 안 된다.
- **FR-002**: SPARQL은 스펙 003의 논리 계획으로 컴파일되어야 하며, 별도 실행 엔진을 만들어서는
  안 된다.
- **FR-003**: 어댑터는 코어 데이터 모델의 의미론을 변경해서는 안 된다. 어댑터 전용 개념은
  어댑터 계층에 머물러야 한다.

**RDF 적재/내보내기**

- **FR-004**: 시스템은 Turtle, N-Triples, N-Quads, RDF/XML, JSON-LD 형식의 적재를 지원해야
  한다.
- **FR-005**: 시스템은 동일 형식들로 내보내기를 지원해야 하며, 적재 → 내보내기 round-trip이
  의미적으로 동등해야 한다.
- **FR-006**: 시스템은 IRI를 노드·타입·프로퍼티의 안정적 식별자로 보존해야 하며, 접두어
  등록·축약을 지원해야 한다.
- **FR-007**: 시스템은 리터럴의 `xsd:` 데이터타입과 언어 태그를 보존해야 한다.
- **FR-008**: 시스템은 명명된 그래프(named graph)를 지원해야 한다.
- **FR-009**: 시스템은 blank node를 지원하며 의미적 동등성을 보존해야 한다.
- **FR-010**: 매핑 불가능하거나 손실이 발생하는 구성요소는 조용히 버려지지 않고 보고되어야
  하며, 원본 트리플을 보존하는 fallback 저장이 있어야 한다.

**온톨로지 매핑**

- **FR-011**: 시스템은 `rdfs:Class`/`owl:Class` 를 entity 타입으로, `rdfs:subClassOf` 를
  타입 상속으로 매핑해야 한다.
- **FR-012**: 시스템은 `owl:ObjectProperty` 를 관계 타입으로, `owl:DatatypeProperty` 를
  attribute 타입으로 매핑해야 한다.
- **FR-013**: 시스템은 `rdfs:domain`/`rdfs:range` 를 role 타입 제약(002)으로 매핑해야 한다.
- **FR-014**: 시스템은 `owl:FunctionalProperty` 등 카디널리티 공리를 002의 카디널리티 제약으로
  매핑해야 한다.
- **FR-015**: 매핑 결과는 사람이 검토 가능한 매핑 보고서로 제공되어야 한다.

**SPARQL**

- **FR-016**: 시스템은 SPARQL 1.1 Query(SELECT, ASK, CONSTRUCT, DESCRIBE)를 지원해야 한다.
- **FR-017**: 시스템은 SPARQL 결과를 표준 형식(SPARQL Results JSON/XML/CSV)으로 반환해야 한다.
- **FR-018**: 시스템은 SPARQL 프로토콜 호환 엔드포인트를 제공해야 하며, 기존 SPARQL 클라이언트가
  수정 없이 접속 가능해야 한다.
- **FR-019**: 동일 의미의 SPARQL 질의와 Cypher 질의는 동일한 결과를 반환해야 한다.
- **FR-020**: 시스템은 SPARQL 지원 범위를 기능별 매트릭스로 문서화하고, 미지원 기능 사용 시
  명확한 오류를 반환해야 한다.
- **FR-021**: SPARQL UPDATE 지원 여부는 명시적으로 선언되어야 하며, 지원 시 트랜잭션 의미론을
  따라야 한다.

**추론**

- **FR-022**: 시스템은 OWL 2 RL 프로파일의 부분집합을 지원해야 하며, 지원 범위를 명시적으로
  문서화해야 한다.
- **FR-023**: 추론된 사실은 명시 사실과 구분 가능해야 하며, 추론 근거를 조회할 수 있어야 한다.
- **FR-024**: 추론은 질의 단위로 켜고 끌 수 있어야 한다.
- **FR-025**: 추론 확장에는 깊이·시간·결과 수 상한이 적용되어야 한다.
- **FR-026**: 미지원 공리는 적재 시점에 보고되어야 하며, 조용히 무시되어서는 안 된다.

**SHACL**

- **FR-027**: 시스템은 SHACL Core 형태(shape) 기반 검증을 지원하고 표준 형식의 위반 보고서를
  생성해야 한다.
- **FR-028**: 시스템은 스키마 제약으로 표현 가능한 shape를 002의 제약으로 승격하는 경로를
  제공해야 하며, 승격 불가 shape는 이유와 함께 보고해야 한다.

**GraphQL**

- **FR-029**: 시스템은 타입 카탈로그로부터 GraphQL 스키마를 생성할 수 있어야 한다.
- **FR-030**: 중첩 GraphQL 질의는 단일 그래프 질의로 처리되어야 하며, N+1 질의를 발생시켜서는
  안 된다.

### Key Entities

- **IRI Registry**: IRI ↔ 내부 식별자 매핑과 접두어 등록부.
- **RDF Literal**: 값, `xsd:` 데이터타입 IRI, 언어 태그를 갖는 리터럴 표현.
- **Named Graph**: 트리플의 소속 그래프 구분.
- **Ontology Mapping Report**: OWL/RDFS → 타입 시스템 매핑 결과와 미매핑 구성요소 목록.
- **SPARQL Query Plan**: SPARQL AST → 공용 논리 계획 변환 결과.
- **SHACL Shape**: 검증 형태 정의와 스키마 제약 승격 상태.
- **Inference Axiom**: OWL 공리와 그 활성화 상태, 지원 프로파일 표시.
- **Fallback Triple Store**: 매핑 불가 구성요소를 원형 보존하는 영역(코어 모델과 분리된 보조
  저장이 아니라, 코어 위의 예비 표현).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 공개 온톨로지 **5종 이상**(schema.org, FOAF, SKOS, Dublin Core, 산업 표준 1종)이
  경고 없이 또는 문서화된 알려진 한계만으로 적재된다.
- **SC-002**: RDF 적재 → 내보내기 round-trip에서 의미적 동등성이 **99% 이상**의 트리플에 대해
  보존되며, 손실 항목은 100% 보고된다.
- **SC-003**: SPARQL 1.1 Query 테스트 스위트 통과율이 **70% 이상**이며 회귀하지 않는다.
- **SC-004**: 동일 의미의 SPARQL/Cypher 질의 쌍 **50개**에 대해 결과가 100% 일치한다.
- **SC-005**: SPARQL 질의의 실행 계획이 대응 Cypher 질의와 동일한 물리 연산자를 사용함이
  자동 테스트로 검증된다.
- **SC-006**: 기존 SPARQL 클라이언트 **2종 이상**이 코드 수정 없이 엔드포인트에 접속해 질의
  가능하다.
- **SC-007**: 10만 클래스 규모 온톨로지 적재가 **10분 이내** 완료되고 상속 인덱스가 구축된다.
- **SC-008**: OWL 2 RL 지원 규칙별 테스트 케이스의 **100%** 가 통과한다(지원 선언 범위 내).
- **SC-009**: SHACL Core 검증 결과가 참조 구현과 **95% 이상** 일치한다.
- **SC-010**: 추론 활성화로 인한 질의 지연 증가가 지원 규칙 집합에서 **3배 이내**다.

## Assumptions

- Cypher가 중심 언어이며, SPARQL·GraphQL은 어댑터다. 어댑터 기능 요구가 Cypher 코어 의미론과
  충돌하면 Cypher가 우선한다.
- 완전한 OWL 2 DL 추론(tableau reasoner)은 범위 밖이다. OWL 2 RL 프로파일의 부분집합으로
  시작하고, 지원 범위를 명시적으로 문서화한다.
- 관계 특성 기반 추론의 기반 메커니즘은 스펙 **002-ontology-type-system** 이 제공하며, 본
  스펙은 OWL 공리를 그 위로 매핑한다.
- SPARQL 엔드포인트의 HTTP 프로토콜 계층은 PostgREST 또는 경량 프록시로 구현할 수 있으며,
  구현 방식은 plan 단계에서 결정한다.
- RDF-star / SPARQL-star는 초기 범위 밖이나, 관계에 프로퍼티를 붙이는 속성 그래프 모델이 이를
  자연스럽게 수용할 수 있으므로 로드맵에 둔다.
- SPARQL UPDATE는 초기 릴리스에서 읽기 전용으로 시작하고, 쓰기 지원은 후속 릴리스로 미룬다.
- 어댑터 성능은 Cypher 네이티브 경로보다 느릴 수 있으며, 그 차이를 문서화한다. 다만 별도 실행
  엔진을 만들지 않으므로 코어 성능 개선이 어댑터에도 그대로 전달되어야 한다.
