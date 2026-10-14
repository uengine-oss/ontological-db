# ADR-029: 데이터 패브릭을 Ontological 안에 둔다 — 가상 타입은 FDW 위의 매핑 뷰, 가상 관계는 키 조인

| 항목 | 값 |
|---|---|
| 상태 | **Proposed** (실험 브랜치 `exp/virtual-fabric`) |
| 날짜 | 2026-10-09 |
| 영향 범위 | interop, cypher, catalog, bolt(변경 없음) |
| 근거 | `experiments/fabric/` (설정 SQL, 벤치마크, `results.json`), `engine/tests/sql/13_virtual_fabric.sql` |

> **이 문서가 답하는 질문**
> - 원천 시스템에 있는 행을 **사이퍼 한 번으로** — 그래프에 저장된 노드와 섞어서 — 질의할 수 있는가?
> - 그 일을 Ontological이 직접 할 것인가(A), MindsDB에 맡길 것인가(B), 지금처럼 분리해 둘 것인가(C)?
> - 사이퍼로 접근 경로를 하나로 묶으면 MindsDB의 다중 소스 조인 SQL을 잃는가?

## 배경 — 지금 가상 노드가 동작하는 방식

ontologic의 Ontology Studio에서 가상 노드는 **클래스 단위 바인딩**이다. `class_datasource_bindings`(SQLite)에
datasource·table·key_columns·column_map을 두고, 그래프에는 인스턴스가 0개다. 질의 경로는 다음과 같다.

```
사이퍼 → 결과에 _datasource_refs {class, datasource, table, join_on:"from_col = to_col"(자유 텍스트)}
      → 에이전트가 스키마를 다시 조회하고 SQL을 직접 작성
      → text2sql(:8020) → MindsDB MySQL(:47335) → 원천 DB
```

**조인은 에이전트 머릿속에서 일어난다.** 그래프와 원천 사이의 관계가 구조화되어 있지 않고
(`join` 설명 문자열), 그래프 노드와 원천 행을 한 질의로 엮을 방법이 없다. PuppyGraph가 푸는 문제가 이것이다.
PuppyGraph는 정점·간선을 테이블 매핑으로 선언하고 Cypher/Gremlin을 원천 SQL로 컴파일한다. 다만 자체 저장소가
없어서 **물질화된 그래프와 가상 행을 섞지 못한다.** Ontological은 둘 다 할 수 있는 위치에 있다.

## 고려한 선택지

**A. Ontological 내장.** Ontological은 PostgreSQL 확장이고, 사이퍼는 SQL로 컴파일된다(ADR-009). 외부 테이블은
플래너에게 평범한 릴레이션이다. 그러니 가상 타입은 spec 005의 매핑 뷰(`og_map_table`)를 **FDW 외부 테이블** 위에
얹은 것이면 된다. 원천에 무엇을 보낼지(필터·정렬·같은 서버 안의 조인)는 FDW가 정하고, 보낼 수 없는 교차 소스
조인은 PostgreSQL 실행기가 맡는다.

**B. MindsDB 유지, Ontological이 위임.** 가상 타입은 MindsDB를 가리키고, 연합(federation)은 MindsDB가 맡는다.
Ontological은 원천 자격증명을 하나도 갖지 않는다.

**C. 분리 유지(현행).** 그래프는 그래프, 패브릭은 MindsDB. 조인은 클라이언트(에이전트)가 한다.

## 실험 — 무엇을 만들었나
아래 표는 함수 이름으로 적혀 있다. 지금은 같은 일을 문장으로 한다(바로 아래 "선언은 문장으로" 참고). 함수는 그 문장들의 구현체로 남아 있다.


