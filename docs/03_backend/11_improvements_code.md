# 코드 개선 포인트

> ⚠️ **이 문서는 감사 커밋 `7d60c82` 의 스냅샷이다.** 조용히 틀린 답을 내던 다섯
> 항목(ARCH-01, ARCH-02/CODE-01, CODE-33, CODE-34, PERF-20)은 이후 수정되었다.
> 현재 상태는 [`03_backend/12_fixed_correctness.md`](12_fixed_correctness.md) 를 볼 것.


> **이 문서가 답하는 질문**
> - 지금 코드에서 무엇이 문제이고, 얼마나 심각한가?
> - 각 문제의 근거는 어느 파일 몇 줄인가?
> - 무엇부터 고쳐야 하는가?

**모든 항목은 실제 코드를 읽고 근거를 특정한 것이다.** 일반론은 없다.
"미확인"이라고 적힌 것은 정적 읽기만으로 확정할 수 없어 실행 확인이 필요한 항목이다.

조사 규모:

```
$ grep -rn "unwrap()\|expect(\|panic!" engine/src --include=*.rs | wc -l
202
$ grep -rn "#\[cfg(test)\]" engine/src bolt/src --include=*.rs | wc -l
9        (엔진 8 + bolt 1, #[test] 함수 총 35개)
$ grep -rn "let _ = Spi::run" engine/src --include=*.rs | wc -l
4
$ grep -rn "Spi::run.*\.ok();" engine/src --include=*.rs | wc -l
26
$ 1,000줄 초과 파일: cypher/compile.rs(1591), cypher/parser.rs(1177), typeql/parser.rs(1108)
```

---

## 우선순위 요약 (상위 8건)

| 순위 | ID | 제목 | 왜 먼저인가 |
|---|---|---|---|
| 1 | `CODE-01` | 플랜 캐시가 스키마 변경에 무효화되지 않음 | 같은 세션에서 **질의가 실패**한다 |
| 2 | `CODE-33` | 쓰기 질의의 `WITH` 절이 조용히 무시됨 | `MATCH … WITH n LIMIT 1 DELETE n`이 **전부 삭제**한다 |
| 3 | `CODE-34` | 쓰기 경로 `count(DISTINCT)`가 `dedup()`만 함 | **답이 틀린다** |
| 4 | `CODE-06` | `int8` 프로퍼티에 실수를 쓰면 `text`로 확장 | 숫자 비교와 인덱스가 **조용히** 사라진다 |
| 5 | `CODE-02` | `stable` 함수가 DDL을 실행 | 읽기 전용 트랜잭션/스탠바이에서 실패 |
| 6 | `CODE-07` | 타입 확장 DDL이 `let _ =`로 결과를 버림 | 카탈로그와 실제 컬럼 타입이 **어긋난다** |
| 7 | `CODE-29` | `compile.rs`(1,591줄) / `session.rs`(606줄)에 단위 테스트 0 | 위 버그들이 잡히지 않은 이유 |
| 8 | `CODE-08` | TypeQL 쓰기가 속성 값을 SQL 리터럴로 조립 | FR-026 규칙에서 유일하게 벗어난 경로 |

---

