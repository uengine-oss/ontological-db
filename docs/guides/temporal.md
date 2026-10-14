# 바이템포럴 (시점) 질의 사용자 매뉴얼

> **이 문서가 답하는 질문**
> - "그때 실제로 어땠나"와 "그때 우리는 무엇을 알고 있었나"를 어떻게 따로 묻는가?
> - 소급 변경과 정정(오기 수정)은 어떻게 구분해서 쓰는가?
> - 질의 쪽은 무엇을 바꿔야 하는가? (답: 시각 하나만 지정한다)
> - 누가, 어느 시스템에서 썼는지는 어떻게 남고 어떻게 읽는가?
> - 어떤 질의 모양은 시점 질의를 지원하지 않는가?

직접 따라 해 보려면 [튜토리얼](../tutorials/temporal-tutorial.md)과
[`examples/tutorials/temporal.sql`](../../examples/tutorials/temporal.sql) 을 보세요.

근거 코드: [`engine/src/temporal/mod.rs`](../../engine/src/temporal/mod.rs),
트리거·설정 함수는 [`engine/sql/access.sql`](../../engine/sql/access.sql) 의
`og__temporal`, `og__stamp`, `og_valid_at`, `og_system_at`, `og_require_current`, `og_meta`,
Bolt 매핑은 [`bolt/src/session.rs`](../../bolt/src/session.rs) 의 `TX_SETTINGS`.
검증: [`engine/tests/sql/09_bitemporal.sql`](../../engine/tests/sql/09_bitemporal.sql),
[`07_write_security.sql`](../../engine/tests/sql/07_write_security.sql),
[`10_evolution_temporal_coverage.sql`](../../engine/tests/sql/10_evolution_temporal_coverage.sql),
[`tests/bolt-context/run.py`](../../tests/bolt-context/run.py).

---

## 1. 세 개의 축

| 축 | 묻는 것 | 저장 위치 | 질의에서 지정 |
|---|---|---|---|
| **valid time** (유효 시간) | 세상에서 그것이 **언제 사실이었나** | 행마다 `__valid tstzrange` | `og.valid_at` |
| **system time** (시스템 시간) | 우리가 그것을 **언제부터 믿고 있었나** | 현재 행 `__sys_from`, 이력 행 `[__sys_from, __sys_to)` | `og.system_at` |
| **schema version** | 그 행이 **어떤 스키마 아래서** 쓰였나 | `og_catalog.schema_version.snapshot` | `og_schema_snapshot(graph, at => …)` |

왜 셋 다 필요한가:

- **valid time 만 있으면** 정정과 변경을 구분할 수 없습니다. 2023 년 연봉을 200 으로 적었다가
  250 으로 고치면, "2023 년 연봉은?"의 답은 250 이지만, "작년 감사 때 보고한 2023 년 연봉은?"의
  답(200)은 사라집니다.
- **system time 만 있으면** 소급 변경을 표현할 수 없습니다. 오늘 입력했지만 1 월 1 일부로 적용되는
  인상은, "언제 입력했나"와 "언제부터 유효한가"가 다릅니다.
- **스키마 버전**이 없으면 옛 행을 읽을 때 그 행이 쓰일 당시 `age` 가 문자열이었는지, 어떤
  프로퍼티가 필수였는지 알 수 없습니다. [schema-evolution.md](schema-evolution.md) §8 의
  스냅샷이 이 축입니다. 감사 로그(`og_data.og_audit.schema_version`)에도 쓰기마다 스키마 버전이 남습니다.

질의는 **시각만** 지정합니다. Cypher 문장은 바뀌지 않습니다.

---

## 2. 켜기

```sql
SELECT og_enable_temporal('corp', 'Person');     -- 엔티티 타입
SELECT og_enable_temporal('corp', 'WORKS_AT');   -- 관계 타입
SELECT og_temporal_types('corp');                -- {Company,Person,WORKS_AT}
```

`og_enable_temporal(graph, type)` 은

- 그 타입과 **모든 하위 타입**(나중에 만들어지는 하위 타입 포함)에 적용됩니다.
  엔티티와 관계 타입만 가능합니다 (`only entity and relation types can be temporal`).
- 저장 테이블에 `__valid tstzrange` 와 `__sys_from timestamptz` 를 추가합니다.
  **기존 행은** `__valid = [-infinity, infinity)`, `__sys_from = 켠 시각` 이 됩니다. 켜기 전의
  일은 기록된 적이 없으므로, "작년에는 어땠나"에 오늘 값을 답하지 않도록 한 것입니다.
