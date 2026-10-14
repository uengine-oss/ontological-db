# 스키마 진화 (Schema Evolution) 사용자 매뉴얼

> **이 문서가 답하는 질문**
> - 데이터가 이미 들어 있는 그래프의 스키마를 서비스를 멈추지 않고 어떻게 바꾸는가?
> - 어떤 변경은 데이터 양과 상관없이 공짜이고, 어떤 변경은 행을 다시 쓰는가?
> - 각 단계(expand / migrate / validate / contract)는 정확히 무엇을 하고 무엇을 잠그는가?
> - 실패하면 무엇을 보고, 어떻게 되돌리는가?

직접 따라 해 보려면 [튜토리얼](../tutorials/schema-evolution-tutorial.md)과
실행 가능한 스크립트 [`examples/tutorials/schema_evolution.sql`](../../examples/tutorials/schema_evolution.sql)
을 보세요. 측정된 비용(행 수에 따른 소요 시간, Neo4j·Apache AGE 비교)은
[schema-evolution-benchmark.md](../schema-evolution-benchmark.md) 에 있습니다.

근거 코드: [`engine/src/catalog/evolution.rs`](../../engine/src/catalog/evolution.rs),
스냅샷은 [`engine/sql/access.sql`](../../engine/sql/access.sql) 의 `og__schema_snapshot`,
동작 검증은 [`engine/tests/sql/08_schema_evolution.sql`](../../engine/tests/sql/08_schema_evolution.sql)
과 [`engine/tests/sql/10_evolution_temporal_coverage.sql`](../../engine/tests/sql/10_evolution_temporal_coverage.sql).

---

## 1. 왜 네 단계로 나누는가

데이터가 있는 그래프에서 "`age` 를 문자열에서 정수로 바꾼다"는 한 문장이 아닙니다.
`ALTER TABLE … ALTER COLUMN … TYPE` 한 번으로 끝내면

- 테이블 전체를 다시 쓰는 동안 **ACCESS EXCLUSIVE 잠금**이 걸려 읽기·쓰기가 모두 멈추고,
- 변환할 수 없는 값(`"abc"`) 하나 때문에 **전체가 실패**하며, 어느 행이 문제인지 알려 주지 않고,
- 그 사이 옛 형식으로 쓰는 애플리케이션이 **즉시 깨집니다.**

Ontological 은 이를 Expand → Migrate → Validate → Contract 의 네 호출로 나눕니다.

| 단계 | 하는 일 | 막히는 것 |
|---|---|---|
| **expand** | 옛 구조 옆에 새 구조를 추가. 이때부터 **새 쓰기는 새 규칙을 지켜야** 한다 | 짧은 카탈로그/DDL 잠금 |
| **migrate** | 기존 행을 `batch` 행씩 옮긴다. 여러 번 호출, 중단 후 재개 가능 | 없음 (행 단위 UPDATE) |
| **validate** | 데이터가 새 규칙을 만족하는지 검사. 위반 건수와 **위반 행 id(또는 값) 샘플**을 보고 | 없음 |
| **contract** | 옛 구조를 지우고 새 구조가 이름을 이어받는다 | 짧은 카탈로그/DDL 잠금 |

변경 하나는 `og_catalog.schema_change` 의 한 행입니다. 단계(phase), 전후 스키마 버전,
그리고 `report`(분석 결과·진행 커서·위반 샘플)를 기록하므로, 스키마 변경이 "누가 한 번
돌린 스크립트"가 아니라 **조회·감사 가능한 일급 객체**가 됩니다.

```
planned ──expand──▶ expanded ──migrate──▶ migrating ──validate(0건)──▶ validated ──contract──▶ contracted
   │                   │                      │                            │
   └──────────── abort ┴──────────────────────┴────────────────────────────┴──▶ aborted
```

- `validate` 는 `expanded`, `migrating`, `validated` 에서 부를 수 있고, 위반이 0 건일 때만 `validated` 로 넘어갑니다. 위반이 있으면 단계는 그대로입니다.
- `migrate` 는 `expanded`, `migrating` 에서만 부를 수 있습니다. (한 번도 부르지 않고 validate 로 가도 됩니다 — 옮길 것이 없는 변경은 그렇게 씁니다.)
- `contract` 는 `validated` 에서만 됩니다.
- `abort` 는 `contracted`/`aborted` 가 아니면 언제든 됩니다.