| MindsDB / PuppyGraph | 여기 | 하는 일 |
|---|---|---|
| `CREATE DATABASE erp WITH ENGINE='postgres'` | `og_create_datasource(name, engine, params)` | FDW 서버와 사용자 매핑을 만든다. 비밀번호는 사용자 매핑에만 저장되고, 사용자 매핑은 superuser만 읽을 수 있다 |
| 테이블 하나 / 정점 매핑 | `og_map_virtual(graph, Type, ds, 'schema.table', key)` | `IMPORT FOREIGN SCHEMA`로 컬럼·타입을 읽고 프로퍼티를 선언한 뒤, 타입 저장소를 외부 테이블 위의 뷰로 바꾼다 |
| `CREATE VIEW … JOIN …` | `og_import_foreign` + `og_map_relation(graph, Type, 'schema.view', key)` | 임의 SQL 뷰를 타입으로 노출한다. 복합키 부여나 다중 소스 조인 뷰에 쓴다 |
| 간선 매핑(from/to 컬럼) | `og_create_virtual_rel(graph, R, A, a_prop, B, b_prop)` | `(a:A)-[:R]->(b:B)` ⇔ `a.a_prop = b.b_prop`. 양 끝은 가상이든 네이티브든 상관없다 |
| (복사본으로 전환) | `og_materialize_mapping(graph, Type)` (spec 005) | 가상 타입 하나를 저장된 테이블로 복사한다. 질의와 관계 선언은 그대로이고, 그때부터 쓸 수 있다 |
| `EXPLAIN` | `og_fabric_explain(graph, query, analyze)` | 사이퍼가 각 원천에 보낸 SQL, 넘어온 행 수, 로컬에서 한 조인을 단계별로 보여 준다 |

컴파일러 변경은 `join_rel` 앞의 분기 하나다. 가상 관계는 인접(`og_adj`) LATERAL 대신 **조인 술어**로 컴파일되고,
라벨이 없는 끝점에는 관계 정의에서 라벨을 추론한다. 관계 변수(`[r:R]`)는 `_id/_type/_src/_dst`를 갖춘 값으로
반환되므로, **Bolt 게이트웨이를 고치지 않아도** Neo4j 드라이버가 `Node`와 `Relationship`으로 받는다(실측).

### 선언은 문장으로 — 패브릭 문장과 SQL 게이트웨이

처음 구현의 표면은 `SELECT og_map_virtual('scm', 'Item', 'erp', 'erp.item', 'item_id')` 같은 함수 호출이었다.
위치 인자 다섯 개가 무엇인지 읽어서는 알 수 없고, MindsDB를 쓰던 사람에게는 낯설다. 그래서 선언을 문장으로 받는다.

```sql
CREATE DATASOURCE erp WITH ENGINE = 'postgres', PARAMETERS = {"host": …, "schema": "erp", …};
SHOW TABLES FROM erp;
CREATE VIRTUAL NODE Item FROM erp.item KEY item_id;
CREATE VIRTUAL RELATIONSHIP SHIPS FROM Shipment(ref_po_no) TO PurchaseOrder(po_id);
MATCH (sh:Shipment)-[:SHIPS]->(po:PurchaseOrder) RETURN …;      -- 사이퍼도 문장이다
```

이 문장들은 **사이퍼가 들어오는 곳이면 어디서나** 받는다. `og_cypher()` 가 패브릭 문장을 알아보고 번역기로
넘기므로, `og_cypher()` 를 부르는 Studio와 Bolt 게이트웨이는 고칠 것 없이 같은 문장을 받는다. 주 사용 경로는
Studio의 편집기이고, 튜토리얼도 그 경로만 쓴다.

- **설정은 사이퍼 맵, 비밀은 파라미터.** `PARAMETERS = {host: '…', password: $pw}`. JSON 표기도 받는다.
  Studio에서는 `:param pw => '…'` 로 넣는다(페이지 메모리에만 있고 질의 기록에 남지 않는다). 감사 기록에는
  설정 블록이 `{…}` 로 가려져 남고, Studio의 SQL 탭에도 `CREATE DATASOURCE` 의 설정은 나오지 않는다.
- **조인·집계는 정의에만.** `CREATE VIRTUAL NODE Score KEY k AS SELECT … FROM erp.a JOIN wms.b …`.
  임의 SQL을 질의 통로로 여는 대신, SQL을 노드 정의 안에 한 번만 두고 질의는 계속 사이퍼로 한다.
  정의는 `SELECT` 하나여야 하고 `;` 뒤에 무언가를 붙이면 거부된다.