- 이력 테이블 `og_data.h_<type_id>` 를 만듭니다 (저장 테이블과 같은 컬럼 + `__sys_to`, `__op`).
- 트리거 `og_b_temporal`(함수 `og__temporal()`, SECURITY DEFINER)을 붙입니다. 트리거이므로
  **Cypher 뿐 아니라 raw SQL `UPDATE` 도 똑같이 버전이 남습니다.**
- 스탬핑(§6)도 함께 켭니다.
- 스키마 버전을 올립니다.

끄는 함수는 없습니다.

> 예전 단일 축 이력 기능(`og_enable_history` / `og_as_of`, spec 008)과는 별개입니다.
> 새로 쓰는 코드는 이 문서의 방식을 쓰세요.

---

## 3. 쓰기: `og.valid_from`, `og.valid_to`

쓰기는 평소와 같습니다. 다만 세션(또는 트랜잭션) 설정으로 **그 변경이 세상에서 언제부터
사실인지**를 말할 수 있습니다.

| 설정 | 의미 | 기본값 |
|---|---|---|
| `og.valid_from` | 이 쓰기가 효력을 갖는 시각 X | `now()` |
| `og.valid_to` | 유효 구간의 끝 | 새 행은 `infinity`, 갱신은 옛 행의 끝 |

`SET LOCAL` 로 트랜잭션 안에서만 쓰는 것을 권합니다. `SET` 으로 세션에 걸어 두면 이후 모든
쓰기에 적용됩니다.

```sql
BEGIN;
SET LOCAL og.valid_from = '2023-01-01';
SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}) SET p.salary = 200');
COMMIT;
```

트리거가 하는 일 (X = `og.valid_from` 또는 `now()`):

| 연산 | 동작 |
|---|---|
| INSERT | `__valid = [X, og.valid_to 또는 ∞)`, `__sys_from = now()` |
| UPDATE / DELETE | ① 옛 행을 이력에 보관: `__sys = [옛 __sys_from, now)`, `__op = 'u'`(갱신) 또는 `'d'`(삭제) |
| | ② X 가 옛 유효 구간 시작보다 뒤면, 옛 값이 여전히 사실인 앞부분 `[옛 시작, X)` 을 이력에 **다시 주장**: `__sys = [now, ∞)`, `__op = 'v'` |
| | ③ (UPDATE) 새 행: `__valid = [max(X, 옛 시작), og.valid_to 또는 옛 끝)`, `__sys_from = now()` |
| 보이는 값이 안 바뀌는 UPDATE | 이력 없음 (시스템 컬럼·마이그레이션 그림자 컬럼만 바뀐 경우) |

### 3.1 변경 vs 정정

| 하고 싶은 것 | `og.valid_from` | 결과 |
|---|---|---|
| 지금부터 바뀜 | (설정 안 함) | 옛 값은 `[시작, now)`, 새 값은 `[now, …)` |
| 과거 시점부터 바뀜 (소급) | `'2023-01-01'` | 옛 값은 `[시작, 2023-01-01)`, 새 값은 `[2023-01-01, …)` |
| **정정** — 원래부터 그 값이었다 | `'-infinity'` | 유효 구간은 그대로, 값만 교체. 옛 값은 system time 으로만 남는다 |

`'-infinity'` 는 "세상이 바뀐 게 아니라 우리가 아는 것이 바뀌었다"는 뜻입니다. valid time 으로
물으면 새 값만 보이고, 정정 이전 system time 으로 물어야 옛 값이 보입니다.

### 3.2 삭제

`DELETE` 도 X 를 따릅니다. `og.valid_from = '2024-06-01'` 로 관계를 지우면 그 관계는
`[시작, 2024-06-01)` 동안만 존재한 것으로 남습니다. `DETACH DELETE` 한 노드는 현재에서는
사라지지만, 삭제 이전의 `og.system_at` 으로 물으면 보입니다.

### 3.3 `OG_TEMPORAL_RANGE`

X 가 현재 행의 유효 구간 **끝 이후**면 거부됩니다 (SQLSTATE `22023`).

```
OG_TEMPORAL_RANGE: og.valid_from 2031-01-01 00:00:00+00 lies after the end of the current validity ["2020-01-01 …","2030-01-01 …")
```

### 3.4 쓰기 시 주의