## 1. High

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| CODE-01 | 플랜 캐시가 스키마 변경에 무효화되지 않음 | **High** · **fixed** | `engine/src/cypher/mod.rs:26-31,47-67`; `engine/src/catalog/labeling.rs:172-182`; `engine/src/cypher/views.rs:91-97` | 캐시 키가 `(graph, query)`뿐이다. 컴파일 산출물은 타입 id·생성 뷰 이름(`og_data.v_5`)을 담고 있고, 스키마가 바뀌면 `bump_schema_version`이 `drop_all_views()`로 뷰를 전부 지운다. 같은 백엔드에서 캐시된 SQL을 재실행하면 없는 뷰를 참조한다 | 캐시 키에 그래프의 `max(schema_version)`을 포함하거나, `bump_schema_version`에서 `PLAN_CACHE.clear()`를 호출 | 세션 중 스키마가 바뀌어도 질의가 계속 동작 | 버전 조회가 질의마다 1회 SPI. 캐시 히트 경로가 무료가 아니게 됨 → `schema_version_seq`의 `currval`을 백엔드-로컬로 캐싱해 완화 |
| CODE-33 | 쓰기 질의에서 `WITH` / `CALL` 절이 조용히 무시됨 | **High** · **fixed** | `engine/src/cypher/mod.rs:171-176, 239-243, 258-319` | 읽기 부분은 `take_while(Match \| Unwind)`로만 모으고, 나머지는 per-row 루프의 `_ => {}`(라인 318)에 떨어진다. 따라서 `MATCH (n) WITH n LIMIT 1 DELETE n`은 `WITH`를 무시하고 매치된 **전부**를 지운다 | `run_write` 진입 시 지원하지 않는 절 조합을 명시적으로 거절(`error!`)하거나, `WITH`를 읽기 부분에 포함시켜 컴파일 | 데이터 손실 위험 제거 | 기존에 (잘못) 동작하던 질의가 오류가 됨 — 그것이 올바른 동작 |
| CODE-34 | 쓰기 경로 `count(DISTINCT x)`가 `Vec::dedup()`만 사용 | **High** · **fixed** | `engine/src/cypher/mod.rs:346-355` | `values.dedup()`은 **연속된** 중복만 제거한다. 정렬하지 않으므로 `[a,b,a]`에서 3을 센다. `CREATE … RETURN count(DISTINCT x)`가 틀린 수를 돌려준다 | 정렬 후 `dedup()` 하거나 `HashSet`/`BTreeSet`으로 교체 (`cmp_json`이 이미 있다) | 쓰기 질의의 DISTINCT 집계가 정확해짐 | JSON 값 전순서 정의 필요 — `cmp_json`(`mod.rs:386-391`)을 재사용 |
| CODE-06 | 정수 프로퍼티에 실수를 쓰면 `float8`이 아니라 `text`로 확장 | **High** | `engine/src/storage/mod.rs:53-60, 62-73, 127-153` | `infer_column_type(1.5) = "float8"`, 기존 컬럼은 `int8`. `type_accepts("int8","float8")`은 `false`이고 `int8`은 `WIDENABLE`이므로 **`text`로 확장**된다. `SET n.score = 1` 다음 `SET n.score = 1.5`면 그 컬럼의 숫자 비교·범위 인덱스가 사라진다 | `type_accepts`에 `("int8","float8")` 승격 경로를 추가하고, `declare_new_props`가 `text` 대신 `float8`로 `ALTER`하도록 분기 | 흔한 패턴에서 인덱스와 타입이 보존됨 | `ALTER COLUMN TYPE float8`도 전체 재작성이다. 정밀도 손실은 없음(int8→float8은 2^53 초과에서 손실 가능) |
| CODE-02 | `#[pg_extern(stable)]` 함수가 DDL을 실행 | **High** | `engine/src/cypher/mod.rs:74-80` → `engine/src/cypher/views.rs:135`; `engine/src/typeql/mod.rs:82-96` → `engine/src/typeql/schema.rs:526-552` | `og_cypher_sql`은 `views::ensure_view`로 `CREATE OR REPLACE VIEW`를, `og_typeql_sql`은 `ensure_has_type`로 `INSERT` + `CREATE TABLE`을 실행한다. `STABLE`은 "데이터베이스를 수정하지 않는다"는 계약이다 | 두 함수를 `volatile`로 바꾸거나, 컴파일 경로에서 뷰 생성을 분리해 `ensure_view`가 필요할 때 `error!`로 안내 | 읽기 전용 트랜잭션·스탠바이·`og_apply_role(read_only)`에서 동작 | `volatile`로 바꾸면 플래너가 이 함수를 상수 접기하지 않음 — 진단용 함수라 영향 미미 |
| CODE-07 | 타입 확장 DDL이 `let _ =`로 결과를 버림 | **High** | `engine/src/storage/mod.rs:138-140, 147-151` | `ALTER TABLE … ALTER COLUMN … TYPE text`가 실패해도 바로 다음 `UPDATE og_catalog.property SET data_type='text'`가 실행된다. 두 문장 모두 `let _ =`다. 실패하면 **카탈로그는 text, 컬럼은 int8**이 되고, 이후 `plan_props`가 `($2->>'k')::text`를 int8 컬럼에 넣으려 한다 | `unwrap_or_else(|e| error!("failed to widen '{key}' on {table}: {e}"))`로 바꾸고, 카탈로그 갱신을 ALTER 성공 뒤로 이동 | 카탈로그/물리 스키마 불일치 제거 | 이전에 조용히 넘어가던 케이스가 오류가 됨 — 그것이 올바른 동작 |
| CODE-08 | TypeQL 쓰기가 속성 값을 SQL 리터럴로 조립 | **High** | `engine/src/typeql/write.rs:649-674`(`typed_literal`), 사용처 `write.rs:242, 257, 321, 345, 513-519` | 값이 `lit_str`(`typeql/compile.rs:616-618`)로 이스케이프되어 SQL 텍스트에 들어간다. Cypher 경로가 지키는 "바인딩 파라미터 하나"(spec 003 FR-026)와 다르다. 이스케이프 구현에 결함이 생기면 즉시 주입 경로가 된다 | `Spi::run_with_args` / `client.select(..., &[...])`의 바인딩 파라미터로 전환. 값 타입별 `DatumWithOid` 매핑 필요 | FR-026 규칙이 전 코드에 균일해짐. 플랜 재사용도 개선 | 값 타입이 런타임에 결정되므로 파라미터 OID를 동적으로 골라야 함. `find_one`(`write.rs:124-173`)도 함께 고쳐야 함 |
| CODE-25 | SPI 결과의 `unwrap()`/`expect()` 202건 중 사용자 도달 가능한 것 | **High** | 전수: `grep -rn "unwrap()\|expect(\|panic!" engine/src --include=*.rs \| wc -l` → 202. 위험 사례: `engine/src/catalog/labeling.rs:44,59-60,207,227`; `engine/src/storage/mod.rs:31-32,168,171-174,188-194,512`; `engine/src/cypher/views.rs:44,46-48,74,152`; `engine/src/catalog/types.rs:316,400,472-476` | `row.get(1).unwrap().unwrap()` 형태의 이중 언랩이 다수다. SPI 오류와 NULL을 구분하지 않고 둘 다 `called Option::unwrap() on a None value`를 낸다. 사용자는 무엇이 잘못됐는지 알 수 없다 | `catalog/types.rs:112-119` 패턴을 표준으로 삼는다: `Result`는 `.expect("<내부 불변식>")`, `Option`은 `.unwrap_or_else(\|\| error!("<사용자 설명>"))`. 우선 `NOT NULL` 보장이 없는 컬럼 읽기부터 | 오류 메시지가 진단 가능해짐 | 건수가 많아 한 번에 하면 diff가 커진다. 파일 단위로 나눠 진행 |