- **원천에는 쓰지 않는다.** 데이터소스가 들여온 모든 외부 테이블에 쓰기를 거부하는 트리거를 건다. 사이퍼뿐
  아니라 SQL로 `DELETE FROM erp.supplier` 를 쳐도 막힌다. 원천 계정이 관리자 권한이어도 `INSERT`·`UPDATE`·
  `DELETE`·`TRUNCATE` 가 거부되고 데이터가 그대로임을 MySQL·PostgreSQL 양쪽에서 확인했다. 쓰기 가능한
  가상 노드(`WRITABLE`)는 다음 단계로 미뤘다. 여러 원천에 걸친 쓰기는 원자적이지 않기 때문이다.

Studio 쪽에서 이 과정에 드러난 것 셋을 고쳤다. 파라미터 입력이 없었다(`:param` 추가). 결과 컬럼 순서가
`RETURN` 순서가 아니라 키 정렬 순서였다(`og_cypher_columns` 를 쓰도록). 서버가 연결을 끊으면 — DB 재시작만으로도 —
연결 풀의 미처리 오류로 Studio 프로세스가 죽었다(오류 처리기 추가).

PostgreSQL 클라이언트(`psql`, DB 도구)를 위한 통로는 따로 있다.
PostgreSQL은 확장에 최상위 문법을 추가할 길을 주지 않으므로(ADR-008) 두 부분으로 나눴다.

| 부분 | 위치 | 하는 일 |
|---|---|---|
| 번역기 | 확장 (`engine/src/fabric/sqlish.rs`) | 패브릭 문장이나 사이퍼를 SQL 문자열로 바꾼다(`og_fabric_translate`). `og_fabric(text)` 는 바꿔서 실행까지 한다 |
| SQL 게이트웨이 | 별도 프로세스 (`sql-gateway/`) | PostgreSQL 프로토콜을 바이트 그대로 중계한다. 첫 단어가 패브릭 문장이나 사이퍼인 문장만 서버에 번역을 물어 그 답을 대신 보낸다 |

고려한 다른 길과 버린 이유:

- **프로토콜을 직접 구현하는 프록시**(MindsDB 방식). 인증·트랜잭션·prepared statement를 프록시가 다시
  구현해야 한다. 이 ADR의 "발견 — B가 표준 경로로는 성립하지 않는다"가 바로 그런 계층에서 난 고장이다.
  중계기는 모르는 것을 통과시키므로 그 종류의 고장이 날 자리가 없다. `COPY`, 커서, 확장 프로토콜, 트랜잭션
  안의 사이퍼 쓰기와 SQL `ROLLBACK` 까지 실측으로 확인했다.
- **문법을 게이트웨이에 두기.** 게이트웨이 없이는 문장을 쓸 수 없게 되고, 회귀 테스트가 닿지 않는다.
  번역기를 확장에 두었기 때문에 `14_fabric_statements.sql` 이 문법을 고정하고, Bolt 쪽에서도 같은 번역기를 부를 수 있다.
- **임의 SQL을 Bolt로 통과시키기**(`CALL sql('…')` 류). Studio가 SQL을 직접 쓰지 않는다는 계층 규칙의
  우회로가 되고, 조인 지식을 다시 질의하는 쪽에 돌려준다. 그래서 SQL은 노드 정의(`AS SELECT`)에만 둔다.

데이터소스는 이제 `schema`(PostgreSQL)나 `database`(MySQL)를 받아 그 테이블을 **데이터소스 이름의 스키마**로
가져온다. `og_ext_erp__erp.supplier` 가 아니라 `erp.supplier` 다. 그래서 가상 노드를 선언하기 전에도
`SELECT … FROM erp.supplier JOIN wms.shipment …` 가 평범한 SQL로 동작한다.

게이트웨이의 한계: 사이퍼 결과 컬럼이 모두 `text` 이고, 게이트웨이에 친 사이퍼에는 파라미터를 붙일 수 없고,
TLS를 받지 않는다. 그리고 서버가 모든 클라이언트를 게이트웨이의 주소로 보므로, `pg_hba.conf` 가 그 주소를
`trust` 로 두면 게이트웨이에 닿는 누구나 인증 없이 들어온다. 기본 수신 주소를 루프백으로 둔 이유다.


## 측정 — SCM 데모, 네 시스템, 복사 0건