---

## 2. 데이터 양과 무관한 변경 vs 행을 다시 쓰는 변경

Ontological 에서 프로퍼티 이름·타입 이름·계층상의 위치는 **카탈로그 항목**이고, 물리 저장소를
가리킬 뿐입니다. 물리 컬럼이 `p_<name>` 인 것은 처음 만들 때의 이름일 뿐이고, 테이블은
`og_data.n_<type_id>` 입니다. 노드 id 는 타입 id 를 품고 있지만 이름은 품지 않습니다. 그래서:

| 변경 | 행을 다시 쓰는가 | 비고 |
|---|---|---|
| `rename_property` | **아니오** (카탈로그만) | 같은 컬럼에 이름을 하나 더 붙였다가 옛 이름을 뗀다 |
| `rename_type` | **아니오** (카탈로그만) | id 불변. 별칭 뷰만 옮긴다 |
| `add_parent` / `remove_parent` | **아니오** | 타입 DAG 의 구간 라벨을 다시 매길 뿐. 상속 컬럼은 재작성 없는 `ADD COLUMN`. 테스트가 `xmin` 불변으로 증명 |
| `drop_property` | **아니오** | PostgreSQL 의 `DROP COLUMN` 은 메타데이터만 바꾼다 |
| `add_property` | 대체로 아니오 | nullable `ADD COLUMN` 은 재작성 없음. 단, 그동안 `__ext`(선언 안 된 프로퍼티 저장소)에 있던 값은 컬럼으로 옮긴다 |
| `alter_property_type` | **예** | 표현(타입)이 바뀌므로 모든 행을 캐스팅. 배치로 나눠 수행 |
| `set_required` | 부분적 | NULL 인 행만 기본값으로 채운다. 검증은 전체 스캔(단, 쓰기는 막지 않음) |
| `add_unique` | 스캔 | 중복 검사 스캔 + contract 에서 유니크 인덱스 생성 |
| `alter_role` | 스캔 | 기존 관계의 역할 수행자를 전부 검사 |

주의: **`og_schema_plan` 자체도 스캔할 수 있습니다.** 영향 분석(변환 불가 값 수, NULL 수,
잃게 될 값 수, 중복 수)을 위해 `count(*)` 를 돌리기 때문입니다. 큰 테이블에서는 plan 도
운영 시간대를 고려해 부르세요.

측정 수치는 [schema-evolution-benchmark.md](../schema-evolution-benchmark.md) 를 보세요.

---

## 3. SQL API

| 함수 | 반환 | 설명 |
|---|---|---|
| `og_schema_plan(graph text, change jsonb)` | `int` (change_id) | 변경을 기록하고 영향을 분석해 `report` 에 저장. **아무것도 바꾸지 않는다** |
| `og_schema_expand(change_id int)` | `jsonb` (report) | 새 구조 추가. 스키마 버전 증가 |
| `og_schema_migrate(change_id int, batch int = 10000)` | `jsonb` `{change_id, moved, done, migrated}` | 한 번 호출에 **테이블마다** 최대 `batch` 행 이동 |
| `og_schema_validate(change_id int)` | `jsonb` `{change_id, phase, violations, samples}` | 위반 0 이면 `validated` |
| `og_schema_contract(change_id int)` | `jsonb` (report) | 옛 구조 제거. 스키마 버전 증가 |
| `og_schema_abort(change_id int)` | `jsonb` `{change_id, phase:"aborted"}` | expand 되돌리기 |
| `og_schema_apply(graph text, change jsonb)` | `jsonb` (report) | 위 전부를 한 트랜잭션에서 |
| `og_schema_changes(graph text)` | `TABLE(change_id, kind, phase, spec, report, created_by, created_at, updated_at)` | 그 그래프의 변경 기록, 최신순 |
| `og_schema_snapshot(graph text, version int8 = NULL, at timestamptz = NULL)` | `jsonb` | 그 버전(또는 시각) 시점의 스키마 전체 |