---

## 2. Medium

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| CODE-29 | 핵심 파일에 단위 테스트가 0 | Med | `#[cfg(test)]` 있는 파일 9개(`engine/src/{cypher/lexer.rs:266, cypher/parser.rs:1133, id.rs:93, typeql/lexer.rs:279, typeql/parser.rs:964,1058, typeql/compile.rs:776, adapters/rdf.rs:850}`, `bolt/src/packstream.rs:285`). **`cypher/compile.rs`(1,591줄)와 `bolt/src/session.rs`(606줄)에는 없다** | 두 파일 모두 사용자 입력을 직접 다루는데 테스트가 없다. `CODE-33`/`CODE-34`/`CODE-15`가 잡히지 않은 이유 | 의존성 없는 순수 함수부터 시작: `compile.rs`의 `blind_expr` `multiplicity_blind` `mentions_alias` `quote_ident` `sql_str`, `session.rs`의 `speaks` `split_plan_prefix` `to_bolt` `to_json` `Failure::from_pg` | DB 없이 도는 회귀망 확보 | 없음. 이 함수들은 `Compiler`/`Session` 인스턴스가 필요 없다 |
| CODE-05 | 쓰기 경로가 무거운 DDL을 유발 | Med | `engine/src/storage/mod.rs:180-200` → `engine/src/catalog/types.rs:539-599`; `engine/src/catalog/labeling.rs:117-182` | 평범한 `CREATE (:Person {age:30})` 하나가 `ALTER TABLE ADD COLUMN`(모든 서브타입), `UPDATE <table> … WHERE __ext ? 'age'`(전체 테이블), `DROP VIEW … CASCADE`(모든 타입 뷰), `relabel_graph`(그래프 전체 라벨 재작성 + 타입 수만큼 SPI INSERT)를 부를 수 있다 | ① `relabel_graph`의 라벨 INSERT를 단일 다중행 문장으로 (`labeling.rs:160-167`) ② `drop_all_views`를 스키마 변경 대상 타입의 조상/자손 뷰로 한정 ③ 문서에 "대량 적재 전 스키마 선언" 명시 | 초기 적재 지연·락 경합 감소 | ②는 자손 관계 계산이 필요. 놓치면 낡은 뷰가 남아 `CODE-01`과 결합해 오답 |
| CODE-11 | Bolt 오류 코드가 영어 메시지 부분 문자열 매칭 | Med | `bolt/src/session.rs:578-593` | 엔진에 오류 분류가 없어(`Result<_, String>`), 게이트웨이가 `message.contains("not supported")` 등으로 Neo4j 코드를 복원한다. 엔진 문구를 바꾸면 코드가 조용히 바뀐다. `"unknown label"` 조건은 **현재 코드에 그런 문구가 없어 이미 죽어 있다**(실제는 `"label '…' does not exist"` notice와 `"type '…' does not exist"`) | 엔진에 오류 종류 열거형을 도입하고 SQLSTATE로 실어 보낸다. 최소 단기 조치로 죽은 조건 제거 + 매칭 목록을 상수로 분리하고 테스트 추가 | Bolt 오류 코드가 드라이버 기대와 맞음 | 오류 타입 도입은 광범위 리팩터(`CODE-32`)와 묶어야 함 |
| CODE-32 | 오류 타입 부재 — `Result<_, String>` 7종 별칭 | Med | `engine/src/cypher/compile.rs:149`, `cypher/parser.rs:20`, `typeql/compile.rs:21`, `typeql/mod.rs:27`, `typeql/parser.rs:17`, `typeql/schema.rs:25`, `typeql/write.rs:18` | 오류 코드·분류·원인 사슬이 없다. SQLSTATE가 전부 `XX000`. 컨텍스트는 `format!("{ctx}: {e}")`로 손수 붙인다(`typeql/write.rs:155` 등, 규약 아님) | `enum OgError { Parse{..}, UnknownType{..}, Unsupported{..}, Internal{..} }`를 도입하고 `Display`로 현재 메시지를 그대로 보존. `error!` 호출부에서 SQLSTATE 매핑 | Bolt/Studio/에이전트가 코드로 분기 가능. `CODE-11` 해소 | 전 모듈 시그니처 변경. 메시지 문구는 반드시 그대로 유지해야 `engine/tests/sql/*.sql`과 `session.rs` 매칭이 안 깨짐 |
| CODE-23 | 인접 세그먼트 append의 기본키 경쟁 | Med | `engine/src/storage/adjacency.rs:19-44` | 꼬리 세그먼트가 없거나 꽉 찼을 때, 두 트랜잭션이 모두 UPDATE에서 0행을 받고 같은 `seq`를 계산해 INSERT한다 → `PRIMARY KEY (src, etype, dir, seq)` 위반. 같은 노드에 동시 엣지 추가가 **간헐적으로 실패**한다 | INSERT에 `ON CONFLICT (src, etype, dir, seq) DO UPDATE SET nbr = og_adj.nbr \|\| EXCLUDED.nbr, eid = og_adj.eid \|\| EXCLUDED.eid, n = og_adj.n + 1`을 추가하거나, 실패 시 append를 1회 재시도 | 동시 쓰기에서 간헐 실패 제거 | `ON CONFLICT DO UPDATE`는 `n < CHUNK` 상한을 넘길 수 있다 — `WHERE og_adj.n < 256` 조건 필요 |
| CODE-24 | `og_node_json`/`og_edge_json`이 `PARALLEL UNSAFE` | Med | `engine/sql/access.sql:208-235, 237-264`; 사용처 `engine/src/cypher/compile.rs:991, 1013, 1087, 1111` | `LANGUAGE plpgsql`이고 `PARALLEL` 지정이 없어 기본 UNSAFE다. 컴파일러가 타입 미상 프로퍼티/변수 접근에 이 함수를 쓰므로, `MATCH (n) WHERE n.x = 1`류 질의는 **병렬 계획을 아예 못 받는다** | 함수 본문이 `EXECUTE`만 쓰고 데이터를 바꾸지 않으므로 `PARALLEL SAFE` 선언 가능성 검토(동적 SQL이라 안전성 확인 필요). 또는 라벨 없는 패턴에 경고 note를 남긴다(`Compiler.notes`) | 무라벨 질의에서 병렬 스캔 가능 | `EXECUTE format(...)`가 있는 함수를 PARALLEL SAFE로 선언하는 것은 신중해야 함. **안전성 미확인** |
| CODE-15 | `move_join_to_end`가 `optional`을 검사하지 않음 | Med | `engine/src/cypher/compile.rs:671-678` (주석 674-675: "Only matters under OPTIONAL MATCH") | 조건이 `node_join_added && mark > 0 && self.from.len() > mark + 1`뿐이다. 일반 MATCH에서도 노드 조인이 홉 뒤로 이동한다. 의미론은 같지만(모두 `CROSS JOIN`) **README.md:235-243의 문서화된 산출물과 FROM 순서가 달라진다.** 실제 출력은 **미확인** — `og_cypher_sql`로 확인 필요 | 조건에 `optional &&`를 추가하거나, README 예제를 현재 출력으로 갱신. 어느 쪽이든 `compile.rs`에 스냅샷 테스트를 추가 | 문서와 코드의 일치 | 조건을 바꾸면 OPTIONAL MATCH 술어 배치가 달라질 수 있음 — `04_neo4j_compat.sql`로 회귀 확인 필요 |
| CODE-16 | `RETURN *`의 컬럼 순서가 비결정적 | Med | `engine/src/cypher/compile.rs:484-490`(`self.binds.keys()`); 대조: `compile.rs:302-306` `bound_vars()`는 **정렬한다** | `binds`는 `HashMap`이라 순회 순서가 프로세스마다 다르다. `RETURN *`의 SELECT 컬럼 순서가 백엔드마다 달라진다. `og_cypher_columns`는 `RETURN *`에 빈 배열을 돌려주므로(`cypher/mod.rs:731-733`) Bolt는 첫 행 jsonb 키 순서(정렬됨)로 폴백한다 — **직접 SQL과 Bolt가 서로 다른 순서를 본다** | `build_core`에서도 `bound_vars()`를 쓰거나 `keys()` 결과를 정렬 | 결과 순서가 결정적이 됨 | 기존에 우연히 특정 순서에 의존하던 클라이언트가 영향받음 |
| CODE-17 | `delete_instance`의 무제한 재귀 | Med | `engine/src/typeql/write.rs:552-604` (재귀 호출 593-595) | 인스턴스가 롤을 맡은 관계를 재귀적으로 삭제한다. 방문 집합도 깊이 제한도 없다. 관계가 관계의 플레이어인 순환 구조에서 무한 재귀 → 스택 오버플로 가능. **실제 도달 가능성 미확인**(순환 구조를 만들 수 있는지 스키마 제약 확인 필요) | 방문한 id의 `HashSet`을 인자로 전달하거나 반복문 + 워크리스트로 전환 | 순환 구조에서도 종료 보장 | 없음 |
| CODE-13 | 라벨 대안 `(:A\|B)`가 합집합이 아니라 교집합으로 해석됨 | Med | `engine/src/cypher/parser.rs:647-653`(`\|`를 `labels` 벡터에 평탄화) → `engine/src/catalog/types.rs:152-189` `resolve_label_set`(가장 구체적인 하나를 고르고, 없으면 `LabelMatch::Nothing`) | Cypher의 `(:A\|B)`는 "A 또는 B"인데, 여기서는 `(:A:B)`와 같은 경로를 타서 "A이면서 B"로 해석된다. 무관한 두 라벨이면 조용히 **빈 결과**가 된다 | 파서에서 `\|` 구분을 보존(`labels: Vec<Vec<String>>` 또는 별도 필드)하고, 컴파일러가 `og_data.og_node`에 `type_id = ANY(union of subtypes)` 술어를 붙이도록 | Neo4j 의미론과 일치 | AST 변경 + `bind_node`(`compile.rs:717-801`) 분기 추가. 관계 타입 `[:A\|B]`는 이미 합집합으로 올바르게 처리됨(`compile.rs:833-841`) — 비대칭 해소 |
| CODE-18 | `og_typeql`이 `_params`를 조용히 무시 | Med | `engine/src/typeql/mod.rs:49-53`(`_params: default!(JsonB, "'{}'")`) | 시그니처에 파라미터를 받는다고 되어 있으나 본문에서 쓰지 않는다. 호출자는 파라미터가 적용됐다고 믿는다 | ① 파라미터를 구현하거나 ② 비어 있지 않은 값이 오면 `error!("parameters are not supported by og_typeql yet")` | API가 정직해짐 | ②는 기존 호출자를 깨뜨릴 수 있으나, 어차피 동작하지 않던 것 |
| CODE-30 | 1,000줄 초과 파일 3개 | Med | `engine/src/cypher/compile.rs`(1,591), `engine/src/cypher/parser.rs`(1,177), `engine/src/typeql/parser.rs`(1,108) | 한 파일이 여러 관심사를 담아 리뷰 단위가 크고 테스트 진입점이 흐릿하다 | 아래 "3. 분할 축 제안" 참조 | 리뷰 단위 축소, 테스트 작성 유도 | 단순 분할은 이득이 없다. `impl Compiler` 블록을 파일로 쪼개면 오히려 찾기 어려워질 수 있음 |
| CODE-27 | SQL 회귀 판정이 출력 비교가 아니라 오류 개수 비교 | Med | `tests/run.sh:23-38`; `engine/tests/pg_regress/expected/`에 `setup.out` 1개뿐, `engine/tests/sql/`에 대응 `.out` 없음 | `actual <= expected`(ERROR 줄 수)만 본다. 질의가 **틀린 답**을 돌려줘도 통과한다. `05_reachability.sql`의 `LIKE '%og_reach(%' AS ...` 불리언 컬럼이 `f`여도 통과한다 | ① `psql -f` 출력을 `engine/tests/expected/<name>.out`과 diff ② 최소한 불리언 어서션 컬럼에 대해 `\if` + `\echo FAIL` 패턴 도입 | 회귀를 실제로 잡음 | 출력 diff는 부동소수·타이밍·OID 때문에 불안정해질 수 있다 — 어서션 방식(②)이 더 안전 |
| CODE-09 | 쓰기 경로의 SPI 왕복 증폭 | Med | `engine/src/storage/mod.rs:160-222`(`plan_props`), `mod.rs:253-291`, `mod.rs:402-452`; `engine/src/storage/adjacency.rs:19-44` | 노드 1개 생성에 최소 SPI 4회(`og_id_alloc` UPSERT, 프로퍼티 조회, 레지스트리 INSERT, 타입 테이블 INSERT). 엣지는 롤 조회 + 인접 append 2회가 더해져 7회 이상. `UNWIND $rows AS r CREATE …`로 1만 행을 쓰면 7만 회 이상 | ① `plan_props`의 프로퍼티 조회를 `(type_id → PropPlan)` 백엔드-로컬 캐시로(스키마 버전으로 무효화) ② `create_pattern`을 배치화해 같은 타입의 노드를 다중행 INSERT로 | 배치 적재 처리량 개선 | ①은 `CODE-01`과 같은 무효화 문제를 새로 만든다 — 스키마 버전 키를 반드시 포함 |
| CODE-31 | `plan_props` 내 프로퍼티 조회 쿼리 완전 중복 | Med | `engine/src/storage/mod.rs:161-177` 과 `mod.rs:181-197` | 동일한 SELECT + 동일한 `.map()` 튜플 변환이 두 번 그대로 적혀 있다(`declare_new_props`가 스키마를 바꿨을 때 다시 읽기 위함). 유사 쿼리가 `engine/src/cypher/views.rs:36-52`에도 있다(`type_id = ANY($1)`) | `fn load_properties(type_id: i32) -> Vec<(String,String,String)>`으로 추출하고 두 곳에서 호출 | 20여 줄 감소, 변경 지점 1곳 | 없음 |
| CODE-10 | `prefer_reachability`가 가변 길이 홉마다 SPI 질의 | Med | `engine/src/cypher/compile.rs:46-50`, 호출부 `compile.rs:865` | 컴파일 시점에 `pg_class.reltuples` 조회를 홉마다 1회. 캐시 미스마다 발생하며, 다중 가변 길이 홉 패턴이면 홉 수만큼 | `Compiler`에 `Option<(f64,f64)>` 필드를 두고 첫 호출 결과를 재사용 | 컴파일 지연 감소 | 없음. 값은 한 컴파일 안에서 바뀌지 않는다 |
| CODE-26 | `og_audit.error_code`에 코드가 아니라 메시지가 들어감 | Med | `engine/sql/bootstrap.sql:385`(컬럼 정의); `engine/src/cypher/mod.rs:125-132`; `engine/src/typeql/mod.rs:118-125` | `error_code`에 `err.map(\|e\| e.chars().take(200).collect())` — 오류 **메시지 앞 200자**를 넣는다. 컬럼 이름과 내용이 불일치하고, 200자에서 잘려 원문도 아니다 | `CODE-32`의 오류 종류를 `error_code`에, 전문을 새 `error_message` 컬럼에 | 감사 로그로 오류 유형 집계 가능 | 스키마 변경 → `pg_extension_config_dump` 재확인 필요 |
| CODE-20 | Bolt 게이트웨이가 결과를 전량 버퍼링 | Med | `bolt/src/session.rs:291-320`(RUN 시점에 전 행 수집), `session.rs:333-357`(PULL은 버퍼를 자를 뿐) | `PULL {n: 10}`을 보내도 이미 전체 결과가 메모리에 있다. 게이트웨이 메모리가 결과 크기에 비례하고, 커넥션마다 스레드 1개이므로 합산된다 | PostgreSQL 커서(`DECLARE … CURSOR`) 또는 `postgres` 크레이트의 `query_raw` 스트리밍으로 전환 | 대용량 결과에서 메모리 상한 확보 | `og_cypher()`가 SRF이므로 커서로 감쌀 수 있으나, 트랜잭션 밖 커서는 `WITH HOLD`가 필요 |
| CODE-12 | `catalog` → `cypher` 순환 의존 | Med | `engine/src/catalog/labeling.rs:175`(`crate::cypher::views::drop_all_views()`); `engine/src/catalog/types.rs:564-566`(`crate::cypher::compile::sql_str`) | 도메인 코어(`catalog`)가 상위 계층(`cypher`)을 역참조한다. `cypher`만 떼어 테스트하거나 교체할 수 없다 | ① `sql_str`/`quote_ident`를 `engine/src/sqltext.rs`(무의존 유틸)로 이동 ② 뷰 무효화를 콜백/옵저버로 뒤집거나 `views`를 `catalog` 아래로 이동 | 의존 방향이 단방향이 됨 | ②는 구조 변경이라 diff가 큼. ①만 해도 절반 해소 |