ERP·통관은 PostgreSQL(robo-postgres), WMS·CRM은 MySQL(robo-mysql-sample)에 있다. 원천마다 키 이름이 다르다.
A는 사이퍼 한 번이고, M은 같은 질문을 손으로 쓴 연합 SQL로 MindsDB HTTP에 보낸 것이다(B의 최선 경우이자
C의 SQL 단계). 중앙값 7회이며, **두 결과가 같은지 먼저 확인한 뒤에만** 시간을 비교한다.

| # | 질문 | A 사이퍼/FDW | M MindsDB SQL | 결과 일치 | 원천에서 가져온 행(A) |
|---|---|---|---|---|---|
| Q1 | 단일 소스 필터 | 1.1 ms | 9.0 ms | ✓ | 4 |
| Q2 | 같은 소스 hop + 집계 (PG·PG) | 1.6 ms | 11.2 ms | ✓ | 5 — **조인 자체가 원천으로 내려감** |
| Q3 | 교차 소스 2 hop (MySQL→PG→PG) | 3.3 ms | 45.6 ms | ✓ | 121 |
| Q4 | 세 시스템 (통관→WMS→ERP), `IN` 필터 | 3.4 ms | 44.6 ms | ✓ | 133 |
| Q5 | 분기 패턴 CRM×ERP | 3.1 ms | 61.6 ms | ✓ | 45 — CRM 3테이블 조인이 MySQL로 내려감 |
| Q7 | 4 hop, 세 시스템, 방향 혼합 | 5.0 ms | 74.1 ms | ✓ | 334 |
| Q8 | 복합키 가상 뷰 (재고 × 품목) | 19.7 ms | 50.3 ms | ✓ | 180 |
| Q9 | Q8 + `LIMIT 20` | 17.8 ms | 56.4 ms | **✗ MindsDB 15행** | 180 |
| Q6 | 그래프 전용 지식 → 가상 행 | 4.2 ms | 11.7 ms (C 2단계) | ✓ | 48 |

Q6는 MindsDB 혼자서는 답할 수 없는 질문이다. `RiskEvent`·`Region`은 그래프에만 있다. 현행 흐름(C)은 그래프를
먼저 조회하고, 받은 키로 SQL을 만들어 다시 보낸다. A는 이 일을 패턴 하나로 끝낸다.

`MATCH (:Region {name:'East Asia'})<-[:IN_REGION]-(e:RiskEvent)-[:AFFECTS]->(s:Supplier)<-[:SHIPPED_BY]-(sh:Shipment)`

**시간 수치를 읽는 법.** 데이터가 작고(최대 6.6천 행) 모두 로컬 도커에서 돌았다. MindsDB 수치에는 HTTP와
MindsDB 자체의 파싱·계획 비용이 들어 있다. 배율 자체는 규모에서의 증거가 아니다. 이 표가 증거로 삼는 것은
두 가지다. **답이 같다는 것**, 그리고 **원천에 무엇이 내려갔는가**(`EXPLAIN VERBOSE`의 Remote SQL)이다.

## 발견 — B가 표준 경로로는 성립하지 않는다

MindsDB의 MySQL 엔드포인트를 `mysql_fdw`로 가리키는 것이 B의 가장 싼 구현이었다. 이 시도는 세 곳에서 막혔다.

1. **`IMPORT FOREIGN SCHEMA` 실패.** MindsDB의 `information_schema` 질의가 *무관한* 다른 데이터소스의
   뷰 오류로 깨진다. 데이터소스 하나가 고장 나면 카탈로그 전체가 막힌다.
2. **prepared statement 메타데이터를 지원하지 않는다.** 가장 단순한 `SELECT` 조차
   `failed to retrieve query result set metadata`로 실패한다.
3. **방언 이중 번역.** mysql_fdw는 MySQL 방언(`<=>`, `ORDER BY x IS NULL`, 괄호로 중첩한 조인)으로 내리고,
   MindsDB는 그것을 PostgreSQL 백엔드에 다시 번역하지 못한다
   (`operator does not exist: character varying <=> unknown`, `Syntax error`).

B를 제대로 하려면 MindsDB 전용 클라이언트를 Rust로 짜야 한다. 그리고 **푸시다운 규칙을 컴파일러 안에 다시
구현해야 한다.** FDW가 이미 하는 일을, 번역 단계를 하나 더 거치면서 다시 하는 셈이다.

## 발견 — MindsDB 연합 조인의 LIMIT 오답