### 3.1 변경 종류와 JSON

모든 변경 JSON 은 `"kind"` 와 종류별 키를 가집니다. `type` 은 타입 이름, `property` 는 프로퍼티
이름입니다. 데이터 타입 이름은 `og_add_property` 와 같습니다 (`string`, `int`, `long`, `float`,
`bool`, `datetime`, `date`, `uuid`, `numeric`, `jsonb`, `text[]`, `int[]`, `vector(N)`).

| kind | JSON |
|---|---|
| `add_property` | `{"kind":"add_property","type":"Person","property":"nick","data_type":"string"}` |
| `drop_property` | `{"kind":"drop_property","type":"Person","property":"age"}` |
| `rename_property` | `{"kind":"rename_property","type":"Person","property":"name","new_name":"fullName"}` |
| `alter_property_type` | `{"kind":"alter_property_type","type":"Person","property":"age","to":"int"}` |
| `set_required` | `{"kind":"set_required","type":"Person","property":"age","default":0}` (`default` 생략 가능) |
| `add_unique` | `{"kind":"add_unique","type":"Person","property":"email"}` |
| `rename_type` | `{"kind":"rename_type","type":"Employee","new_name":"Staff"}` |
| `alter_role` | `{"kind":"alter_role","relation":"WORKS_AT","role":"employer","player":"Org","card_min":1,"card_max":1}` (`card_*` 생략 가능) |
| `add_parent` | `{"kind":"add_parent","type":"Person","parent":"Party"}` |
| `remove_parent` | `{"kind":"remove_parent","type":"Person","parent":"Party"}` |

변경은 타입 **자신과 모든 하위 타입**의 저장 테이블에 적용됩니다.

### 3.2 종류별로 각 단계가 하는 일

| kind | plan 이 보고하는 것 | expand | migrate | validate | contract | abort |
|---|---|---|---|---|---|---|
| `add_property` | `values_in_ext`: `__ext` 에 이미 있는 값 수 | `og_add_property` 실행 — `ADD COLUMN` + `__ext` 값 이동. **이 시점에 변경이 사실상 끝난다** | 없음 | 항상 0 | 없음 | **프로퍼티를 지우지 않는다** (expand 가 이미 완결) |
| `drop_property` | `values_lost`: 사라질 값 수 | 없음 | 없음 | 항상 0 | `DROP COLUMN`, 카탈로그 삭제, 뷰 재생성 | 없음 |
| `rename_property` | 잠금 메모 | 같은 컬럼에 새 이름을 추가 — **두 이름 모두 읽기·쓰기 가능** | 없음 | 항상 0 | 옛 이름 삭제, `write_grant`·호환 인덱스의 프로퍼티 이름도 따라 바꿈 | 새 이름 삭제 |
| `alter_property_type` | `uncastable`, `uncastable_samples`(노드 id), `shadow` 컬럼 이름 | 그림자 컬럼 `p_<col>__s<id>` 추가, **이중 쓰기** 트리거 `og_c_shadow_<id>` 부착 | 텍스트를 거쳐 캐스팅, id 커서로 배치 이동. 이력 테이블도 | 옛 값이 있는데 새 값이 없는 행 수(이력 포함) | 트리거 제거, 옛 컬럼 삭제, 그림자 컬럼 이름 변경, 인덱스 재생성, 컬럼 권한 재적용 | 트리거와 그림자 컬럼 삭제 |
| `set_required` | `missing`, `missing_samples`(id), `fill` | `CHECK (col IS NOT NULL) NOT VALID` 추가 — 새 쓰기 즉시 강제, 기존 행은 스캔 안 함 | `default` 로 NULL 채우기(배치). `default` 없으면 아무것도 안 함 | NULL 행 수. 0 이면 `VALIDATE CONSTRAINT` | `SET NOT NULL`(검증된 CHECK 덕분에 스캔 없음), CHECK 삭제 | CHECK 삭제 |
| `add_unique` | `duplicate_values`, `duplicate_samples`(**값**) | 없음 | 없음 | 계층 전체에서 중복된 값 | 구체 테이블마다 유니크 인덱스 `uq_<type_id>_<col>` 생성 | 없음 |
| `rename_type` | 잠금 메모 | 없음 | 없음 | 항상 0 | 이름 변경, 별칭 뷰 이동. **옛 이름은 이 순간부터 안 통한다** | 없음 |
| `alter_role` | `violations`, `violation_samples`(관계 id) | 없음 | 없음 | 새 player 타입(또는 그 하위)이 아닌 역할 수행자 | 역할의 player·카디널리티 갱신 | 없음 |
| `add_parent` | `gains_properties`(물려받을 프로퍼티), `rows_touched: 0` | 없음 | 없음 | 항상 0 | 부모 연결, 구간 재라벨링, 상속 컬럼 추가, 스탬핑·시간버전·쓰기 권한 상속 | 없음 |
| `remove_parent` | `rows_touched: 0` | 없음 | 없음 | 항상 0 | 부모 연결 해제, 재라벨링. 상속받던 프로퍼티는 **자기 것으로 남는다** | 없음 |