---

## 3. Low

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| CODE-03 | `push_prop_filters` / `push_rel_prop_filters` 본문 동일 | Low | `engine/src/cypher/compile.rs:803-815` 와 `compile.rs:957-969` | 시그니처도 본문도 완전히 같다. 한쪽만 고치는 버그가 생길 자리 | 하나로 합치고 호출부 4곳(`compile.rs:749, 765, 799, 933, 951`)을 갱신 | 13줄 감소 | 없음 |
| CODE-04 | `mentions_alias`가 문자열 부분 일치 | Low | `engine/src/cypher/compile.rs:1575-1580`, 사용처 `compile.rs:252-275` | `"{alias}."` 부분 문자열을 찾는다. SQL 문자열 리터럴 안의 `'n1.x'`도 매치한다. OPTIONAL MATCH 술어가 잘못된 조인의 `ON`에 붙을 수 있다. 실제 발생 조건이 좁아 **재현 여부 미확인** | 술어를 문자열이 아니라 `(sql, referenced_aliases: HashSet<String>)` 구조로 들고 다닌다 | 술어 배치가 확실해짐 | `constrain`/`close_optional` 시그니처 변경 |
| CODE-14 | `Compiler.ctes`가 죽은 필드 | Low | `engine/src/cypher/compile.rs:116`(선언), `compile.rs:593-597`(소비). `self.ctes.push(...)` 호출부 **0건** | `WITH RECURSIVE` 조립 코드가 있으나 항목이 채워지지 않아 항상 빈 문자열 | 제거하거나, 남긴다면 "향후 CTE 기반 재작성용" 주석을 명시 | 죽은 코드 제거 | 없음 |
| CODE-19 | Bolt `speaks()`의 range 판정이 무효 | Low | `bolt/src/session.rs:103-108` | `major == 4 && minor >= 4 && minor - range.min(minor) <= 4`. `minor >= 4`가 이미 요구된 상태에서 세 번째 항은 항상 참이다. range 바이트가 실질적으로 무시된다 | 조건을 `major == 4 && minor >= 4`로 단순화하거나, 의도가 "4.0~4.4 범위 제안 수용"이라면 하한 검사를 제대로 구현 | 코드 의도가 명확해짐 | 없음 — 동작은 그대로 |
| CODE-21 | Bolt 노드의 `labels`가 항상 원소 1개 | Low | `bolt/src/session.rs:534-537`(`Value::List(vec![string(ty)])`); 대조: `engine/src/cypher/compile.rs:1470-1478` `labels()`는 상위 타입 사슬 전체 반환 | Cypher 함수 `labels(n)`은 `["Vehicle","Car","EV"]`를 주는데, Bolt Node 구조체의 labels는 `["EV"]` 하나다. 같은 노드가 표면에 따라 다른 라벨을 갖는다 | `og_cypher()` 노드 jsonb에 `_labels` 배열을 추가하고 게이트웨이가 그것을 쓰도록 | 두 표면의 일관성 | jsonb 크기 증가. `to_bolt`의 `_` 접두사 필터(`session.rs:518`)가 이미 처리 |
| CODE-22 | `bolt/src/session.rs`에 테스트 0 | Low | `bolt/`의 `#[cfg(test)]`는 `packstream.rs:285` 하나뿐 | 606줄 중 순수 함수가 6개(`speaks` `split_plan_prefix` `to_bolt` `to_json` `record` `Failure::from_pg`)인데 테스트가 없다 | 해당 6개에 테이블 주도 테스트 추가 | `CODE-11`/`CODE-19`/`CODE-21`을 고정 | 없음 |
| CODE-28 | `pg_regress` 스캐폴드가 잘못된 확장 이름 | Low | `engine/tests/pg_regress/sql/setup.sql`: `CREATE EXTENSION engine;` vs `engine/ontological.control`, `engine/Cargo.toml:2`(`name = "ontological"`) | `CREATE EXTENSION engine`은 존재하지 않는다. `#[pg_test]`가 하나도 없어 현재 아무것도 실행되지 않으므로 실패가 드러나지 않는다 | `CREATE EXTENSION ontological CASCADE;`로 수정하고, `#[pg_test]`를 도입할지 결정. 도입하지 않을 거면 디렉터리 제거 | 죽은 스캐폴드 정리 | 없음 |
| CODE-35 | `set_node_props_inner` / `set_edge_props_inner` 중복 | Low | `engine/src/storage/mod.rs:298-326` 와 `mod.rs:329-347` | UPDATE 문 조립 로직이 거의 동일하다. 노드 쪽만 "컬럼이 없으면 `__ext`만 갱신" 조기 반환(`mod.rs:303-312`)이 있고 엣지 쪽에는 없다 — 비대칭이 의도인지 불명 | 공통 `fn write_props_to(table, id, plan, props)`로 추출 | 30여 줄 감소, 비대칭 해소 | 조기 반환 분기의 의도를 먼저 확인해야 함 |
| CODE-36 | 문자열 기반 SQL 조립 — 빌더 부재 | Low | `engine/src/cypher/compile.rs` 전반(`from: Vec<String>`, `wheres: Vec<String>`); `engine/src/typeql/compile.rs:46-47` 동일 | FROM 항목과 술어가 전부 `String`이다. 별칭 참조를 알 수 없어 `mentions_alias`(`CODE-04`) 같은 문자열 검사가 필요해졌고, `move_join_to_end`(`CODE-15`) 같은 인덱스 조작이 생겼다 | 최소한 `struct Join { sql: String, alias: String, refs: Vec<String>, kind: JoinKind }`로 승격 | 조인 조작이 안전해지고 `CODE-04`/`CODE-15`가 구조적으로 해소 | 큰 리팩터. `CODE-29`(테스트)를 먼저 하지 않으면 위험 |
| CODE-37 | 감사 INSERT가 질의마다 1회 | Low | `engine/src/cypher/mod.rs:122-135`, `engine/src/typeql/mod.rs:115-128` | 읽기 질의에도 `og_data.og_audit`에 INSERT가 붙는다. 읽기 전용 트랜잭션에서는 실패해 `.ok()`로 삼켜지므로 **감사 기록이 전무**하다 | 설정(`og_catalog.setting`)으로 on/off 가능하게 하고, 읽기 전용 세션에서는 감사 불가를 한 번 `notice`로 알린다 | 읽기 처리량 개선 + 감사 공백 가시화 | 기본값을 off로 바꾸면 spec 008 FR-027 기대와 어긋남 |

