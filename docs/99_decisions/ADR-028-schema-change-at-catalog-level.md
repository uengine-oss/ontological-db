# ADR-028: 스키마 변경은 가능한 한 카탈로그(매핑) 수준에서 해결하고, 표현이 바뀌는 변경만 행을 배치로 다시 쓴다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-09-30 (작성일. 커밋 전) |
| 영향 범위 | catalog, storage, cypher, security, temporal |
| 근거 | `engine/src/catalog/evolution.rs`(모듈 주석과 종류별 `expand`/`migrate`/`contract`), `engine/sql/bootstrap.sql`(`og_catalog.schema_change`), `engine/sql/access.sql`(`og__shadow_write` — 이중 쓰기 트리거), `engine/tests/sql/08_schema_evolution.sql`, `engine/tests/sql/10_evolution_temporal_coverage.sql`, [spec 012](../../specs/012-schema-evolution/plan.md) |

> **이 문서가 답하는 질문**
> - 속성 이름을 바꾸면 행이 다시 쓰이는가?
> - 어떤 스키마 변경이 데이터 크기에 비례하는 비용을 갖는가?
> - 운영 중 변경은 왜 네 번의 호출인가?

## 배경

스키마 변경의 비용은 보통 데이터 크기에 비례한다고 가정된다. 그러나 이 저장소의 저장 구조에서
이름은 저장소의 주소가 아니다.

- 속성 `name` 의 컬럼은 `p_name` 이지만, 그건 **태어날 때의 이름**일 뿐이다. 카탈로그
  `og_catalog.property(type_id, name, column_name)` 이 이름 → 컬럼을 매핑한다.
- 타입의 저장 테이블은 `og_data.n_<type_id>` 다. 이름이 아니라 id 다.
- 상속은 행의 속성이 아니라 카탈로그의 구간 라벨이다([ADR-007](ADR-007-nested-set-interval-labels.md)).
  노드 id 는 구체 타입 id 를 인코딩하지만([ADR-003](ADR-003-int8-identifier-bitfields.md)) 조상은
  인코딩하지 않는다.

그러므로 "이름을 바꾼다", "상위 타입을 끼워 넣는다"는 **카탈로그 항목의 편집**이다.

## 고려한 선택지

1. **물리적 변경.** 이름을 바꾸면 컬럼·테이블을 `RENAME` 하고, 재부모화는 행을 옮긴다(또는 id
   재발급). Neo4j 의 레이블 변경(`SET n:New REMOVE n:Old`)이 이 모양이고 비용이 노드 수에
   비례한다.
2. **카탈로그 수준 우선.** 이름·계층 변경은 카탈로그에서 끝내고, 저장소는 그대로 둔다.
   표현(컬럼 타입)이 바뀌는 변경만 행을 다시 쓰며, 그것은 Expand → Migrate(배치) → Validate →
   Contract 로 쪼갠다.
3. **모든 변경을 한 번에(트랜잭션 하나).** TypeDB `redefine` 모양. 작은 그래프에서는 편하지만
   큰 그래프에서 표현 변경은 긴 잠금이 된다.

## 결정

**2안, 그리고 3안은 `og_schema_apply` 로 편의 제공.**

| 종류 | 해결 수준 | 행 재작성 |
|---|---|---|
| `rename_property` | 카탈로그 — 같은 컬럼에 두 번째 이름, 축소 시 옛 이름 삭제 | 없음 |
| `rename_type` | 카탈로그 — 이름, 별칭 뷰 | 없음, id 불변 |
| `add_parent` / `remove_parent` | 카탈로그 — `type_parent` + 구간 재라벨, 상속 컬럼은 `ADD COLUMN`(재작성 없음) | 없음 (`xmin` 불변을 08 이 단언) |
| `drop_property` | `DROP COLUMN`(PG 에서 메타데이터 연산) + 카탈로그 | 없음 |
| `add_property` | `ADD COLUMN`(기본값 없음) | `__ext` 에 같은 키가 있던 행만 |
| `add_unique`, `alter_role` | 검사 + 인덱스/카탈로그 | 없음 |
| `set_required` | `NOT VALID` → `VALIDATE` → `SET NOT NULL` | 기본값을 채울 빈 행만, 배치 |
| `alter_property_type` | 그림자 컬럼 + 이중 쓰기(한시 트리거) + 텍스트 경유 캐스트 + 교체 | **전 행, 배치** |