`rename_property` 와 `rename_type` 의 차이에 주의하세요. 프로퍼티 이름 변경은 expand 부터
contract 까지 **두 이름이 공존**하므로 애플리케이션을 순서대로 배포할 수 있지만, 타입 이름
변경은 공존 기간이 없습니다. contract 순간 옛 라벨(`MATCH (:Employee)`)은 더 이상 그 타입을
가리키지 않습니다.

### 3.3 잠금

코드가 `report.lock` 에 적는 내용 그대로입니다.

| kind | 잠금 |
|---|---|
| `add_property` | 테이블마다 짧은 ACCESS EXCLUSIVE (`ADD COLUMN`, 재작성 없음) |
| `alter_property_type` | expand(`ADD COLUMN`)와 contract(`DROP`/`RENAME`)에서 짧은 ACCESS EXCLUSIVE. migrate 는 일반 행 UPDATE |
| `set_required` | expand 의 `NOT VALID` CHECK 는 짧은 잠금. validate 의 `VALIDATE CONSTRAINT` 는 SHARE UPDATE EXCLUSIVE — **쓰기는 계속된다** |
| `rename_property`, `rename_type` | 카탈로그만. 테이블 재작성 없음 |
| `add_parent` / `remove_parent` | 카탈로그 + 재작성 없는 `ADD COLUMN` |

expand 와 contract 는 스키마 버전을 올립니다. 그러면 Cypher 플랜 캐시가 무효화되고 생성 뷰가
다시 만들어지므로, 다음 질의부터 새 구조를 봅니다.

---

## 4. 이중 쓰기 (alter_property_type)

expand 와 contract 사이에 그 타입(또는 하위 타입)에 대한 쓰기는 **옛 컬럼과 그림자 컬럼을
둘 다** 채웁니다. 이중 쓰기는 expand 가 구체 테이블마다 붙이는 한시 트리거
`og_c_shadow_<change_id>`(`BEFORE INSERT OR UPDATE OF <col>`, 함수 `og__shadow_write`)가 하고
contract·abort 가 뗍니다. 엔진이 아니라 테이블이 하는 일이므로 **Cypher 든 원시 SQL 이든 같습니다.**
그래서:

- **읽기는 여전히 옛 컬럼**을 봅니다. `"60"` 은 contract 전까지 문자열 `"60"` 으로 나옵니다.
- **새 쓰기는 새 타입으로 캐스팅 가능해야** 합니다. `age: "old"` 는 expand 직후부터
  `22P02 invalid input syntax for type integer` 로 거부됩니다. 원시 SQL
  `UPDATE … SET p_age = 'x'` 도 같은 오류로 거부됩니다.
- 이미 migrate 된 행을 원시 SQL 로 바꿔도 그림자 컬럼이 따라 바뀌므로, 그 값은 contract 뒤에도
  남습니다.
- migrate 가 옮기지 못한 행(변환 불가 값)은 **평소처럼 Cypher 로 고치면** 이중 쓰기가 그림자
  컬럼도 채웁니다. 그 뒤 validate 를 다시 부르면 됩니다.