- **쓰기는 항상 "현재 행"을 바꿉니다.** 과거 유효 구간(이력의 `'v'` 조각)을 직접 고치는 기능은
  없습니다. `og.valid_from` 을 현재 행의 시작보다 앞으로 주면 `max(X, 시작)` 이 되어 현재 구간만
  바뀝니다.
- **`og.valid_at`/`og.system_at` 을 설정한 채로 쓰지 마세요.** MATCH 는 과거 조각을 찾지만 SET 은
  같은 id 의 현재 행을 바꿉니다. 예를 들어 `og.valid_at = '2025-01-01'` 에서 본 값이 1 이라
  `SET p.sal = 9` 를 하면, 실제로는 2030 년부터 유효한 현재 행이 9 로 바뀝니다(실측). 현재 행의
  유효 구간이 이미 끝났다면 `OG_TEMPORAL_RANGE` 가 납니다.
- **미래 시점 변경.** `og.valid_from` 이 미래인 변경(예: 2030 년부터 인상)을 넣으면, 현재 행의
  유효 구간이 미래에서 시작하고, 그때까지의 값은 이력 테이블의 `'v'` 조각(`__sys_to = infinity`)
  에 남습니다. 이 조각은 지금도 믿고 있는 것이라 **현재의 일부**로 읽힙니다. 시각을 설정하지
  않은 질의는 그때까지 옛 값을, `og.valid_at` 을 그 이후로 두면 새 값을 봅니다(§4.3).
- 시각 문자열은 세션 `TimeZone` 으로 해석됩니다. `'2023-01-01'` 은 그 시간대의 자정입니다.

---

## 4. 읽기: `og.valid_at`, `og.system_at`

| 설정 | 묻는 것 | 설정 안 하면 |
|---|---|---|
| `og.valid_at` | 그 시각에 세상에서 사실이었던 것 (지금 아는 바로는) | `now()` |
| `og.system_at` | 그 시각에 데이터베이스가 믿고 있던 것 | `now()` |

둘 다 설정 안 하면 현재 상태입니다. 둘 다 설정하면 "그 시각에 믿었던, 그 시각의 사실"입니다.

### 4.1 SQL

```sql
SET og.valid_at = '2021-06-01';
SELECT og_cypher('corp', 'MATCH (p:Person {name: "ann"}) RETURN p.salary AS salary');
RESET og.valid_at;

-- 값이 변수에 있을 때 (psql \gset, 애플리케이션 파라미터 등)
SELECT set_config('og.system_at', '2026-09-30 01:55:00+00', false);   -- 세션
SELECT set_config('og.system_at', '2026-09-30 01:55:00+00', true);    -- 트랜잭션 한정
RESET og.system_at;

-- 트랜잭션 한정
BEGIN;
SET LOCAL og.valid_at = '2023-06-01';
SELECT og_cypher(...);
COMMIT;
```

현재 설정을 확인하는 함수: `og_valid_at()`, `og_system_at()`(설정이 없으면 `now()`),
`og_asof_active()`(둘 중 하나라도 설정되어 있으면 `true`).

### 4.2 Bolt (Neo4j 드라이버)

Bolt 게이트웨이는 `tx_metadata` 의 키를 트랜잭션 한정 설정으로 바꿉니다. 자동 커밋 `RUN` 과
명시적 트랜잭션(`BEGIN`) 모두 됩니다. 다음 트랜잭션으로 새지 않습니다.

| `tx_metadata` 키 | 설정 |
|---|---|
| `actor` | `og.actor` |
| `source` | `og.source` |
| `reason` | `og.reason` |
| `request_id`, `requestId` | `og.request_id` |
| `valid_at`, `validAt` | `og.valid_at` |
| `system_at`, `systemAt` | `og.system_at` |
| `valid_from`, `validFrom` | `og.valid_from` |
| `valid_to`, `validTo` | `og.valid_to` |

모르는 키는 무시됩니다. 문자열이 아닌 값은 텍스트로 바뀝니다.

```python
from neo4j import GraphDatabase, Query

driver = GraphDatabase.driver("bolt://127.0.0.1:7687", auth=(user, ""))
with driver.session(database="corp") as s:
    # 쓰기: 누가, 어디서, 언제부터
    s.run(Query("CREATE (:Person {name: 'ann', salary: 100})",
                metadata={"actor": "agent-9", "source": "crm", "validFrom": "2020-01-01"})).consume()

    # 명시적 트랜잭션
    tx = s.begin_transaction(metadata={"actor": "agent-10", "validFrom": "2023-01-01"})
    tx.run("MATCH (p:Person {name: 'ann'}) SET p.salary = 200").consume()
    tx.commit()

    # 읽기: 2021 년 6 월 기준
    q = "MATCH (p:Person {name: 'ann'}) RETURN p.salary AS s"
    s.run(Query(q, metadata={"validAt": "2021-06-01"})).single()["s"]      # 100
```