변경마다 `og_catalog.schema_change` 한 행이 단계·보고·커서를 갖는다. 확장 이후의 새 쓰기는
새 규칙을 먼저 따른다(`NOT VALID` 제약, 그림자로의 캐스트). 표현 변경의 이중 쓰기는 확장이
전환 중인 테이블에 붙이고 축소·중단이 떼는 트리거(`og_c_shadow_<change_id>`)라서, 원시 SQL
쓰기도 그림자를 채우고 새 타입에 맞지 않으면 거부된다. 버전이 오를 때마다 카탈로그
스냅샷이 저장된다.

## 근거

- **대부분의 변경이 데이터 크기와 무관한 비용을 갖는다.** 카탈로그 행 몇 개, 구간 재라벨 한 번
  (타입 DAG 크기에 비례), 뷰 재생성. 측정: [`docs/schema-evolution-benchmark.md`](../schema-evolution-benchmark.md).
- **이 성질은 새로 만든 것이 아니라 이미 있던 것이다.** ADR-005 가 속성을 "카탈로그가 생성한
  컬럼"으로, ADR-007 이 상속을 "구간 라벨"로 정했을 때부터 이름과 계층은 저장소와 분리되어
  있었다. 이 결정은 그것을 스키마 변경에 쓴다.
- **표현 변경만은 피할 수 없다.** `text` 의 `"42"` 를 `int4` 의 42 로 만드는 것은 바이트를 바꾸는
  일이다. 그래서 그것만 배치·재개 가능하게 하고, 캐스트는 텍스트를 거친다(`'3.5'::int8` 은
  실패하지만 `3.5::int8` 은 조용히 반올림한다).

## 결과

**긍정적**

- 이름·계층 변경이 운영 중에도 짧은 카탈로그 잠금으로 끝난다.
- 변경 이력과 버전별 스냅샷이 질의 가능한 데이터다.
- 이력 테이블(ADR-027)도 같은 변경을 받는다; 삭제된 컬럼은 이력에 남는다.

**부정적 / 감수한 대가**

- **물리 이름과 논리 이름이 갈라진다.** `rename_property('name' → 'fullName')` 이후 컬럼은 여전히
  `p_name` 이다. 원시 SQL 사용자는 `og_catalog.property` 나 별칭 뷰를 봐야 한다.
- **타입 분리/병합은 이 방식으로 안 된다.** 노드 id 가 구체 타입을 인코딩하므로, 노드를 다른
  타입으로 옮기면 id 가 바뀌고 그 id 를 가리키는 모든 인접 항목·간선을 다시 써야 한다.
  구현하지 않았다.
- **표현 변경 중에는 그 컬럼의 쓰기마다 트리거가 돈다.** 이중 쓰기를 엔진 경로가 아니라
  트리거에 둔 것은 원시 SQL 쓰기도 덮기 위해서다. 트리거는 전환 중인 테이블의 해당 컬럼
  (`UPDATE OF <col>`)에만 붙으므로, 마이그레이션의 배치 채우기와 다른 컬럼의 쓰기는 비용을
  치르지 않는다([spec 012 plan](../../specs/012-schema-evolution/plan.md) Complexity Tracking).
- `add_unique` 와 `alter_role` 은 확장 단계에서 새 쓰기를 강제하지 않는다.
- TypeQL `redefine`/`undefine` 은 이 파이프라인을 거치지 않는다.

## 재검토 조건

- 타입 분리/병합이 요구될 때 — id 가 구체 타입을 인코딩한다는 ADR-003 의 전제와 충돌하므로,
  재식별 비용을 받아들일지 id 체계를 바꿀지를 함께 결정해야 한다.
- 벤치마크가 카탈로그 수준 변경의 비용이 데이터 크기와 무관하지 **않음**을 보이면(예: 뷰 재생성,
  재라벨 비용) 이 문서를 고친다.

<!-- affects: catalog, storage, cypher, security, temporal -->