- 그림자 컬럼만 바뀌는 UPDATE(=migrate 의 백필)는 옛 컬럼을 건드리지 않으므로 이중 쓰기
  트리거(`UPDATE OF <col>`)가 돌지 않습니다. 또 "보이는 변경"이 아니므로 스탬프
  (`__updated_at` 등)를 바꾸지 않고, 시간 버전 타입이라도 이력을 남기지 않습니다
  (`access.sql` 의 `og__visible`).

---

## 5. 보고서와 위반 샘플 읽기

`og_schema_plan` 이 돌려주는 것은 change_id 뿐이고, 분석 결과는 `report` 에 있습니다.

```sql
SELECT og_schema_plan('hr', '{"kind":"alter_property_type","type":"Person","property":"age","to":"int"}');
SELECT phase, jsonb_pretty(report) FROM og_schema_changes('hr') WHERE change_id = 1;
```

```
 planned | {
         |     "from": "text",
         |     "lock": "brief ACCESS EXCLUSIVE at expand (ADD COLUMN) and contract (DROP/RENAME)",
         |     "column": "p_age",
         |     "shadow": "p_age__s1",
         |     "to_sql": "int4",
         |     "type_id": 1,
         |     "uncastable": 1,
         |     "uncastable_samples": [ 68719476739 ]
         | }
```

- 샘플은 최대 10 개입니다. `alter_property_type`·`set_required` 는 **노드 id**,
  `alter_role` 은 **관계 id**, `add_unique` 는 **중복된 값 자체**입니다.
- id 로 그 노드를 보려면: `og_cypher('hr', 'MATCH (p) WHERE id(p) = 68719476739 RETURN p')`.
- migrate/validate 가 진행되면서 `report` 에 `cursor`(테이블별 마지막 id), `migrated`,
  `migrated_last_call`, `migration_done`, `violations`, `violation_samples` 가 쌓입니다.

`og_schema_migrate` 의 반환값:

```
{"done": false, "moved": 3, "migrated": 3, "change_id": 1}
```

- `batch` 는 **테이블마다** 적용됩니다. `Person` 과 하위 타입 `Employee` 는 테이블이 다르므로
  `batch = 2` 여도 한 번에 3 행이 옮겨질 수 있습니다.
- `done` 은 "이번 호출에서 옮긴 행이 0" 이라는 뜻입니다. 그래서 **마지막에 한 번 더** 불러야
  `done: true` 가 나옵니다. 이후에 더 불러도 무해합니다.
- 각 호출은 자체 트랜잭션(자동 커밋이면)이므로, 도중에 끊겨도 커서부터 이어집니다.
- 이력 테이블(시간 버전 타입)은 키가 없어 페이지 단위가 아니라 호출마다 한 문장으로 옮깁니다.

반복 호출 예:

```sql
DO $$
DECLARE r jsonb;
BEGIN
    LOOP
        r := og_schema_migrate(1, 10000);
        EXIT WHEN (r->>'done')::bool;
    END LOOP;
END $$;
```

주의: 위 `DO` 블록은 **한 트랜잭션**입니다. 행 잠금을 짧게 유지하려면 애플리케이션이나
`psql` 에서 호출마다 커밋하세요 (예: 셸 루프, 또는 `CALL` 가능한 프로시저에서 `COMMIT`).

---

## 6. 되돌리기 — `og_schema_abort`

contract 전이라면 언제든 버릴 수 있습니다.

| kind | abort 가 하는 일 |
|---|---|
| `alter_property_type` | 이중 쓰기 트리거 제거, 그림자 컬럼(과 이력 테이블의 그림자 컬럼) 삭제 |
| `set_required` | `og_req_<id>` CHECK 삭제 |
| `rename_property` | 추가했던 새 이름 삭제 |
| `add_property` | **아무것도 안 함** — expand 에서 이미 프로퍼티가 생겼다. 지우려면 `drop_property` |
| 그 밖 | 구조를 바꾸지 않았으므로 단계만 `aborted` |