---

## 4. 1,000줄 초과 파일의 분할 축 제안 (`CODE-30` 상세)

**단순히 줄 수로 나누는 것은 이득이 없다.** 아래는 관심사 경계에 따른 축이다.

### `engine/src/cypher/compile.rs` (1,591줄)

| 새 파일 | 옮길 것 | 근거 라인 | 이 축인 이유 |
|---|---|---|---|
| `compile/mod.rs` | `Compiler` 구조체, `Bind`, `Compiled`, `new`, `fresh`, 절 루프(`compile_read`, `compile_match`, `compile_with`, `compile_call`) | 102–470 | 파이프라인 골격 |
| `compile/pattern.rs` | `compile_pattern`, `bind_node`, `join_rel`, `push_prop_filters`, `resolve_label*` | 647–969 | **패턴 → 조인**. 가장 자주 바뀌고 가장 위험한 부분 |
| `compile/expr.rs` | `expr`, `binary`, `func`, `type_of`, `prop_sql*`, `jsonb_arg`, `element_id_sql`, `type_id_sql`, `node_json`, `rel_json` | 976–1568 | **표현식 → SQL**. 패턴과 독립적으로 확장된다 |
| `compile/select.rs` | `build_core`, `build_select`, `build_tabular` | 477–641 | 투영/정렬/그룹화 조립 |
| `compile/optional.rs` | `OptionalScope`, `constrain`, `close_optional`, `note_optional_join`, `move_join_to_end`, `mentions_alias` | 130–275, 1575–1580 | OPTIONAL MATCH 술어 배치 — `CODE-04`/`CODE-15`가 여기 모인다 |
| `compile/reachability.rs` | `prefer_reachability`, `blind_expr`, `multiplicity_blind` | 20–100, 339–349 | **DB 없이 테스트 가능**. `CODE-29`의 첫 대상 |
| `sqltext.rs` (모듈 밖) | `quote_ident`, `sql_str` | 1584–1591 | `catalog/types.rs:564-566`이 이미 빌려 쓴다 → `CODE-12` 동시 해소 |