조인 결과에 교차 소스 술어(`stock.on_hand_qty < item.safety_stock`)가 걸린 상태에서 `LIMIT n`을 주면,
MindsDB가 돌려주는 행이 **n보다 적다.** 오류는 나지 않는다.

| LIMIT | 1 | 5 | 50 | 127 | 200 |
|---|---|---|---|---|---|
| MindsDB 반환 행 | **0** | **3** | **41** | **112** | 127 |
| A (사이퍼) | 1 | 5 | 50 | 127 | 127 |

LIMIT이 소스별 스캔에 먼저 적용되고 조인과 필터가 그 뒤에 오는 것으로 보인다. 에이전트가 "상위 N건"을
물을 때 조용히 틀린 답이 나온다. 현행 C 경로가 바로 이 경로다.

## 발견 — 네이티브 그래프에도 해당하는 결함 네 가지

패브릭 때문에 드러났지만, 저장된 그래프에도 똑같이 적용되는 문제다.

- **`=`가 `IS NOT DISTINCT FROM`으로 컴파일되었다.** 원천 DB는 이 형태에 B-tree를 쓰지 못한다. MySQL로는 `<=>`가
  되어 방언 문제를 일으킨다. 의미도 Cypher와 다르다. Cypher에서 `null = 'x'`는 `false`가 아니라 `null`이다.
  → 한쪽이 상수나 파라미터이면 평범한 `=`/`<>`로 컴파일한다.
- **`x IN [리터럴…]`이 jsonb 포함(`@>`)으로 컴파일되었다.** 인덱스를 못 쓰고 원천으로도 내려가지 않는다.
  Q4에서는 통관 테이블 전체가 넘어왔다. → 타입이 알려진 컬럼과 리터럴 목록이면 SQL `IN (…)`으로 컴파일한다.
  고친 뒤에는 `status = ANY('{HOLD,REJECTED}')`가 원천으로 내려간다.

- **`OPTIONAL MATCH`에서 매칭되지 않은 노드가 `null`이 아니라 `{}`였다.** 그래서 `count(n)`이 0이 아니라 1을 셌고,
  "짝이 없는 것 찾기"(`WITH a, count(b) AS c WHERE c = 0`)가 항상 빈 결과를 냈다. 튜토리얼의 안티 조인 예제를
  실행하다 발견했다. → 노드·관계 직렬화가 별칭의 `id`가 NULL이면 NULL을 낸다.
- **`og_cypher_explain`의 `plan`이 항상 `null`이었다.** `EXPLAIN (FORMAT JSON)`은 `json`을 돌려주는데 `jsonb`로
  읽었고, 실패가 `.ok()`에 묻혔다. → `json`으로 읽는다.

기존 회귀 6개 파일과 새 `13_virtual_fabric.sql`(가상 타입·관계), `14_fabric_statements.sql`(패브릭 문장)이 모두 통과한다.

## 결정 (제안)

**A.** 데이터 패브릭의 *질의 경로*는 Ontological 안에 둔다. 가상 타입과 가상 관계가 시맨틱 레이어이고,
사용자와 에이전트는 사이퍼 하나로 물질화된 노드와 원천 행을 함께 다룬다.

### "사이퍼로 통일하면 다중 소스 조인 SQL을 잃는가" — 잃지 않는다

- 다중 소스 조인은 **가상 관계로 한 번 선언된다.** 그 뒤로는 모든 hop이 그 조인이다. MindsDB에서는 사용자가
  매번 조인을 쓰고, 여기서는 온톨로지가 조인을 기억한다.
- 관계로 표현하기 어려운 조인(복합키, 집계 뷰, 여러 소스를 미리 묶은 뷰)은 `og_map_relation`으로 **SQL 뷰를
  타입으로** 만든다. MindsDB의 `CREATE VIEW`와 같은 역할이고, 차이는 뷰 위의 필터도 원천까지 내려간다는 점이다.
- 여기는 PostgreSQL이므로 **외부 테이블에 직접 SQL을 쓰는 길도 그대로 열려 있다.**

### MindsDB의 자리

질의 경로에서는 빠진다. MindsDB만 가진 것이 필요하면 원천 커넥터 쪽에서 해결한다. 대상은 ML 모델,
지식베이스, SaaS·API 커넥터이고, 순서는 다음과 같다.