`planned` 상태에서 abort 하면 스키마 버전도 올리지 않습니다. contract 된 변경은 abort 할 수
없습니다. 반대 방향의 새 변경(예: `int → string`)을 계획하세요.

---

## 7. `og_schema_apply` vs 단계별 호출

`og_schema_apply(graph, change)` 는 plan → expand → migrate(10만 행씩 끝까지) → validate →
contract 를 **하나의 함수 호출 = 하나의 트랜잭션**으로 실행합니다.

- 검증에 실패하면 `OG_SCHEMA_VIOLATION`(SQLSTATE `23514`)으로 실패하고 **아무것도 바뀌지 않습니다.**
  변경 기록조차 롤백되므로 change_id 에 빈 번호가 생깁니다.
  ```
  23514 OG_SCHEMA_VIOLATION: 6 row(s) violate the change; nothing was applied. samples: [68719476738, …]
  ```
  힌트: `use og_schema_plan / expand / migrate / validate to fix the data before contracting`
- 트랜잭션 하나이므로 expand 가 잡은 잠금(예: `ADD COLUMN` 의 ACCESS EXCLUSIVE)이
  **커밋까지 유지**됩니다. `alter_property_type` 을 큰 테이블에 apply 하면 마이그레이션 내내
  그 테이블의 읽기·쓰기가 막힙니다.

그래서:

| 이런 경우 | 이렇게 |
|---|---|
| 카탈로그만 바꾸는 변경 (`rename_type`, `add_parent`, `remove_parent`, `drop_property`) | `og_schema_apply` 로 충분 |
| 작은 그래프, 개발/테스트 환경 | `og_schema_apply` |
| 행을 다시 쓰거나 스캔하는 변경을 운영 중인 큰 그래프에 | 단계별 호출, migrate 는 호출마다 커밋 |
| 애플리케이션 배포와 맞물린 이름 변경 (`rename_property`) | 단계별 — expand 후 새 앱 배포, 옛 앱 퇴역 후 contract |
| 데이터 정리가 필요할지 모를 때 | 먼저 `og_schema_plan` 만 부르고 report 를 본다 |

---

## 8. 변경 이력과 스키마 스냅샷

```sql
SELECT change_id, kind, phase, spec, report, created_by, created_at, updated_at
  FROM og_schema_changes('hr');
```

`og_catalog.schema_change` 에는 `from_version`(plan 시점의 스키마 버전)과
`to_version`(contract/abort 시점)도 있습니다. 스키마 버전은 **모든 그래프가 공유하는 하나의
시퀀스**(`og_catalog.schema_version_seq`)이므로 번호가 그래프 안에서 연속이지 않을 수 있습니다.

스키마 버전이 오를 때마다 그래프의 카탈로그 전체가 `og_catalog.schema_version.snapshot` 에
JSON 으로 저장됩니다.

```sql
SELECT og_schema_snapshot('hr');                         -- 현재
SELECT og_schema_snapshot('hr', 6);                      -- 버전 6 이하 중 최신
SELECT og_schema_snapshot('hr', at => '2026-09-01');     -- 그 시각 이전 중 최신
```

반환 형태:

```json
{ "version": 23, "changed_at": "…", "description": "schema change 7 aborted",
  "types": [ { "name": "Person", "kind": "e", "abstract": false, "parents": ["Party"],
               "properties": { "age": {"type": "int4", "column": "p_age", "required": true, "key": false}, … },
               "roles": {}, "temporal": false, "stamped": false }, … ] }
```

`properties.*.column` 을 보면 이름 변경 후에도 물리 컬럼이 그대로(`fullName` → `p_name`)임을
확인할 수 있습니다. 시간 버전 기능과 함께 쓰면 "이 행은 어떤 스키마 아래서 쓰였나"에 답합니다
([temporal.md](temporal.md) §1).

---

## 9. 오류 코드