### `engine/src/cypher/parser.rs` (1,177줄)

| 새 파일 | 옮길 것 | 근거 라인 |
|---|---|---|
| `parser/mod.rs` | `Parser` 구조체, 토큰 헬퍼, `parse_query`, 절 디스패치 | 14–243 |
| `parser/pattern.rs` | `parse_pattern`, `parse_node_pat`, `parse_rel_pat`, `parse_prop_map` | 614–742 |
| `parser/expr.rs` | 우선순위 사슬 전체 (`parse_or` … `parse_case`) | 748–1130 |
| `parser/ddl.rs` | `create_is_ddl`, `parse_ddl_*`, `parse_drop`, 비예약어 헬퍼 | 245–509 |

DDL 분리가 특히 명확하다 — `lexer.rs:24-27`이 명시한 "예약하지 않은 단어" 처리가
전부 그쪽에 몰려 있다.

### `engine/src/typeql/parser.rs` (1,108줄)

| 새 파일 | 옮길 것 |
|---|---|
| `parser/mod.rs` | 스테이지 디스패치 (`parser.rs:140-175`) |
| `parser/schema.rs` | `define` 본문 파싱 (타입/`owns`/`relates`/`plays`/어노테이션) |
| `parser/pattern.rs` | `match`/`insert` 패턴 파싱 |
| `parser/fetch.rs` | `fetch` 문서 파싱 |

