# ADR-003: 식별자를 `[shard:9][type:18][local:36]` int8 비트필드로 인코딩한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-06 (`specs/001-graph-storage-engine/plan.md` 기준일) |
| 영향 범위 | storage, cypher, typeql, cluster |
| 근거 | `engine/src/id.rs:1-12`, `specs/001-graph-storage-engine/plan.md` "식별자 인코딩", `.specify/memory/constitution.md` 원칙 III, `engine/sql/bootstrap.sql:47` |

> **이 문서가 답하는 질문**
> - 노드/엣지 ID가 왜 UUID도 문자열도 아닌 int8인가?
> - 왜 상위 9비트가 지금은 항상 0인데도 예약되어 있는가?

## 배경

헌법 원칙 III는 *"식별자는 (그래프, 타입, 지역 오프셋)을 인코딩한 **고정폭 정수**여야 한다.
문자열 키 조인으로 트래버설하는 설계는 금지한다"* 고 못 박는다. 문제는 **어떤 정보를 ID
안에 넣을 것인가**였다.

## 고려한 선택지

1. **불투명 시퀀스 int8** — 단순. 그러나 "이 노드가 어느 타입 테이블에 있는가"를 알려면
   `og_data.og_node` 조인이 매번 필요하다.
2. **UUID / 문자열 키** — 분산에 유리하나 헌법이 명시적으로 금지. 16바이트는 인접 배열
   크기를 두 배로 만든다.
3. **비트필드 int8** — 상위 비트에 shard와 type_id를 박아 넣는다.

## 결정

**3안.** 64비트를 다음과 같이 나눈다 (`engine/src/id.rs:3-8`).

```text
 bit 63        54           36                              0
 +---+----------+------------+-------------------------------+
 | 0 | shard: 9 | type_id:18 |          local_id: 36         |
 +---+----------+------------+-------------------------------+
```

- bit63 = 0 으로 고정해 항상 양수 (`plan.md` "식별자 인코딩").
- `og_catalog.type_id_seq`의 `MAXVALUE 262143`(18비트)이 카탈로그 쪽에서 같은 제약을
  강제한다 (`engine/sql/bootstrap.sql:47`).
- 범위를 벗어나면 `make_id`가 `ereport(ERROR)`로 중단한다 — 잘린 ID가 저장에 도달할 수
  없게 하기 위함이다 (`engine/src/id.rs:31-44`).

## 근거

- `engine/src/id.rs:10-12` 원문: *"Everything a traversal touches is a fixed-width
  integer: the type of a node is a shift-and-mask, not a catalog join, and the shard
  bits are reserved up front so spec 007 can distribute without rewriting identifiers."*
- 이 인코딩이 상위 계층에서 실제로 회수되는 지점이 확인된다.
  `specs/010-typeql-query-surface/plan.md`: *"식별자의 18비트에 type_id가 박혀 있으므로,
  `has genre $g`의 장르 필터는 이웃 id에 대한 시프트-마스크다. 카탈로그 조인도, 별도
  인덱스도 필요 없다."*
- 8바이트 고정폭이기 때문에 인접 배열(`int8[]`)이 CHUNK 256개에서 4KB에 들어가고, 그것이
  ADR-004의 페이지 지역성 전제가 된다.

## 결과

**긍정적**
- 타입 판정이 조인이 아니라 시프트-마스크. `og_id_type`/`og_id_shard`/`og_id_local`로 SQL
  에서도 노출된다.
- 샤딩 도입 시 `local_id`가 불변이므로 재분배가 ID 재작성을 요구하지 않는다
  (`specs/007-distributed-cluster/plan.md` P1).

**부정적 / 감수한 대가**
- **타입당 노드 수가 2^36(약 687억)로, 그래프당 타입 수가 2^18(262,143)로 제한된다.**
  이 한계는 하드 에러로 드러나며 조용히 넘지 않는다.
- 샤드 9비트(512)는 지금 전혀 쓰이지 않는 채로 모든 ID에 실려 있다. 미구현 기능(ADR-021)에
  대한 선불이다.
- 타입을 삭제해도 `type_id`는 재사용되지 않는다 (`bootstrap.sql:31-33`: *"type_id is
  embedded in every node & edge identifier, therefore it is globally unique and never
  reused"*).

## 재검토 조건

- 단일 그래프의 타입 수가 262,143에 근접하거나(온톨로지 자동 생성 워크로드), 단일 타입의
  인스턴스가 2^36에 근접할 때 — 비트 배분을 재조정해야 하며, 이는 저장 포맷 변경이다.
- 샤드 수요가 512를 넘을 때. 현재 샤딩 자체가 미구현이므로(ADR-021) 실질적 트리거는
  샤딩 구현 착수 시점이다.

<!-- affects: storage, cypher, typeql, cluster -->