1. PGDG가 패키징한 FDW: `oracle_fdw`, `tds_fdw`(MSSQL), `mongo_fdw` 등. 이번 실험에서 쓴 `mysql_fdw`도 PGDG 패키지다.
2. **Supabase Wrappers.** pgrx 기반의 Rust FDW 프레임워크로, BigQuery·ClickHouse·S3·Snowflake·Stripe 등을
   지원한다. "Rust로 동작해야 한다"는 요구와 같은 툴체인(pgrx)이다.
3. MindsDB 고유 기능(예측 모델 등)이 필요한 경우에만, MindsDB를 **원천 하나**로 붙이는 전용 래퍼를 둔다.

## 남은 문제 (정직하게)

| 문제 | 현재 | 다음 |
|---|---|---|
| 식별자 | 문자열 키를 36비트로 해시한다. 100만 행에서 충돌 확률은 약 1/14만이다. `id(n)` 조건은 원천으로 내려가지 않는다 | 키 프로퍼티를 정식 식별자로 노출(`elementId` = `Type:key`)하고, `id(n)=` 조건을 키 조건으로 역변환한다 |
| 미라벨 스캔 | `MATCH (n)`에 가상 노드가 나오지 않는다(`og_node` 레지스트리 밖). 가상 hop의 끝점은 라벨을 추론하므로 문제가 없다 | 의도된 동작으로 문서화하거나, 가상 타입만 UNION한다 |
| 가변 길이 | 가상 관계의 `*1..n`을 명시적으로 거부한다 | 재귀 CTE로 확장한다(자기참조 FK, 예: BOM) |
| 쓰기 | 가상 타입은 읽기 전용이고, 데이터소스 테이블은 SQL 경로에서도 쓰기가 거부된다(트리거). `CREATE`·`SET`·`REMOVE`·`DELETE`를 엔진이 "virtual type and read-only" 오류로 거부한다. 처음 구현에서는 `DELETE`가 원천까지 내려갔고, 원천 계정이 읽기 전용이라 막혔을 뿐이었다. 지금은 원천 계정의 권한과 무관하게 막는다(회귀 테스트로 고정). 가상 노드에 **저장된 간선**을 붙이는 것은 허용된다 | 쓰기 가능한 매핑은 spec 005 T012와 함께 결정한다. TypeQL의 `insert`·`delete`에도 같은 가드를 넣었으나, TypeQL 경로는 회귀 테스트로 고정하지 않았다 |
| 큰 × 큰 교차 소스 조인 | 양쪽을 PG로 가져와 해시 조인한다(Q7: 334행) | `use_remote_estimate` 기반 parameterized foreign scan(세미조인 푸시다운)을 측정한다 |
| 바이템포럴 | 가상 타입에 valid time이 없다. 미검증 | `og.valid_at`이 걸린 질의에서 가상 타입을 거부할지, 현재값으로 답할지 정한다 |
| 권한 | 뷰는 `security_invoker`. 원천 권한은 사용자 매핑의 계정을 따른다 | 역할별 사용자 매핑(`FOR role`)으로 원천 권한을 위임한다 |
| Studio 연동 | Ontological의 Studio(포탈)에서는 문장으로 선언·질의·진단이 된다. ontologic의 Ontology Studio는 아직 자기 SQLite 바인딩을 쓴다. 포탈의 스키마 패널은 가상 노드를 인스턴스 0개로만 보여 준다 | ontologic이 publish 때 `class_datasource_bindings`를 `CREATE VIRTUAL NODE`/`RELATIONSHIP` 문장으로 Bolt에 보내고, 관계의 `join` 자유 텍스트를 구조화된 from/to로 저장한다. 그러면 `_datasource_refs`와 에이전트 SQL 단계가 사라진다. 포탈 스키마 패널에 가상 표시를 넣는다 |

## 재현

- 따라 하기: [`docs/tutorials/virtual-fabric-tutorial.md`](../tutorials/virtual-fabric-tutorial.md).
  작은 원천 두 대(PostgreSQL·MySQL)로 12단계를 밟는다. 문서의 출력은 빌드 스크립트가 실제로 실행해 채운다.
- 측정: `experiments/fabric/README.md`.