---

## 5. 조사 방법 (재현 가능)

```bash
# unwrap/expect/panic 규모
grep -rn "unwrap()\|expect(\|panic!" engine/src --include=*.rs | wc -l          # 202
grep -rc "unwrap()\|expect(\|panic!" engine/src --include=*.rs | sort -t: -k2 -rn

# 테스트 모듈
grep -rn "#\[cfg(test)\]" engine/src bolt/src --include=*.rs
grep -rc "#\[test\]" engine/src bolt/src --include=*.rs | grep -v ":0"

# 결과를 버리는 SPI 호출
grep -rn "let _ = Spi::run" engine/src --include=*.rs
grep -rn "Spi::run.*\.ok();" engine/src --include=*.rs

# 오류 타입
grep -rn "type .*Result<T> = Result<T, String>" engine/src --include=*.rs

# 파일 크기
wc -l engine/src/**/*.rs engine/src/*.rs bolt/src/*.rs | sort -rn | head -20

# 락 관련 (전부 0건)
grep -rni "advisory\|for update\|lock table\|SERIALIZABLE\|SAVEPOINT" engine/src bolt/src engine/sql
```

---

## 6. 권장 처리 순서

1. **`CODE-29`** (테스트 먼저) — 순수 함수 테스트 없이 아래를 고치면 회귀를 못 잡는다.
2. **`CODE-33`, `CODE-34`** — 답이 틀리는 버그. 테스트와 함께.
3. **`CODE-01`** — 캐시 무효화. `CODE-09`의 프로퍼티 캐시 도입 전에 해야 같은 실수를 반복하지 않는다.
4. **`CODE-06`, `CODE-07`** — 타입 확장 경로. 둘이 같은 코드 블록이므로 함께.
5. **`CODE-02`** — 한 줄 변경.
6. **`CODE-03`, `CODE-31`, `CODE-35`, `CODE-14`, `CODE-28`, `CODE-19`** — 저위험 정리. 묶어서 한 커밋.
7. **`CODE-08`** — TypeQL 파라미터 바인딩. 독립적이고 diff가 크다.
8. **`CODE-32` → `CODE-11` → `CODE-26`** — 오류 타입 도입. 가장 큰 리팩터이므로 마지막.