(이 흐름은 [`tests/bolt-context/run.py`](../../tests/bolt-context/run.py) 가 공식 드라이버로 검증합니다.)

### 4.3 가시성 규칙

생성 타입 뷰는 시간 버전 타입에 대해 현재 테이블과 이력 테이블을 세 분기로 합쳐 다음 조건으로
거릅니다.

| 분기 | 보이는 조건 |
|---|---|
| 현재 행 | `__valid @> og_valid_at() AND __sys_from <= og_system_at()` |
| 열린 이력 행 (`__sys_to = infinity` — 지금도 믿는 유효 구간 조각, `'v'`) | `__sys_to = 'infinity' AND __valid @> og_valid_at() AND __sys_from <= og_system_at()` |
| 닫힌 이력 행 (대체된 옛 믿음, `'u'`·`'d'`) | `og_asof_active() AND __sys_to <> 'infinity' AND __valid @> og_valid_at() AND __sys_from <= og_system_at() AND __sys_to > og_system_at()` |

- 컴파일된 SQL 은 시각과 무관하게 같습니다. **플랜 캐시에 영향이 없습니다.**
- 시각을 설정하지 않으면 닫힌 이력 분기는 한 번 평가되는 상수 필터로 걸러져 **스캔하지 않습니다.**
- 열린 이력 분기는 시각 설정과 무관하게 늘 읽힙니다. 이력 테이블의 부분 GiST 인덱스
  (`USING gist (__valid) WHERE __sys_to = 'infinity'`)로 탐색하며, 열린 행은 보통 소수입니다.
  그래서 시각을 설정하지 않은 질의도 시간 버전 타입마다 이 인덱스를 한 번 탐색합니다.
- 시간 버전 관계 타입의 한 홉은 인접 목록(현재만 앎) 대신 엣지 테이블(같은 세 분기)을 읽습니다.

---

## 5. 지원하는 질의 모양과 `OG_TEMPORAL_UNSUPPORTED`

시각이 설정되어 있는데 질의 모양이 현재 상태만 볼 수 있으면, 오늘 데이터로 조용히 답하는 대신
오류를 냅니다 (SQLSTATE `0A000`).

| 질의 모양 | 시각 설정 시 | 메시지의 what |
|---|---|---|
| 라벨 있는 노드 `(p:Person)` | 지원 | |
| 타입이 하나인 한 홉 `-[:WORKS_AT]->`, `-[w:WORKS_AT]->` | 지원 | |
| 변수 없는 무타입 한 홉 `-->` | 지원 (시간 버전 관계 타입의 엣지 테이블을 읽음) | |
| 라벨 없는 노드 `(n)` (그래프에 시간 버전 노드 타입이 있을 때) | **오류** | `an unlabelled node pattern` |
| 시간 버전 관계 타입의 가변 길이 `-[:WORKS_AT*1..2]->` | **오류** | `variable-length traversal over a temporal relationship type` |
| 타입이 하나가 아닌 관계 변수 `-[r]->`, `-[r:A\|B]->` | **오류** | `a relationship variable without a single type` |
| TypeQL (`og_typeql`) | **오류** | `TypeQL` |

```
OG_TEMPORAL_UNSUPPORTED: an unlabelled node pattern cannot answer a point-in-time query (og.valid_at / og.system_at is set)
HINT: Use typed patterns and single-hop relationships over temporal types, or RESET og.valid_at and og.system_at.
```

시각을 `RESET` 하면 같은 질의가 그대로 실행됩니다.

시간 버전이 아닌 타입은 시각과 무관하게 항상 현재 상태를 돌려줍니다. 시점 질의에 등장하는
타입은 모두 시간 버전으로 켜 두세요.

---

## 6. 누가, 어디서: 스탬핑과 `og_meta`

`og_enable_temporal` 은 스탬핑도 켭니다. 스탬핑만 따로 켜려면 `og_enable_stamping(graph, type)`.
트리거 `og_a_stamp`(`og__stamp()`)가 다음 컬럼을 **세션에서** 채웁니다. 행에 적힌 값은 무시되므로
raw SQL 로도 위조할 수 없습니다.