| 상황 | 메시지 | SQLSTATE |
|---|---|---|
| 단계가 맞지 않음 (예: validate 전에 contract) | `OG_SCHEMA_PHASE: change 1 is 'migrating'; contract needs it to be validated` | `55000` |
| apply 중 검증 실패 | `OG_SCHEMA_VIOLATION: N row(s) violate the change; nothing was applied. samples: […]` | `23514` |
| alter_property_type expand 후 새 타입으로 캐스팅 안 되는 쓰기 (원시 SQL 포함) | 예: `invalid input syntax for type integer: "old"` | `22P02` 등 캐스팅 오류 |
| set_required expand 후 값 없는 쓰기 | `new row for relation "n_1" violates check constraint "og_req_3"` | `23514` |
| add_unique contract 후 중복 쓰기 | `duplicate key value violates unique constraint "uq_…"` | `23505` |
| 알 수 없는 kind | `unknown schema change kind 'x'. one of: …` | `XX000` |
| 필수 키 누락 | `schema change needs "property"` | `XX000` |
| 상위 타입에서 물려받은 프로퍼티를 하위 타입에서 변경 | `'fullName' is inherited from 'Person'; change it there` | `XX000` |
| 없는 프로퍼티 | `property 'x' is not declared on 'Person'` | `XX000` |
| 이미 같은 타입 | `'age' is already int4` | `XX000` |
| 이름 충돌 | `'fullName' is already a property of 'Person' or a subtype` / `type 'Staff' already exists` | `XX000` |
| 예약어로 이름 변경 (`_id`, `__x` 등) | `'__x' is a reserved name` | `XX000` |
| add_parent 순환 | `'Party' is below 'Person'; that would be a cycle` | `XX000` |
| add_parent 종류 불일치 / 타입 충돌 | `… are different kinds of type` / `'x' is declared on both with different types` | `XX000` |
| `add_property` 에 `"required": true` | `add the property optional, then use a set_required change` | `XX000` |
| 없는 change_id | `schema change 99 does not exist` | `XX000` |

`XX000` 으로 적은 것은 plan/expand 단계의 입력 검증 오류로, 구체적인 SQLSTATE 가 붙지 않은
일반 오류입니다. plan 에서 나는 오류는 변경을 기록하지 않습니다.

---

## 10. 제약과 아직 안 되는 것

- **타입 분할/병합 없음.** 노드 id 가 타입 id 를 품고 있어서, 노드를 다른 타입으로 옮기려면
  id 를 새로 매기고 그 노드를 가리키는 모든 인접 목록을 다시 써야 합니다. 이 파이프라인에는
  들어 있지 않습니다.
- **TypeQL `redefine` / `undefine` 은 이 파이프라인에 연결되어 있지 않습니다.** 스키마 변경은
  위 SQL 함수로 하세요.
- **유니크는 구체 테이블 단위입니다.** `add_unique` 의 validate 는 계층 전체에서 중복을 찾지만,
  contract 가 만드는 인덱스는 테이블(구체 타입)마다 하나라서, 그 뒤에 `Person` 과 `Employee`
  사이에 같은 값을 쓰는 것은 막지 못합니다 (`og_add_property(is_key)` 와 같은 한계).
- 변경은 **선언된 타입에서만** 할 수 있습니다. 하위 타입의 복사본을 바꾸면 둘이 어긋나기 때문입니다.
- `add_unique` 의 validate 와 contract 사이에 들어온 중복은 contract 의 인덱스 생성에서 실패합니다.
  validate 를 contract 직전에 다시 부르세요.
- `add_property` 의 plan 은 프로퍼티가 이미 선언되어 있는지 확인하지 않습니다. 이미 있으면
  expand 가 카탈로그 중복 키 오류로 실패합니다. (소유자 권한으로 선언 안 된 프로퍼티를 쓰면
  자동으로 컬럼으로 승격되므로, 이 kind 는 주로 `__ext` 에 쌓인 값을 명시적으로 선언할 때 씁니다.)
- `set_required` 의 기본값 채우기는 일반 UPDATE 이므로, 시간 버전 타입이면 채운 행마다 이력이 남습니다.

<!-- affects: docs/tutorials/schema-evolution-tutorial.md, examples/tutorials/schema_evolution.sql -->
<!-- requires-update: engine/src/catalog/evolution.rs -->