---

## 7. 이 문서에서 "미확인"으로 남긴 것

| 항목 | 무엇이 미확인인가 | 확인 방법 |
|---|---|---|
| `CODE-15` | 현재 `og_cypher_sql`의 실제 FROM 순서 | `SELECT og_cypher_sql('default', $$ MATCH (p:Person)-[:ACTED_IN]->(w:Work) WHERE p.born > 1960 RETURN w.title $$);` |
| `CODE-17` | 관계가 관계의 플레이어가 되는 순환 구조를 실제로 만들 수 있는지 | TypeQL `define`으로 상호 참조 관계를 선언한 뒤 `delete` 시도 |
| `CODE-24` | `og_node_json`을 `PARALLEL SAFE`로 선언해도 안전한지 | 동적 `EXECUTE`가 병렬 워커에서 허용되는지 PostgreSQL 문서/실험 |
| `CODE-04` | `mentions_alias` 오탐이 실제 질의로 재현되는지 | OPTIONAL MATCH + 별칭 이름을 담은 문자열 리터럴 조합 |
| `CODE-23` | 인접 append 경쟁이 실제 부하에서 얼마나 자주 나는지 | 같은 노드에 동시 `CREATE (a)-[:R]->(b)` 부하 테스트 |

<!-- affects: backend, quality, performance, api -->
<!-- requires-update: 08_operations/, 03_backend/10_coding_rules.md -->