| 컬럼 | 값 |
|---|---|
| `__created_at`, `__created_by` | 최초 INSERT 의 `now()`, `current_user` |
| `__updated_at`, `__updated_by` | 마지막으로 보이는 값을 바꾼 쓰기의 `now()`, `current_user` |
| `__actor` | 그 쓰기의 `og.actor` (애플리케이션이 주장하는 행위자) |
| `__source` | 그 쓰기의 `og.source` |

`__actor`·`__source` 는 쓰기마다 새로 채워집니다. 마지막 쓰기에서 `og.source` 를 주지 않았다면
`source` 는 `null` 입니다.

`og_meta(id)` 는 한 요소의 시스템 컬럼을 앞의 `__` 를 뗀 이름으로 돌려줍니다.

```sql
SELECT og_meta((og_cypher('corp', 'MATCH (p:Person {name: "ann"}) RETURN id(p) AS i')->>'i')::int8);
```
```json
{"actor": "lee@payroll", "source": null, "created_by": "dev", "updated_by": "dev",
 "created_at": "…", "updated_at": "…", "valid": "[\"2023-01-01 00:00:00+00\",infinity)", "sys_from": "…"}
```

- 현재 행만 읽습니다. 삭제된 요소는 `NULL`. 과거 버전의 스탬프는 이력 테이블에 있습니다.
- `og.actor`, `og.source`, `og.reason`, `og.request_id` 는 감사 로그 `og_data.og_audit` 의
  `actor`, `source`, `reason`, `request_id` 컬럼에도 남습니다. `db_user`(실제 DB 역할),
  `schema_version`, `action` 도 함께. `og.reason` 은 감사 로그에만 남고 행 스탬프에는 없습니다.
- 이름이 `__` 로 시작하는 프로퍼티와 `_id`, `_type`, `_src`, `_dst` 는 예약어라 Cypher 로 쓸 수
  없습니다 (`OG_RESERVED_PROPERTY`, SQLSTATE `42501`).

---

## 7. 이력 테이블에 무엇이 있나

`og_data.h_<type_id>` 는 저장 테이블의 모든 컬럼 + `__sys_to`, `__op` 입니다.

| `__op` | 의미 |
|---|---|
| `u` | 갱신으로 대체된 옛 믿음. `[__sys_from, __sys_to)` 동안 믿었다 |
| `d` | 삭제로 대체된 옛 믿음 |
| `v` | 소급/시점 변경으로 잘린 앞부분 `[옛 시작, X)`. 지금도 믿고 있으므로 `__sys_to = infinity` |

튜토리얼의 예 (입사 → 2023 년부 인상 → 정정):

```
 salary | valid_from |        valid_to        | sys_from | sys_to | op
--------+------------+------------------------+----------+--------+----
    100 | 2020-01-01 | infinity               | W1       | W2     | u
    200 | 2023-01-01 | infinity               | W2       | W3     | u
    100 | 2020-01-01 | 2023-01-01 00:00:00+00 | W2       | ∞      | v
```

- 쓰기 역할은 이력 테이블에 DML 권한이 없습니다. 트리거가 SECURITY DEFINER 로 씁니다.
- 스키마 변경이 이력 테이블에도 반영됩니다: 새 컬럼 추가, 타입 변경(`alter_property_type` 은
  이력 행도 마이그레이션), **삭제된 컬럼은 이력 테이블에 남습니다**(옛 값이 들어 있으므로).
- 보존(retention) 정책은 없습니다. 이력은 계속 쌓입니다.

---

## 8. 제약과 아직 안 되는 것

- TypeQL 의 시점 질의 없음 (시각 설정 시 오류).
- 시간 버전 관계 타입 위의 가변 길이 시점 순회 없음 (오류).
- 라벨 없는 시점 스캔 없음 (오류).
- 과거 유효 구간 조각의 소급 편집 없음 — 쓰기는 현재 행에만 (§3.4).
- 시각을 설정한 채 쓰면 현재 행이 바뀜 — 거부하지 않음 (§3.4).
- 이력 보존 정책 없음. `og_enable_temporal` 을 끄는 함수 없음.

<!-- affects: docs/tutorials/temporal-tutorial.md, examples/tutorials/temporal.sql -->
<!-- requires-update: engine/src/temporal/mod.rs, engine/sql/access.sql, bolt/src/session.rs -->
