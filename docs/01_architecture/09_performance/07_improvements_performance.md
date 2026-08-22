# 성능 개선 포인트 (PERF-01 ~ PERF-30)

> **이 문서가 답하는 질문**
> - 지금 이 코드에서 성능을 가장 크게 되찾을 수 있는 지점은 어디인가?
> - 각 제안의 근거는 어느 파일 몇 번째 줄인가?
> - 제안이 실제로 효과가 있는지 **어떻게 확인**하는가?

---

## 0. 규칙

**필수**

- ✅ 모든 항목은 실제 코드 라인 또는 실측 JSON을 근거로 한다.
- ✅ 측정되지 않은 효과는 **"추정"**이라고 적고, 어떤 측정치에서 어떻게 유도했는지 밝힌다.
- ✅ 각 항목의 "검증 방법"을 먼저 돌려 현상을 재현한 뒤에 고친다.

**금지**

- ❌ 여러 항목을 한 커밋에 섞는 것. 회귀 게이트가 원인을 지목할 수 없게 된다.
- ❌ 벤치를 다시 돌리지 않고 "빨라졌다"고 쓰는 것.
- ❌ 정확성을 바꾸는 최적화(PERF-03 등)를 정확성 회귀 테스트 없이 넣는 것.

---

## 1. 요약표

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| PERF-01 | `WHERE x = v` 가 `IS NOT DISTINCT FROM` 으로 내려가 인덱스를 못 쓴다 | **High** | `engine/src/cypher/compile.rs:1377` | `=` 대신 `DistinctExpr` 생성. 인라인 맵 표기와 다른 SQL | NULL 가능성이 없는 쪽에는 `=` 방출, 필요할 때만 `IS NOT DISTINCT FROM` | 라벨 스캔이 시퀀셜 → 인덱스로 전환 (추정) | `NULL = NULL` 의미가 바뀜. Cypher는 `=` 를 NULL 전파로 정의 |
| PERF-02 | `count(DISTINCT b)` 가 노드 jsonb 전체를 비교 | **High** | `compile.rs:1421-1427`, `:1095-1113` | 정렬 키가 jsonb. `work_mem` 초과 시 디스크 정렬 | 인자가 `Bind::Node`/`Rel` 이고 집계가 `count(DISTINCT)` 면 `alias.id` 로 대체 | 4홉 195,202 → ? 페이지. 상한 8~10배 (추정) | 없음에 가까움 (id는 유일) |
| PERF-03 | 도착 노드를 타입 뷰로 재조인한다 | **High** | `compile.rs:770-800`, `:906` | 도달 노드마다 PK 프로브 + 힙 페치. 4홉에서 179,032 페이지 초과분 | 라벨 확인만 필요할 때 `og_id_type(u.nbr) = ANY(ARRAY[...])` 로 대체 | 4홉 193 ms → 25~60 ms (추정) | 인접에 매달린 삭제된 노드를 걸러 내지 못함 |
| PERF-04 | `og_reach` 레벨 루프의 복사·해시·물질화 | Med | `traverse.rs:118,135-139,157` | 레벨마다 `frontier.clone()`, `Vec<Vec<Option<i64>>>`, SipHash | 슬라이스 전달, `Vec<i64>` 수신, FxHash 또는 비트맵, 스트리밍 반환 | dense 6홉 71 ms의 10~30% (추정) | 없음 |
| PERF-05 | `og_type_name()` 과 `eid` 언네스트가 불필요하게 방출됨 | Med | `compile.rs:1101`, `:901-903` | 타입이 컴파일 상수인데도 행마다 카탈로그 조회. 관계 변수가 없는데도 `eid` 배열을 디폼 | 상수 타입은 문자열 리터럴로. 홉이 1개이고 관계 변수가 없으면 `unnest(nbr)` 만 | 1홉 프로젝션 비용 감소 (추정) | 동형성(isomorphism) 검사가 필요한 다중 홉에서는 `eid` 가 필요 |
| PERF-06 | `PLAN_CACHE` 에 무효화 키가 없다 | **High** | `cypher/mod.rs:26-30,47-67`, `labeling.rs:172-182` | 스키마 변경이 모든 타입 뷰를 DROP 하지만 캐시된 SQL은 그대로 | 캐시 키에 `og_catalog.schema_version` 의 현재 값을 포함 | 스키마 변경 후 첫 질의 실패/구식 결과 제거 | 캐시 미스가 늘어 컴파일 SPI 9회를 다시 냄 |
| PERF-07 | 방문집합 BFS가 `WITH` 하나로 전면 비활성 | Med | `compile.rs:340-342` | `WITH` 가 있으면 깊이 20이어도 트레일 열거 | 마지막 `WITH` 이후 세그먼트만 검사하거나, 각 `WITH` 프로젝션에 `blind_expr` 를 적용 | 해당 질의가 초 단위 → 밀리초 단위 (측정된 691배 구간) | 판정을 틀리면 **답이 바뀐다** |
| PERF-08 | `og_id_alloc` 의 `ON CONFLICT DO UPDATE` 가 타입당 직렬화를 만든다 | **High** | `storage/mod.rs:24-34` | 같은 타입에 동시 삽입하는 모든 트랜잭션이 한 행의 튜플 락에서 커밋까지 대기 | 타입별 시퀀스(`nextval`) 또는 백엔드별 블록 선할당 | 동시 쓰기 확장성이 1 → 세션 수에 비례 (추정) | id에 구멍이 생김. `pg_extension_config_dump` 등록 필요 |
| PERF-09 | 인접 세그먼트 append의 쓰기 증폭 | **High** | `adjacency.rs:19-44` | `nbr = a.nbr \|\| $4` 는 최대 4 KB 튜플 전체를 새 버전으로 쓴다. `max(seq)` 상관 서브질의도 매번 | 꼬리 세그먼트를 작게(예: 32) 유지하고 `og_reorganize` 로 병합, 다중 이웃 배치 append | 누적 쓰기량 약 8배 감소 (추정) | 세그먼트 수 증가 → 읽기 시 튜플 수 증가 |
| PERF-10 | 카탈로그 캐시가 없다 — 노드 1개에 SPI 9회 | **High** | `storage/mod.rs:246-291`, `types.rs:112-127,286-294,616-622`, `spiu.rs:15-48` | 6/9가 순수 카탈로그 조회. `declare_new_props` 는 매 쓰기마다 이름 조회 | 백엔드-로컬 타입 카탈로그 캐시 + `schema_version` 무효화 | 노드 1개 SPI 9회 → 3회 (추정) | 캐시 일관성. 같은 트랜잭션 내 DDL |
| PERF-11 | 벌크 로드 경로가 없다 | **High** | `cypher/mod.rs:236-243`, `storage/mod.rs` 전반 | `COPY`/`copy_in` 이 코드에 없음. Cypher 쓰기는 행마다 위 절차를 반복 | `UNWIND $rows CREATE …` 를 다중 행 `INSERT … SELECT unnest(...)` 로 컴파일 | 배치 쓰기 처리량 한 자릿수 배 (추정) | `MERGE`/트리거/제약과의 상호작용 |
| PERF-12 | 결과가 스트리밍되지 않고 이중 변환된다 | **High** | `cypher/mod.rs:145-152,108` | 모든 행을 `Vec<serde_json::Value>` 로 물질화, jsonb → serde_json → JsonB 왕복 | `TableIterator`/커서로 스트리밍, `JsonB` 를 그대로 통과 | 큰 결과의 메모리 상한 제거 (미측정) | `og_cypher_stats` 의 카운트 시점 |
| PERF-13 | 읽기 질의마다 `og_audit` 에 INSERT | Med | `cypher/mod.rs:107,122-135` | 모든 읽기가 쓰기가 된다. WAL, 무제한 증가, 읽기 복제본 불가 | 설정으로 on/off, 샘플링, 또는 UNLOGGED 테이블 | 질의당 고정 비용 제거 (추정 0.05~0.15 ms) | 감사 요구사항(spec 008 FR-027) 약화 |
| PERF-14 | 콜드 백엔드 첫 호출이 데이터와 무관하게 ~1,170 페이지를 읽는다 | Med | `03_hot_paths.md §2`, `harness.py:183-206` | 5k/50k/250k 노드에서 1,170/1,173/1,177 페이지로 일정 | 원인 확정 후 PERF-10의 캐시로 제거 | 벤치의 페이지 열이 실제 저장 비용을 반영하게 됨 | 없음 (진단) |
| PERF-15 | `og_reach_sql` 이 어디에도 배선되어 있지 않다 | Med | `access.sql:169-187`, `compile.rs:865` | 얇고 깊은 프론티어에서 `og_reach` 가 CTE에 6.6배 진다 | 프론티어 폭 추정치를 만들고 세 번째 선택지로 편입 | chain 100,000홉 1,016 ms → 154 ms (**측정**) | 프론티어 겹침 통계가 없어 오판 가능 |
| PERF-16 | Bolt RUN 1회에 PostgreSQL 왕복 3회, Cypher 파싱 4~5회 | Med | `bolt/src/session.rs:283-299,444-461` | `og_cypher_check` + `og_cypher_columns` + `og_cypher` | 컬럼과 write 판정을 `og_cypher` 결과에 함께 실어 1회로 | RUN당 왕복 3 → 1 (추정) | 반환 시그니처 변경 |
| PERF-17 | PackStream 경로가 레코드마다 clone·flush·시스템 콜 | Med | `session.rs:353`, `packstream.rs:254-262`, `session.rs:291-299` | `pending[i].clone()` 깊은 복사, 레코드마다 `flush()`, 무버퍼 `TcpStream`, `::text` 캐스트 | `BufWriter` 로 감싸고 PULL 끝에서만 flush, `into_iter()` 로 소유권 이동, 바이너리 jsonb 전송 | 대량 결과의 시스템 콜 1/N (추정) | Bolt 프레이밍 규칙 준수 확인 필요 |
| PERF-18 | Studio 서버가 전체 결과를 메모리에 적재 | **High** | `portal/server/index.js:188-215,317-342,39-45` | `pg.query()` 버퍼링 → `map` → `projectGraph` → `JSON.stringify`. 행 상한·타임아웃 없음 | 커서(`pg-cursor`) 스트리밍 + 행 상한 + `statement_timeout` | Node 프로세스 OOM 제거 | UI가 부분 결과를 다뤄야 함 |
| PERF-19 | 회귀 스위트가 단언 값을 검사하지 않는다 | **High** | `tests/run.sh:14-36`, `engine/tests/sql/05_reachability.sql` | `ERROR` 개수만 센다. `f` 를 출력해도 통과 | `pg_regress` 기대 출력 도입 또는 `\if`/`ASSERT` 로 실패시키기 | 전환 판정 회귀가 실제로 잡힘 | 기대 출력 유지 비용 |
| PERF-20 | `minhop > 1` 에서 재작성이 다른 답을 낸다 | **High** | `traverse.rs:143-153`, `compile.rs:865,874-876` | `og_vlp` 는 트레일 길이, `og_reach` 는 최단 거리 | `min > 1` 이면 재작성 금지, 또는 `og_reach` 에 최단-거리 아닌 의미 추가 | 정확성 확보 (성능은 소폭 손해) | 그만큼 열거 경로로 돌아감 |
| PERF-21 | `ROWS` 고정 추정치가 계획을 왜곡한다 | Med | `access.sql:140,171,197` | 계획에 실제로 쓰이는 두 개(`og_vlp` 100, `og_reach` 100)가 최대 67만 배 빗나감 | `maxhop`·`reltuples` 기반의 `support function`(`SUPPORT`)으로 동적 추정 | 상위 조인이 해시/머지로 바뀔 여지 | 추정이 커지면 다른 계획이 더 나빠질 수 있음 |
| PERF-22 | 벡터 검색이 뷰 위에서 실행되고 `ef_search` 를 설정하지 않는다 | Med | `vector/mod.rs:112,126-132,115-118` | `UNION ALL` 뷰 위 `ORDER BY … LIMIT k`. `filter` 는 텍스트 보간. `hnsw.ef_search` 미설정 | 서브타입이 1개면 구체 테이블 직접 지정, `SET LOCAL hnsw.ef_search`, 필터 선택도에 따른 경로 분기 | HNSW 인덱스 유지 + recall 제어 | pgvector 버전별 동작 차이 |
| PERF-23 | `og_hybrid_search` 가 근접도를 `og_vlp` 로 계산한다 | Med | `vector/mod.rs:251-256` | 양방향·전 타입·깊이 3 트레일 열거 | `og_reach` 로 교체 (다중도가 관측되지 않음) | 3홉 6.85 → 1.61 ms 비율 (**측정**, 단방향 dense 기준) | 없음 — `min(depth)` 로 그룹핑하므로 의미 동일 |
| PERF-24 | 병렬 질의를 사실상 쓰지 못한다 | Med | `cypher/mod.rs:83`, `traverse.rs:80,359,442`, `#[pg_extern]` 78개 중 61개 무표기 | `og_cypher` 가 `PARALLEL UNSAFE` → 최상위 계획 직렬 | 읽기 전용 함수에 `parallel_safe`/`restricted` 부여, SPI 내부 병렬 가능 여부 확인 | 큰 스캔에서 워커 수만큼 (미측정) | SPI를 쓰는 함수를 `parallel_safe` 로 두면 안 됨 |
| PERF-25 | 빌드 설정이 `bolt` 크레이트에 적용되지 않았다 | Low | `bolt/Cargo.toml`, `engine/Cargo.toml` | engine은 `lto="fat"`, `codegen-units=1`. bolt는 `opt-level=3` 만 | bolt에도 동일 프로파일 적용 | 직렬화 핫루프 수 % (추정) | 빌드 시간 증가 |
| PERF-26 | `chunk_size` / `supernode_threshold` 설정이 코드에 반영되지 않는다 | Low | `bootstrap.sql:256-260`, `adjacency.rs:15` | 설정 테이블에 값이 있으나 코드는 하드코딩 `CHUNK = 256` 을 쓴다 | 설정을 읽어 캐시하거나, 설정 행을 지우고 문서에서 상수로 선언 | 튜닝 가능성 확보 | 세그먼트 크기 변경은 기존 데이터와 호환되어야 함 |
| PERF-27 | 회귀 게이트가 페이지 수와 `reach*` 워크로드를 보지 않는다 | **High** | `harness.py:1221-1241`, `bench/results/baseline.json` | `median_ms` 만 비교. 베이스라인에 `reach*` 없음. 없는 질의는 조용히 통과 | `buffers` 비교 추가, `reach` 베이스라인 생성, 누락 질의를 실패로 | 저장 구조 회귀가 실제로 잡힘 | 임계값 조율 필요 (서브밀리초 노이즈) |
| PERF-28 | 라벨 스캔의 `UNION ALL` 브랜치 수가 서브타입 수에 비례한다 | Low (미확인) | `views.rs:102-135` | 큰 온톨로지에서 `MATCH (v:Thing)` 이 N개 스캔의 `Append` 가 됨 | 파티션 테이블 또는 `og_node` 앵커 + `type_id = ANY(...)` 경로를 대안으로 제공 | 미측정 | 실컬럼 통계의 이점을 잃음 |
| PERF-29 | `og_csr_build` 가 자동이 아니고 무효화 전략이 없다 | Med | `traverse.rs:205-210,241-292` | 백엔드-로컬·동결 스냅샷·RLS 미적용. 1M 노드에서 935~968 ms / 23~31 MB | 옵트인 GUC + `schema_version`/엣지 카운터 기반 스테일 감지 + `og_csr_stats` 노출 | dense 6홉 71 → 4.9 ms (**측정**) | MVCC·RLS 포기. 커넥션 폭주 시 메모리 |
| PERF-30 | `og_node_json` / `og_edge_json` 이 plpgsql + 동적 `EXECUTE` 다 | **High** | `access.sql:208-264`, `compile.rs:991,1013`, `access.sql:307-318` | 행마다 SQL 2개 + 동적 `EXECUTE` 1개. 옵티마이저 장벽 | 타입이 컴파일 시점에 확정되면 절대 방출하지 않기, 확정 불가면 `type_id` 별 분기 SQL 생성 | 라벨 없는 패턴·TypeQL 뷰에서 큰 폭 (미측정) | 타입 미상 노드의 일반성 유지 |

---

## 2. 상세

### PERF-01 — `WHERE x = v` 가 인덱스를 못 쓴다

- **심각도**: High
- **근거**: [`engine/src/cypher/compile.rs:1377-1378`](../../../engine/src/cypher/compile.rs)
  ```rust
  BinOp::Eq => format!("({ls} IS NOT DISTINCT FROM {rs})"),
  BinOp::Ne => format!("({ls} IS DISTINCT FROM {rs})"),
  ```
  대비 [`compile.rs:812`](../../../engine/src/cypher/compile.rs) (인라인 맵):
  ```rust
  self.constrain(format!("{lhs} = {rhs}"));
  ```
- **현상**: `MATCH (a:P) WHERE a.val = 7` 은 `DistinctExpr` 를 만든다.
  PostgreSQL의 인덱스 경로 생성기는 `OpExpr` / `ScalarArrayOpExpr` / `NullTest` / `RowCompareExpr` 만
  인덱스 조건으로 매칭하며 `DistinctExpr` 은 그 목록에 없다 → `og_create_index` 로 만든 B-tree가 무시된다.
  같은 뜻을 `MATCH (a:P {val: 7})` 로 쓰면 `n1.p_val = 7` 이 되어 인덱스를 쓴다.
- **정량적 추정**: 공개 벤치의 1홉 페이지 수 1,742 (`ontological`) 대 389 (`ontological_raw`)의
  차이 일부가 여기서 온다는 것이 **가설**이다. `og_create_index` 후에도 인덱스가 쓰이지 않는다면
  50,000행 타입 테이블의 시퀀셜 스캔이 매 홉 질의에 포함된다.
  `docs/benchmark.md` 가 "Ontological was found to have the same gap and was given `og_create_index`"
  라고 적은 조치가 **하네스가 쓰는 표기에서는 무효**일 수 있다.
- **제안**: 비교의 양변 중 하나라도 NULL이 될 수 없다고 판단되는 경우
  (실컬럼 대 리터럴, 실컬럼 대 파라미터) `=` 를 방출한다.
  Cypher의 `=` 는 NULL 전파(`NULL = x` → NULL)이므로 오히려 `=` 가 의미상 정확하다.
  `IS NOT DISTINCT FROM` 은 Cypher `=` 의 정의와 다르다는 점도 함께 검토한다.
- **예상 효과**: 라벨 스캔이 시퀀셜 → 인덱스로 전환 (추정). 1홉/프로퍼티 스캔 지연과 페이지 수 모두 감소.
- **리스크**: `NULL = NULL` 의 결과가 `true` 에서 `NULL` 로 바뀐다. 기존 질의의 결과가 달라질 수 있다.
  `engine/tests/sql/02_cypher.sql` 에 NULL 비교 케이스를 먼저 추가할 것.
- **검증 방법**:
  ```sql
  -- 1) 두 표기가 다른 SQL을 만드는지
  SELECT og_cypher_sql('benchg', $$MATCH (a:P) WHERE a.val = 7 RETURN count(a)$$);
  SELECT og_cypher_sql('benchg', $$MATCH (a:P {val:7}) RETURN count(a)$$);

  -- 2) 각각의 계획
  SELECT og_cypher_explain('benchg', $$MATCH (a:P) WHERE a.val = 7 RETURN count(a)$$, true);
  SELECT og_cypher_explain('benchg', $$MATCH (a:P {val:7}) RETURN count(a)$$, true);
  -- Seq Scan on n_2  vs  Index Scan using ix_2_p_val 인지 확인
  ```

### PERF-02 — `count(DISTINCT b)` 가 노드 jsonb 전체를 비교한다

- **심각도**: High
- **근거**: [`compile.rs:1421-1427`](../../../engine/src/cypher/compile.rs) 의 `count` 처리 →
  `self.expr(&args[0])` → [`compile.rs:1167-1175`](../../../engine/src/cypher/compile.rs) `Expr::Var`
  → `var_value` → [`compile.rs:1095-1113`](../../../engine/src/cypher/compile.rs) `node_json`.
  결과 SQL: `count(DISTINCT (jsonb_strip_nulls(jsonb_build_object('_id', n2.id, '_type', og_type_name(n2.type_id), …)) || COALESCE(n2.__ext,'{}'::jsonb)))`
- **현상**: 도달 노드마다 jsonb 객체를 만들고, `DISTINCT` 정렬 키가 jsonb가 된다.
  `docs/deep-traversal.md` 도 이 비용을 지목한다 —
  *"most of it building a jsonb object per node so `count(DISTINCT b)` can compare whole nodes."*
- **정량적 추정**: 50,000 노드 4홉에서 Cypher 표면과 `og_reach` 직접 호출의 차이는
  **179,032 페이지, 174 ms** ([`bench-50000-20260817T033001Z.json`](../../../bench/results/bench-50000-20260817T033001Z.json)).
  이 중 jsonb 조립과 정렬이 차지하는 몫은 미분리 상태다.
  기본 `work_mem = 4 MB` 에서 50,000개의 노드 jsonb(각 수백 바이트)는 디스크 정렬로 넘어갈 가능성이 높다(추정).
- **제안**: `func("count", args, distinct=true)` 에서 `args[0]` 이 `Expr::Var(v)` 이고
  `binds[v]` 가 `Bind::Node`/`Bind::Rel` 이면 `element_id_sql(v, "count")`
  ([`compile.rs:1046-1054`](../../../engine/src/cypher/compile.rs))를 써서 `count(DISTINCT n2.id)` 를 방출한다.
  `collect(DISTINCT b)` 는 전체 객체가 필요하므로 제외한다.
- **예상 효과**: 정렬 키가 jsonb → `int8`. 4홉 193 ms 중 상당 부분 (상한은 174 ms). **추정**.
- **리스크**: 사실상 없다. 노드 id는 유일하며 `_id` 는 jsonb 안에도 들어 있다.
- **검증 방법**:
  ```sql
  -- 현재 계획에서 정렬/집계 키가 무엇인지
  SELECT og_cypher_explain('benchg',
    $$MATCH (a:P {val:7})-[:K*1..4]->(b:P) RETURN count(DISTINCT b)$$, true);
  -- 수동 A/B: 같은 SQL에서 집계 인자만 바꿔 EXPLAIN (ANALYZE, BUFFERS)
  ```
  ```bash
  python3 bench/harness.py --scale 50000 --degree 20 --workload reach \
      --hops 3,4,5,6 --systems ontological,ontological_raw
  ```

### PERF-03 — 도착 노드를 타입 뷰로 재조인한다

- **심각도**: High
- **근거**: [`compile.rs:770-800`](../../../engine/src/cypher/compile.rs) (`bind_node` 가 항상 FROM 항목을 추가),
  [`compile.rs:906`](../../../engine/src/cypher/compile.rs) (`{to_alias}.id = {u}.nbr`).
- **현상**: `og_adj` 가 이웃 id를 이미 들고 있는데도, 라벨 확인을 위해 도착 노드를 타입 뷰에서 다시 읽는다.
  이것은 [`01_performance_model.md` §3](01_performance_model.md) 에서 본 대로
  **AGE와 동일하게 남아 있는 비용**이다.
- **정량적 추정**: 50,000 노드 4홉에서 Cypher 195,202 페이지 대 `og_reach` 16,170 페이지 —
  차이 179,032 페이지. 도달 노드 약 50,000개 기준 **노드당 약 3.6 페이지** (추정).
- **제안**: 도착 노드 변수가 (a) 프로퍼티 조건이 없고, (b) `RETURN`/`WHERE` 에서 라벨과 id 외에
  아무것도 쓰이지 않을 때, `CROSS JOIN {view} n2 … WHERE n2.id = u.nbr` 대신
  ```sql
  og_id_type(u4.nbr) = ANY(ARRAY[3,7,9]::int4[])
  ```
  를 방출한다. `og_id_type` 은 `IMMUTABLE, PARALLEL SAFE` 인 순수 비트 연산이다
  ([`engine/src/id.rs:73-76`](../../../engine/src/id.rs), 레이아웃은 [`id.rs:3-8`](../../../engine/src/id.rs)).
  서브타입 id 목록은 이미 컴파일 시점에 있다(`labeling::og_subtypes`).
- **예상 효과**: 4홉 193.4 ms → 25~60 ms (추정, 상한은 `og_reach` 직접 호출의 19.0 ms).
- **리스크**: **의미가 달라진다.** 현재의 조인은 도착 노드가 실제로 존재하는지도 검사한다.
  `og_adj` 에 삭제된 노드를 가리키는 이웃이 남아 있으면 새 방식은 그것을 걸러 내지 못한다.
  `og_check_integrity()` 가 이런 상태를 잡아 주는지 먼저 확인할 것.
- **검증 방법**:
  ```sql
  SELECT og_cypher_sql('benchg',
    $$MATCH (a:P {val:7})-[:K*1..4]->(b:P) RETURN count(DISTINCT b)$$);
  -- 손으로 위 SQL의 n2 조인을 og_id_type(...) 술어로 바꾼 뒤
  EXPLAIN (ANALYZE, BUFFERS) <원본>;
  EXPLAIN (ANALYZE, BUFFERS) <수정본>;
  -- 답이 같은지: 두 SQL의 결과를 EXCEPT 로 비교
  SELECT count(*) FROM og_check_integrity();   -- 매달린 인접이 있는지
  ```

### PERF-04 — `og_reach` 레벨 루프의 복사·해시·물질화

- **심각도**: Med
- **근거**: [`traverse.rs:118`](../../../engine/src/storage/traverse.rs) (`HashSet<i64>` = SipHash),
  [`:135-139`](../../../engine/src/storage/traverse.rs) (`Vec<Vec<Option<i64>>>`),
  [`:136`](../../../engine/src/storage/traverse.rs) (`frontier.clone()`, `etypes.clone()`),
  [`:157-160`](../../../engine/src/storage/traverse.rs) (전량 물질화 후 반환).
  대조군: 같은 파일의 CSR 경로는 비트맵을 쓴다 ([`:346-353`](../../../engine/src/storage/traverse.rs)).
- **현상**: 레벨마다 프론티어를 복사하고, 세그먼트마다 `Vec` 을 할당하고,
  이웃마다 `Option<i64>`(16 B)를 만들고, SipHash로 방문 여부를 본다.
- **정량적 추정**: dense 픽스처(999,784 엣지)를 한 번 훑으면 `Option<i64>` 만으로 약 16 MB의 임시 할당(추정).
  프론티어 복사는 포화 시 레벨당 최대 400 KB(추정).
  `og_reach` 71 ms와 `og_csr_reach` 4.86 ms의 14.7배 차이 중
  힙·MVCC가 아닌 이 부분의 몫은 **미분리**.
- **제안**:
  1. `frontier` 를 슬라이스로 넘겨 복사를 없앤다 (pgrx의 배열 바인딩이 허용하는 범위 확인 필요).
  2. `r.get::<Vec<i64>>` 처럼 `Option` 없는 수신이 가능한지 확인하고, 불가하면 재사용 버퍼를 둔다.
  3. `HashSet<i64>` 를 노드 id → 밀집 위치 매핑 없이 쓸 수 없다면, 최소한 `FxHashSet` 으로 교체한다.
  4. 결과를 `Vec` 에 모으지 않고 스트리밍한다(PERF-12과 함께).
- **예상 효과**: dense 6홉 71.42 ms의 10~30% (추정). 큰 그래프일수록 비중 증가.
- **리스크**: 없음(내부 구현). 단 정확성 회귀 테스트는 반드시 돌린다.
- **검증 방법**:
  ```bash
  python3 bench/csr/deep.py --db bench_csr --depths 1,2,3,4,5,6 --starts 5 --label dense
  python3 bench/csr/deep.py --db bench_sparse --depths 8,10,12,16,20 \
      --variants reach_sql,reach,csr --label sparse-deep
  ```
  `og_reach` 열의 중앙값을 변경 전후로 비교하고, `agrees: true` 를 확인한다.

### PERF-05 — `og_type_name()` 과 `eid` 언네스트가 불필요하게 방출된다

- **심각도**: Med
- **근거**: [`compile.rs:1101`](../../../engine/src/cypher/compile.rs)
  ```rust
  format!("'_type', og_type_name({alias}.type_id)"),
  ```
  [`compile.rs:901-903`](../../../engine/src/cypher/compile.rs)
  ```rust
  "… og_data.og_adj {a}, LATERAL unnest({a}.nbr, {a}.eid) AS u(nbr, eid) …"
  ```
- **현상**:
  1. `node_json(alias, Some(tid))` 는 타입이 **이미 확정된** 경로인데도 행마다
     `og_type_name(type_id)` 를 호출한다. 타입 뷰는 `type_id` 를 상수로 만들어 두므로
     ([`views.rs:105`](../../../engine/src/cypher/views.rs) 의 `{sub}::int4 AS type_id`),
     이름도 컴파일 시점 상수다. 단, 서브타입이 여러 개인 뷰에서는 브랜치별로 달라진다.
  2. 관계 변수가 없고 홉이 하나뿐이면 `eid` 는 아무 데도 쓰이지 않는데도
     `int8[]` 배열 하나를 더 디폼한다. `og_reach_sql` 은 같은 상황에서 `unnest(a.nbr)` 만 쓴다
     ([`access.sql:181`](../../../engine/sql/access.sql)).
- **정량적 추정**: `eid` 제거는 홉당 배열 디폼 작업을 절반으로 줄인다(추정).
  `og_type_name` 은 인라인 가능한 `LANGUAGE sql` 이지만 행마다 `og_catalog.type` PK 조회 1회다.
- **제안**:
  1. `view_properties` 처럼 컴파일 시점에 서브타입이 **하나뿐**임을 알 수 있으면
     `'_type', 'P'::text` 리터럴을 방출한다. 여럿이면 현행 유지.
  2. `join_rel` 에서 `rel.var.is_none() && self.rel_ids.is_empty()` 이면 `unnest({a}.nbr)` 만 방출하고,
     `u.eid` 를 참조하는 코드 경로를 비활성화한다.
- **예상 효과**: 1홉 프로젝션 비용 감소. 정량 미측정.
- **리스크**: (2)는 동형성 검사(`u.eid <> other`, [`compile.rs:908-914`](../../../engine/src/cypher/compile.rs))가
  필요한 다중 홉 패턴에서 `eid` 를 요구하므로 조건을 정확히 걸어야 한다.
- **검증 방법**: `og_cypher_sql` 로 두 형태의 SQL을 비교한 뒤
  `EXPLAIN (ANALYZE, BUFFERS)` 의 `Function Scan on unnest` 노드 시간을 비교.

### PERF-06 — `PLAN_CACHE` 에 무효화 키가 없다

- **심각도**: High (성능 + 정확성)
- **근거**: [`cypher/mod.rs:26-30,47-67`](../../../engine/src/cypher/mod.rs) (캐시),
  [`labeling.rs:172-182`](../../../engine/src/catalog/labeling.rs) (`bump_schema_version` → `drop_all_views`),
  [`bootstrap.sql:174-183`](../../../engine/sql/bootstrap.sql) (`schema_version` 테이블의 존재 이유).
- **현상**: 스키마가 바뀌면 모든 타입 뷰가 DROP 된다. 캐시된 SQL은 `og_data.v_2` 를 이름으로 참조하고,
  캐시 히트 시에는 `ensure_view` 가 호출되지 않으므로 뷰를 다시 만들 기회가 없다.
  또한 새로 승격된 프로퍼티가 캐시된 SQL의 `jsonb_build_object` 목록에 들어가지 않는다.
- **정량적 추정**: 캐시는 512개를 넘으면 **전부** 비워진다
  ([`cypher/mod.rs:61-63`](../../../engine/src/cypher/mod.rs)) — LRU가 아니다.
  513번째 질의가 512개의 컴파일을 다시 유발한다(각 SPI 9회 이상 → 약 4,600회의 카탈로그 조회, 추정).
- **제안**: 캐시 키를 `(graph, query)` 에서 `(graph, query, schema_version)` 으로 바꾸고,
  `schema_version` 은 백엔드-로컬로 캐시하되 짧은 주기로 재확인한다. 축출은 LRU로.
- **예상 효과**: 스키마 변경 후 첫 질의의 실패/구식 결과 제거, 전면 삭제로 인한 컴파일 폭풍 제거.
- **리스크**: `schema_version` 확인 자체가 SPI 1회다. PERF-10의 카탈로그 캐시와 함께 설계해야 한다.
- **검증 방법**:
  ```sql
  -- 재현: 같은 연결에서
  SELECT og_cypher('g', $$MATCH (p:Person) RETURN p LIMIT 1$$);   -- 캐시 채우기
  SELECT og_add_property('g','Person','nickname','string');       -- drop_all_views 발동
  SELECT og_cypher('g', $$MATCH (p:Person) RETURN p LIMIT 1$$);   -- 같은 연결에서 다시
  -- 오류가 나거나 nickname 이 빠져 있으면 재현된 것이다
  ```

### PERF-07 — 방문집합 BFS가 `WITH` 하나로 전면 비활성

- **심각도**: Med
- **근거**: [`compile.rs:340-342`](../../../engine/src/cypher/compile.rs)
  ```rust
  if q.clauses.iter().any(|c| matches!(c, Clause::With { .. })) { return false; }
  ```
- **현상**: `WITH` 가 **어디에** 있든, 그것이 다중도를 관측하든 안 하든 재작성이 꺼진다.
  다음은 모두 다중도를 관측할 수 없는데도 트레일 열거로 간다:
  ```cypher
  MATCH (a:P {val:7})-[:K*1..8]->(b:P) WITH DISTINCT b RETURN count(b)
  MATCH (a:P {val:7})-[:K*1..8]->(b:P) WITH b LIMIT 100 RETURN count(DISTINCT b)
  ```
- **정량적 추정**: 재작성이 적용될 때와 아닐 때의 차이가 그대로 손실이다 —
  dense 픽스처 깊이 5에서 Cypher 263.22 ms 대 강제 `og_vlp` 17,714.73 ms,
  깊이 8 이상에서는 **끝나지 않는다** ([`docs/deep-traversal.md`](../../deep-traversal.md)).
- **제안**: 보수적 완화 두 가지.
  1. `WITH` 프로젝션에도 `blind_expr` 를 적용한다. 모든 `WITH` 가 `DISTINCT` 이거나
     다중도 불감 집계만 담고, `WITH` 이후 세그먼트도 불감이면 허용.
  2. 마지막 `WITH` 보다 **뒤에 있는** 가변 길이 홉만 판정 대상으로 삼는다
     (앞쪽 홉은 `WITH` 가 다중도를 볼 수 있으므로 계속 거부).
- **예상 효과**: 위 유형의 질의가 초/무한 → 밀리초. **측정된 691배 구간에 해당**.
- **리스크**: **판정을 틀리면 답이 바뀐다.** `WITH` 는 `SKIP`/`LIMIT`/`ORDER BY` 를 가질 수 있고,
  `LIMIT` 은 행 수에 의존하므로 다중도를 관측한다. `LIMIT`/`SKIP` 이 있으면 무조건 거부해야 한다.
- **검증 방법**: [`engine/tests/sql/05_reachability.sql`](../../../engine/tests/sql/05_reachability.sql) 에
  `WITH` 케이스를 양방향으로 추가하고(그리고 PERF-19를 먼저 고쳐 단언이 실제로 검사되게 하고),
  ```sql
  SELECT og_cypher('r', $$MATCH (x:N {name:'a'})-[:E*1..12]->(y:N) WITH DISTINCT y RETURN count(y)$$)
       = og_cypher('r', $$MATCH (x:N {name:'a'})-[e:E*1..12]->(y:N) WITH DISTINCT y RETURN count(y)$$)
       AS same_answer;
  ```

### PERF-08 — `og_id_alloc` 이 타입당 직렬화를 만든다

- **심각도**: High (동시성)
- **근거**: [`storage/mod.rs:24-34`](../../../engine/src/storage/mod.rs)
  ```rust
  "INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 2)
   ON CONFLICT (type_id) DO UPDATE SET next_id = og_id_alloc.next_id + 1
   RETURNING next_id - 1"
  ```
  테이블 정의: [`bootstrap.sql:244-247`](../../../engine/sql/bootstrap.sql) — `type_id` 가 PK, 타입당 **1행**.
- **현상**: `ON CONFLICT DO UPDATE` 는 그 행에 쓰기 락을 잡고, 락은 **트랜잭션이 끝날 때까지** 유지된다.
  같은 타입에 노드를 만드는 모든 동시 트랜잭션이 한 줄에서 줄을 선다.
  한 트랜잭션이 노드 1,000개를 만들고 나중에 커밋하면, 그동안 다른 세션은 그 타입에 아무것도 만들 수 없다.
- **정량적 추정**: 동시 쓰기 처리량의 상한 ≈ `1 / (트랜잭션 지속시간)` — 세션 수와 무관 (추정).
  **측정된 적이 없다** ([`02_measured_baselines.md` §10](02_measured_baselines.md)).
- **제안**:
  1. 타입 생성 시 `CREATE SEQUENCE og_data.id_seq_<tid>` 를 만들고 `nextval()` 로 할당한다.
     `nextval` 은 트랜잭션 경계를 넘어 락을 잡지 않는다.
     `pg_extension_config_dump` 등록이 필요하다 ([`bootstrap.sql:436-447`](../../../engine/sql/bootstrap.sql) 참고).
  2. 또는 백엔드마다 블록(예: 1,000개)을 선할당해 캐시한다. 36비트 로컬 공간
     ([`id.rs:16`](../../../engine/src/id.rs) — 약 687억)에서 블록 낭비는 무시할 수 있다.
- **예상 효과**: 동시 쓰기 확장성이 타입당 직렬 → 세션 수에 비례 (추정).
- **리스크**: id에 구멍이 생긴다(현재도 롤백 시 생긴다). 백업/복원 시 시퀀스 값 보존 필요.
- **검증 방법**: **먼저 기준선을 만든다.**
  ```bash
  # 8개 세션이 동시에 같은 타입에 노드를 만드는 스크립트를 만들고
  for i in $(seq 1 8); do
    psql -p 28816 -d og -c "DO \$\$ BEGIN FOR i IN 1..2000 LOOP
      PERFORM og_create_node('default','Person', '{}'::jsonb); END LOOP; END \$\$;" &
  done; wait
  # 총 소요시간을 1세션 8,000개와 비교. 비슷하면 직렬화가 확인된 것이다.
  ```
  ```sql
  -- 대기 확인
  SELECT wait_event_type, wait_event, query FROM pg_stat_activity WHERE state = 'active';
  SELECT * FROM pg_locks WHERE relation = 'og_data.og_id_alloc'::regclass;
  ```

### PERF-09 — 인접 세그먼트 append의 쓰기 증폭

- **심각도**: High
- **근거**: [`adjacency.rs:19-44`](../../../engine/src/storage/adjacency.rs),
  [`bootstrap.sql:197-211`](../../../engine/sql/bootstrap.sql).
- **현상**:
  1. `SET nbr = a.nbr || $4::int8` 은 PostgreSQL의 MVCC 규칙상 **튜플 전체의 새 버전**을 쓴다.
     `STORAGE MAIN` 이라 TOAST로 빠지지도 않으므로, 이웃 1개 추가에 최대 4 KB가 다시 쓰인다.
  2. `WHERE … AND a.seq = (SELECT max(seq) FROM og_data.og_adj WHERE …)` —
     append 1회마다 같은 인덱스를 한 번 더 탄다.
  3. 갱신된 행이 없으면 `INSERT` 를 위해 `(SELECT max(seq) + 1 …)` 로 또 한 번 탄다
     ([`adjacency.rs:34-43`](../../../engine/src/storage/adjacency.rs)).
  4. 엣지 1개는 `'o'` 와 `'i'` 두 방향에 각각 append 하므로 이 비용이 **2배**다
     ([`storage/mod.rs:445-446`](../../../engine/src/storage/mod.rs)).
- **정량적 추정**: 세그먼트 하나를 0에서 256 이웃까지 채우는 누적 쓰기량은
  `Σ_{i=1..256} 16·i ≈ 526 KB` — 최종 4 KB 세그먼트를 만들기 위한 **약 128배의 쓰기 증폭** (추정,
  `int8` 배열 2개 기준, 튜플 헤더·WAL 오버헤드 제외).
  `fillfactor = 80` 은 4 KB 튜플에 대해 HOT 업데이트를 성립시키기에 부족하다 (추정).
- **제안**:
  1. **꼬리 세그먼트를 작게 유지한다.** 꼬리 청크 용량을 `c`(예: 32)로 두고,
     가득 차면 새 청크를 연다. 누적 쓰기량은 `8·n·c` 로 줄어 `c = 32` 면 **약 8배 감소** (추정).
     이미 있는 `og_reorganize()` ([`storage/stats.rs:117-140`](../../../engine/src/storage/stats.rs))가
     조각난 세그먼트를 병합하므로, 읽기 쪽 밀도는 유지된다.
  2. **다중 이웃 배치 append**: `nbr = a.nbr || $4::int8[]` 로 여러 엣지를 한 번에 붙인다.
     PERF-11과 함께 하면 `UNWIND … CREATE` 배치가 세그먼트당 1회 갱신이 된다.
  3. `max(seq)` 서브질의를 `ORDER BY seq DESC LIMIT 1` + `ctid` 지정 갱신으로 바꾼다.
- **예상 효과**: 쓰기당 WAL 바이트 약 8배 감소 (추정, 제안 1 기준).
- **리스크**: 세그먼트 수가 늘면 읽기에서 튜플 수가 늘어난다.
  `og_graph_stats()` 의 `packing_ratio` ([`storage/stats.rs:79`](../../../engine/src/storage/stats.rs))로
  감시하고 `og_reorganize()` 주기를 정해야 한다.
- **검증 방법**:
  ```sql
  -- 기준선: 엣지 1,000개를 만들면서 WAL 증가량을 잰다
  SELECT pg_current_wal_lsn() AS before \gset
  DO $$ DECLARE a int8; b int8; BEGIN
    SELECT id INTO a FROM og_data.og_node LIMIT 1;
    SELECT id INTO b FROM og_data.og_node OFFSET 1 LIMIT 1;
    FOR i IN 1..1000 LOOP PERFORM og_create_edge('default','KNOWS', a, b); END LOOP;
  END $$;
  SELECT pg_wal_lsn_diff(pg_current_wal_lsn(), :'before') AS wal_bytes;
  -- 엣지당 바이트 = wal_bytes / 1000. 변경 전후를 비교한다.
  SELECT og_graph_stats('default') -> 'adjacency';
  ```

### PERF-10 — 카탈로그 캐시가 없다: 노드 1개에 SPI 9회

- **심각도**: High
- **근거**: [`03_hot_paths.md` §6](03_hot_paths.md) 의 호출 표.
  개별 조회: [`types.rs:112-119`](../../../engine/src/catalog/types.rs) (`graph_id`),
  [`:121-127`](../../../engine/src/catalog/types.rs) (`try_type_id`),
  [`:286-294`](../../../engine/src/catalog/types.rs) (`type_kind`),
  [`:616-622`](../../../engine/src/catalog/types.rs) (`storage_table`),
  [`storage/mod.rs:161-177`](../../../engine/src/storage/mod.rs) (`plan_props`),
  [`storage/mod.rs:90-100`](../../../engine/src/storage/mod.rs) (`declare_new_props` 의 이름 조회).
  각각 [`spiu.rs:15-23`](../../../engine/src/spiu.rs) 를 통해 **독립적인 `Spi::connect`** 다.
- **현상**: 노드 1개 생성 = SPI 9회, 그중 6회가 순수 카탈로그 조회.
  `declare_new_props` 는 **새 프로퍼티가 하나도 없어도** 그래프/타입 이름을 매번 조회한다
  ([`storage/mod.rs:90-103`](../../../engine/src/storage/mod.rs) — `obj` 가 비어 있지 않으면 무조건 실행).
  읽기 경로도 같다: 컴파일 1회에 SPI 9회, 그중 두 쌍은 **완전히 동일한 질의의 중복 실행**
  ([`03_hot_paths.md` §2](03_hot_paths.md) 의 #2/#4, #3/#5).
- **정량적 추정**: `docs/benchmark.md` 가 "the Cypher engine costs 7.8× over the raw storage path"
  라고 지목한 33.86 ms 대 4.33 ms 의 차이 중 SPI 왕복의 몫은 미분리.
  콜드 백엔드 페이지 수 ~1,170의 유력한 원인이기도 하다 ([`02_measured_baselines.md` §8](02_measured_baselines.md)).
- **제안**:
  1. 백엔드-로컬 타입 카탈로그 캐시: `(graph_id, type_id) → {name, kind, storage_table, subtypes, properties}`.
     `og_catalog.schema_version` 을 무효화 키로 쓴다(PERF-06과 같은 키).
  2. `declare_new_props` 는 **새 키가 실제로 있을 때만** 이름을 조회하도록 순서를 바꾼다
     (기존 프로퍼티 목록과 먼저 대조).
  3. 컴파일 경로에서 같은 `try_type_id`/`view_exists` 를 두 번 부르지 않도록 컴파일러 안에 지역 맵을 둔다.
- **예상 효과**: 노드 1개 SPI 9 → 3회, 읽기 컴파일 SPI 9 → 1~2회 (추정).
- **리스크**: 캐시 일관성. 같은 트랜잭션에서 DDL을 실행하면 즉시 무효화해야 한다.
- **검증 방법**:
  ```sql
  -- SPI 호출 수를 직접 세는 방법이 없으므로 문 수로 대신한다
  CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
  SELECT pg_stat_statements_reset();
  SELECT og_create_node('default','Person','{"name":"x"}');
  SELECT calls, query FROM pg_stat_statements ORDER BY calls DESC LIMIT 20;
  ```
  카탈로그 조회 문들의 `calls` 합이 개선 후 줄어야 한다.

### PERF-11 — 벌크 로드 경로가 없다

- **심각도**: High
- **근거**: `grep -rn "COPY \|copy_in\|CopyIn" engine/src/` → 히트 없음.
  Cypher 쓰기는 [`cypher/mod.rs:236-243`](../../../engine/src/cypher/mod.rs) 의 per-env 루프에서
  행마다 `create_node_inner` / `create_edge_inner` 를 부른다.
- **현상**: `UNWIND $rows AS r CREATE (n:P {name: r.name})` 에 10,000행을 넣으면
  SPI 왕복이 약 90,000회 발생한다(PERF-10의 9회 × 10,000, 추정).
  벤치의 "124,580 / 161,852 edges/s" 는 하네스가 `og_data.*` 에 직접 `INSERT … SELECT` 한 값이지
  이 경로가 아니다 ([`bench/harness.py:322-355`](../../../bench/harness.py)).
- **정량적 추정**: **미측정.** SQL 직접 경로의 1/10~1/50 로 추정하지만 근거는 SPI 왕복 수뿐이다.
- **제안**:
  1. `UNWIND` + `CREATE` 조합을 감지해 타입별로 묶고,
     `INSERT INTO og_data.n_<tid> (id, …) SELECT … FROM jsonb_array_elements($1)` 한 문장으로 컴파일한다.
  2. 인접 갱신도 배치로: `INSERT INTO og_data.og_adj … SELECT … GROUP BY src, chunk`
     (하네스가 [`harness.py:342-355`](../../../bench/harness.py) 에서 쓰는 바로 그 SQL이 참고 구현이다).
  3. id 할당도 배치로 (PERF-08의 시퀀스라면 `nextval` 을 N번, 블록 선할당이라면 1번).
- **예상 효과**: 배치 쓰기 처리량 한 자릿수 배 (추정).
- **리스크**: `MERGE`, 트리거(`og_capture_history`), role 검증, `declare_new_props` 가
  행 단위 의미를 갖는다. 배치 경로는 이들이 없을 때만 적용해야 한다.
- **검증 방법**: 먼저 기준선을 만든다.
  ```sql
  \timing on
  SELECT og_cypher('default',
    $$UNWIND range(1,10000) AS i CREATE (n:Person {name: 'p' + toString(i)})$$);
  ```
  그리고 하네스의 SQL 직접 경로와 비교한다.

### PERF-12 — 결과가 스트리밍되지 않고 이중 변환된다

- **심각도**: High
- **근거**: [`cypher/mod.rs:145-152`](../../../engine/src/cypher/mod.rs)
  ```rust
  table.filter_map(|r| r.get::<JsonB>(1).ok().flatten().map(|j| j.0)).collect()
  ```
  그리고 [`cypher/mod.rs:108`](../../../engine/src/cypher/mod.rs)
  ```rust
  SetOfIterator::new(rows.into_iter().map(JsonB))
  ```
- **현상**: 모든 행을 `Vec<serde_json::Value>` 에 물질화한 뒤에야 첫 행이 나간다.
  행마다 jsonb Datum → `serde_json::Value` 역직렬화 → 다시 `JsonB` 직렬화가 일어난다.
  `LIMIT` 이 없는 질의는 결과 전체를 백엔드 메모리에 올린다.
- **정량적 추정**: **미측정.** 벤치의 모든 질의가 `count(...)` 라 한 행만 낸다 —
  이 비용을 재는 워크로드가 저장소에 없다.
- **제안**: SPI 커서(`Spi::connect` + `client.open_cursor`)로 스트리밍하고,
  jsonb Datum을 `serde_json` 을 거치지 않고 그대로 통과시킨다.
- **예상 효과**: 큰 결과의 메모리 상한 제거, 첫 행 지연 감소. 정량은 워크로드를 만든 뒤에.
- **리스크**: `og_cypher_stats()` 가 세는 시점, `audit()` 의 `rows` 계산 시점이 바뀐다.
- **검증 방법**: 먼저 워크로드를 추가한다 —
  ```sql
  \timing on
  SELECT count(*) FROM og_cypher('benchg', $$MATCH (a:P) RETURN a$$);  -- 50,000행
  ```
  변경 전후로 `pg_backend_memory_contexts` 의 피크와 지연을 비교한다.

### PERF-13 — 읽기 질의마다 `og_audit` INSERT

- **심각도**: Med
- **근거**: [`cypher/mod.rs:107`](../../../engine/src/cypher/mod.rs), [`:122-135`](../../../engine/src/cypher/mod.rs).
  테이블: [`bootstrap.sql:380-390`](../../../engine/sql/bootstrap.sql) — `bigserial` PK + `at DESC` 인덱스.
- **현상**: 모든 `og_cypher()` 호출이 감사 테이블에 한 행을 넣는다. 읽기도 예외가 아니다.
  결과: (1) 질의당 고정 비용, (2) `og_audit` 무제한 증가(정리 정책 없음),
  (3) **읽기 복제본에서 `og_cypher` 를 쓸 수 없다** — spec 007이 read replica를 지원한다고 말하는 것과 충돌.
- **정량적 추정**: 인덱스 1개짜리 테이블에 대한 단일 행 INSERT는 0.05~0.15 ms (추정).
  1홉 질의의 측정 지연 2.104 ms 대비 2~7%.
- **제안**: `og_catalog.setting` 의 키로 on/off + 샘플링 비율을 두고, 기본은 쓰기 질의만 기록.
  또는 `og_audit` 를 `UNLOGGED` 로 만들고 보존 기간을 둔다.
- **예상 효과**: 질의당 고정 비용 제거, 복제본 사용 가능.
- **리스크**: spec 008 FR-027의 감사 요구사항이 약해진다. 기본값을 어디에 둘지는 제품 결정.
- **검증 방법**:
  ```sql
  \timing on
  SELECT og_cypher('benchg', $$MATCH (a:P {val:7}) RETURN a.val$$);   -- 현재
  -- 감사 INSERT를 끈 빌드에서 같은 질의를 반복하고 중앙값 비교
  SELECT count(*), pg_size_pretty(pg_total_relation_size('og_data.og_audit')) FROM og_data.og_audit;
  ```

### PERF-14 — 콜드 백엔드 첫 호출의 고정 비용 (~1,170 페이지)

- **심각도**: Med (진단)
- **근거**: [`02_measured_baselines.md` §8](02_measured_baselines.md) 의 표 —
  5,000 / 50,000 / 250,000 노드에서 `ontological` 의 `prop_scan` 페이지가 1,170 / 1,173 / 1,177.
  측정 방식: [`harness.py:183-206`](../../../bench/harness.py) → [`harness.py:54-66`](../../../bench/harness.py).
- **현상**: 데이터 크기와 무관한 상수. 데이터 스캔이 아니다.
  **원인 미확인.** 후보: 컴파일 경로의 카탈로그 SPI 9회 이상(PERF-10),
  `og_audit` INSERT(PERF-13), pgrx 확장 첫 로드.
- **정량적 추정**: `ontological` 의 모든 페이지 수에서 약 1,170을 빼야 실제 질의 비용이 된다 —
  1홉 1,742 → 약 572, `prop_scan` 1,174 → 약 4 (추정). 그렇게 보면 저장 구조의 우위가 지금 표보다 크다.
- **제안**: 원인을 먼저 분리하고(아래), PERF-10/PERF-13으로 제거한 뒤,
  하네스의 `buffers_read` 를 지연과 같은 세션에서 측정하도록 바꾼다.
- **예상 효과**: 공개 벤치의 페이지 열이 실제 저장 비용을 반영하게 된다.
- **리스크**: 없음 (측정 방법 변경).
- **검증 방법**:
  ```sql
  -- 새 연결에서
  EXPLAIN (ANALYZE, BUFFERS) SELECT og_cypher('benchg', $$MATCH (a:P) WHERE a.val < 100 RETURN count(a)$$);
  -- 같은 연결에서 곧바로 한 번 더 (PLAN_CACHE 히트)
  EXPLAIN (ANALYZE, BUFFERS) SELECT og_cypher('benchg', $$MATCH (a:P) WHERE a.val < 100 RETURN count(a)$$);
  -- 두 번째가 크게 작으면 원인은 컴파일 경로다.
  -- 감사만 분리하려면 og_audit 을 임시로 비우고 크기 변화를 본다.
  ```

### PERF-15 — `og_reach_sql` 이 배선되어 있지 않다

- **심각도**: Med
- **근거**: [`access.sql:169-190`](../../../engine/sql/access.sql) 에 함수는 있으나
  [`compile.rs:865-873`](../../../engine/src/cypher/compile.rs) 은 `og_reach` 와 `og_vlp` 둘 중에서만 고른다.
  `docs/deep-traversal.md` 도 *"`og_reach_sql` is not wired into anything"* 이라고 명시한다.
- **현상**: 프론티어가 얇고 깊이가 큰 그래프에서 `og_reach` 가 SPI 레벨 왕복 때문에 진다.
- **정량적 추정 (측정)**: chain-1M 100,000홉 —
  `og_reach` 1,015.9 ms 대 `og_reach_sql` 154.5 ms (**6.6배**).
  1,000홉 — 8.817 ms 대 1.404 ms (6.3배).
  출처: [`bench/csr/results/deep-chain-20260817T053710Z.json`](../../../bench/csr/results/deep-chain-20260817T053710Z.json).
  반대로 dense 6홉에서는 `og_reach` 71.4 ms 대 `og_reach_sql` 426.5 ms (`og_reach` 가 6배 우세).
- **제안**: 세 번째 선택지를 만들되, 판정 기준이 없다는 것이 정확히 문제다.
  `docs/deep-traversal.md` 는 *"a third automatic choice would have to be made from a statistic that says
  whether frontiers overlap, and no such statistic is available for free"* 라고 적었다. 두 가지 대안:
  1. **런타임 적응**: `og_reach` 안에서 처음 몇 레벨의 프론티어 크기를 보고,
     계속 작으면(예: 3레벨 연속 |frontier| < 16) 나머지를 하나의 재귀 CTE로 넘긴다.
  2. **정적 힌트**: `maxhop` 이 크고(예: > 64) 평균 차수가 1에 가까우면 `og_reach_sql` 을 고른다.
     `prefer_reachability` 가 이미 읽는 `reltuples` 두 개로 판정 가능하다.
- **예상 효과**: 사슬형 그래프에서 6.6배 (**측정된 비율**).
- **리스크**: 순환 그래프에서 `og_reach_sql` 은 `O(k·|V|)` 이므로 오판하면 크게 진다
  (dense 20홉: 3,659 ms 대 69 ms — **53배**).
- **검증 방법**:
  ```bash
  psql -d bench_chain -v shape=chain -v nodes=1000000 -f bench/csr/gen_shape.sql
  python3 bench/csr/deep.py --db bench_chain --depths 10,100,1000,10000,100000 --label chain
  python3 bench/csr/deep.py --db bench_csr --depths 7,8,10,16,20 \
      --variants reach_sql,reach,csr --label dense-deep
  ```
  두 픽스처에서 **동시에** 이기는 판정인지 확인한다.

### PERF-16 — Bolt RUN 1회에 PostgreSQL 왕복 3회

- **심각도**: Med
- **근거**: [`bolt/src/session.rs:267`](../../../bolt/src/session.rs) (`is_write` → `og_cypher_check`),
  [`:283-289`](../../../bolt/src/session.rs) (`og_cypher_columns`),
  [`:291-299`](../../../bolt/src/session.rs) (`og_cypher`).
- **현상**: 하나의 Bolt RUN에 대해 왕복 3회, Cypher 렉싱·파싱 4~5회
  (게이트웨이 3회 + `og_cypher` 내부 1회 + `PLAN_CACHE` 미스 시 1회 더).
- **정량적 추정**: 파싱 자체의 비용은 미측정.
  왕복 2회는 로컬 소켓에서도 각각 프로토콜 바닥값(psql 기준 0.17~0.19 ms) 수준이다 —
  질의당 **약 0.4 ms 추가** (추정).
- **제안**: `og_cypher` 가 컬럼 순서와 write 여부를 결과에 함께 싣도록 하거나,
  `og_cypher_run(graph, query, params) → (columns text[], rows jsonb[])` 를 새로 만든다.
  `EXPLAIN`/`PROFILE` 접두어 처리([`session.rs:264-279`](../../../bolt/src/session.rs))는 유지.
- **예상 효과**: RUN당 왕복 3 → 1, 파싱 4~5 → 1~2 (추정).
- **리스크**: `og_cypher_check` 의 구문 오류 조기 보고가 사라지지 않도록 오류 경로를 유지해야 한다.
- **검증 방법**:
  ```sql
  CREATE EXTENSION IF NOT EXISTS pg_stat_statements;
  SELECT pg_stat_statements_reset();
  -- Neo4j 드라이버로 질의 100회 실행 후
  SELECT calls, query FROM pg_stat_statements WHERE query LIKE '%og_cypher%' ORDER BY calls DESC;
  ```
  `og_cypher_check` / `og_cypher_columns` 의 `calls` 가 100이면 재현된 것이다.

### PERF-17 — PackStream 경로가 레코드마다 clone·flush·시스템 콜

- **심각도**: Med
- **근거**:
  - [`session.rs:353`](../../../bolt/src/session.rs) — `let rec = self.pending[self.cursor + i].clone();`
  - [`packstream.rs:254-262`](../../../bolt/src/packstream.rs) — 메시지마다 `Vec::new()` + `w.flush()`
  - [`session.rs:336`](../../../bolt/src/session.rs) — `stream: &mut TcpStream` (버퍼 없음)
  - [`session.rs:291-299`](../../../bolt/src/session.rs) — `og_cypher(…)::text` 캐스트
  - [`packstream.rs:80`](../../../bolt/src/packstream.rs) — 맵 키마다 `k.clone()`
- **현상**: 행 하나가 나가기까지의 표현 변환:
  `jsonb(바이너리) → text → serde_json::Value → packstream::Value → Vec<u8>`.
  그리고 레코드마다 (a) `Value` 트리 깊은 복사, (b) 새 `Vec` 할당,
  (c) 청크 헤더 `write_all` + 본문 `write_all` + 종료자 `write_all` + `flush()` = **최소 3회의 `write()` 시스템 콜**.
- **정량적 추정**: 10,000행 결과에서 시스템 콜 약 30,000회, 깊은 복사 10,000회 (추정).
  **미측정** — Bolt 처리량 벤치가 없다.
- **제안**:
  1. `stream` 을 `BufWriter<TcpStream>` 으로 감싸고, `write_message` 에서 `flush()` 를 빼서
     PULL 루프의 마지막이나 SUCCESS 직전에만 flush 한다.
  2. `self.pending` 을 `into_iter()`/`drain()` 으로 소비해 `clone()` 을 없앤다
     (`has_more` 처리를 위해 `VecDeque` 또는 커서 + `std::mem::take`).
  3. `::text` 캐스트를 없애고 `postgres` 크레이트의 `Json`/바이너리 수신을 쓴다.
  4. 궁극적으로는 `og_cypher` 에서 PackStream을 직접 만들 수도 있으나 범위가 크다.
- **예상 효과**: 대량 결과의 시스템 콜 1/N, 할당 절반 (추정).
- **리스크**: Bolt는 메시지 경계에서 클라이언트가 응답을 기다릴 수 있으므로,
  flush 시점을 잘못 잡으면 **교착**이 난다. RECORD 뒤 SUCCESS 전에 반드시 flush.
- **검증 방법**:
  ```bash
  strace -f -c -e trace=write ./bolt/target/release/ontological-bolt   # 변경 전후 write 호출 수
  # 처리량: Neo4j 파이썬 드라이버로 50,000행을 받아 소요시간 측정
  ```

### PERF-18 — Studio 서버가 전체 결과를 메모리에 적재

- **심각도**: High
- **근거**: [`portal/server/index.js:188-196`](../../../portal/server/index.js) (`pool.connect` + `client.query`),
  [`:195`](../../../portal/server/index.js) (`r.rows.map`),
  [`:208`](../../../portal/server/index.js) → [`:317-342`](../../../portal/server/index.js) (`projectGraph` 전 행 재귀 순회),
  [`:39-45`](../../../portal/server/index.js) (`JSON.stringify` + `content-length`).
- **현상**: 스트리밍이 없다. 행 상한이 없다. `statement_timeout` 이 없다.
  피크 메모리 = pg 행 객체 + `rows` 배열 + 노드/엣지 `Map` + JSON 문자열 ≈ 결과의 3~4배 (추정).
  같은 요청에서 `og_cypher_sql` 을 한 번 더 호출해 컴파일을 반복한다
  ([`index.js:197-202`](../../../portal/server/index.js)).
- **정량적 추정**: **미측정.** `MATCH (n) RETURN n` 을 50,000 노드 그래프에 던지면
  Node 기본 힙(약 1.5 GB)을 넘길 수 있다(추정).
- **제안**:
  1. `pg-cursor` / `pg-query-stream` 으로 스트리밍하고 NDJSON 또는 청크 응답으로 내보낸다.
  2. 서버 측 행 상한(예: 5,000)과 "잘렸음" 플래그를 응답에 넣는다.
  3. 풀 연결마다 `SET statement_timeout` 을 건다.
  4. `og_cypher_sql` 은 사용자가 요청했을 때만 호출한다.
- **예상 효과**: OOM 제거, 첫 화면 지연 감소.
- **리스크**: 프론트엔드([`portal/web/app.js`](../../../portal/web/app.js))가 부분 결과와
  "잘림" 상태를 표시하도록 바뀌어야 한다.
- **검증 방법**:
  ```bash
  node --max-old-space-size=256 portal/server/index.js &
  curl -s -X POST localhost:7474/api/cypher \
    -H 'content-type: application/json' \
    -d '{"graph":"benchg","query":"MATCH (a:P) RETURN a"}' -o /dev/null -w '%{time_total}\n'
  # 변경 전: OOM 또는 수 초. 변경 후: 상한만큼만.
  ```

### PERF-19 — 회귀 스위트가 단언 값을 검사하지 않는다

- **심각도**: High
- **근거**: [`tests/run.sh:14-36`](../../../tests/run.sh) — `grep -c '^ERROR'` 만 본다.
  `engine/tests/pg_regress/expected/` 에는 `setup.out` 하나뿐.
  대상: [`engine/tests/sql/05_reachability.sql:74-96`](../../../engine/tests/sql/05_reachability.sql) 의
  `count_distinct_uses_reach` / `plain_count_keeps_vlp` / `path_variable_keeps_vlp` /
  `rel_variable_keeps_vlp` / `shallow_keeps_vlp` 등.
- **현상**: 이 불리언들이 `f` 를 내도 테스트는 통과한다.
  `docs/deep-traversal.md` 의 *"The regression suite asserts every one of these cases"* 는
  파일에 대해서는 참이지만 **러너에 대해서는 거짓**이다.
  덧붙여 같은 문서의 전환 규칙 서술이 코드와 두 군데 어긋나 있다
  ([`04_deep_traversal_mechanics.md` §3](04_deep_traversal_mechanics.md)).
- **정량적 추정**: 해당 없음 (게이트 문제).
- **제안**:
  1. 각 단언을 `DO $$ BEGIN IF NOT (…) THEN RAISE EXCEPTION '…'; END IF; END $$;` 로 바꾼다.
     `run.sh` 가 `ERROR` 를 세므로 **러너를 바꾸지 않고도** 즉시 실패하게 만들 수 있다.
  2. 또는 `pg_regress` 로 옮겨 기대 출력을 관리한다.
  3. `docs/deep-traversal.md` 의 `Σ degreeⁱ > |V|` / "Depth ≥ 12" 서술을 코드에 맞게 정정한다.
- **예상 효과**: 전환 판정 회귀가 실제로 잡힌다.
- **리스크**: 기대 출력 유지 비용. 방법 1은 그 비용이 없다.
- **검증 방법**:
  ```bash
  # 일부러 망가뜨려 본다: compile.rs 의 WALKS 를 10_000_000.0 으로 바꾸고 빌드 후
  bash tests/run.sh
  # 현재는 "ok" 가 나온다. 고친 뒤에는 05_reachability.sql 이 FAIL 이어야 한다.
  ```

### PERF-20 — `minhop > 1` 에서 재작성이 다른 답을 낸다

- **심각도**: High (정확성, 성능 재작성이 원인)
- **근거**: [`traverse.rs:143-153`](../../../engine/src/storage/traverse.rs) (방문집합이 최단 거리를 강제),
  [`compile.rs:865`](../../../engine/src/cypher/compile.rs) (`prefer_reachability(max)` — `min` 을 보지 않음),
  [`compile.rs:874-876`](../../../engine/src/cypher/compile.rs) (`{min}` 을 그대로 전달),
  [`access.sql:154-155`](../../../engine/sql/access.sql) (`og_vlp` 는 `depth >= minhop` 인 모든 트레일).
- **현상**: `og_vlp` 는 **길이가 `minhop..maxhop` 인 트레일**로 닿는 노드를,
  `og_reach` 는 **최단 거리가 `minhop..maxhop` 인 노드**를 낸다.
  깊이 `< minhop` 에서 처음 도달한 노드는 `visited` 에 들어가 영영 출력되지 않는다.
- **정량적 추정**: 해당 없음. 반례:
  ```
  a→b, b→c, a→c
  MATCH (a)-[:K*2..2]->(x) RETURN count(DISTINCT x)
  og_vlp  → 1 (a→b→c)
  og_reach → 0 (c 는 깊이 1에서 방문)
  ```
- **제안**: 즉시 조치로 `compile.rs:865` 의 조건에 `min <= 1` 을 추가한다.
  장기적으로는 `og_reach` 에 "최소 깊이 제약이 있는 도달성"을 별도 의미로 구현하거나
  (레벨별 방문집합을 `minhop` 까지는 유지하지 않는 변형),
  `og_reach_sql` 처럼 모든 깊이를 생산하고 필터하는 방식을 쓴다.
- **예상 효과**: 정확성 확보. 성능은 `minhop > 1` 질의에서 열거 경로로 돌아가므로 손해.
- **리스크**: `*2..8` 같은 질의가 다시 느려진다. 그것이 옳은 상태다.
- **검증 방법**:
  ```sql
  SELECT og_create_graph('m');
  SELECT og_create_type('m','N','entity'); SELECT og_add_property('m','N','name','string');
  SELECT og_create_type('m','E','relation');
  SELECT og_cypher('m', $$CREATE (:N {name:'a'})$$);
  SELECT og_cypher('m', $$CREATE (:N {name:'b'})$$);
  SELECT og_cypher('m', $$CREATE (:N {name:'c'})$$);
  SELECT og_cypher('m', $$MATCH (x:N {name:'a'}),(y:N {name:'b'}) CREATE (x)-[:E]->(y)$$);
  SELECT og_cypher('m', $$MATCH (x:N {name:'b'}),(y:N {name:'c'}) CREATE (x)-[:E]->(y)$$);
  SELECT og_cypher('m', $$MATCH (x:N {name:'a'}),(y:N {name:'c'}) CREATE (x)-[:E]->(y)$$);
  ANALYZE;
  -- 재작성 쪽과 강제 열거 쪽의 답이 같아야 한다
  SELECT og_cypher('m', $$MATCH (x:N {name:'a'})-[:E*2..2]->(y:N)  RETURN count(DISTINCT y)$$) AS rewritten,
         og_cypher('m', $$MATCH (x:N {name:'a'})-[e:E*2..2]->(y:N) RETURN count(DISTINCT y)$$) AS forced;
  SELECT og_cypher_sql('m', $$MATCH (x:N {name:'a'})-[:E*2..2]->(y:N) RETURN count(DISTINCT y)$$)
         LIKE '%og_reach(%' AS took_reach;
  ```

### PERF-21 — `ROWS` 고정 추정치가 계획을 왜곡한다

- **심각도**: Med
- **근거**: [`access.sql:140`](../../../engine/sql/access.sql) (`og_vlp … ROWS 100`),
  [`:171`](../../../engine/sql/access.sql) (`og_reach_sql … ROWS 1000`),
  [`:192-197`](../../../engine/sql/access.sql) (`ALTER FUNCTION og_reach … ROWS 100`).
- **현상**: [`05_planner_interaction.md` §3](05_planner_interaction.md) 의 표.
  계획에 실제로 반영되는 두 함수가 가장 크게 빗나간다:
  깊이 6·차수 20에서 `og_vlp` 추정 100 대 실제 약 6,700만 (**약 67만 배**),
  `og_reach` 추정 100 대 실제 최대 50,000 (**500배**).
- **정량적 추정**: 상위 조인(`n2.id = w.node`)이 100행짜리 입력으로 계획되므로
  중첩 루프가 선택되고, 실제 50,000행이 들어오면 50,000회의 인덱스 프로브가 된다.
  이것이 4홉 195,202 페이지의 유력한 경로다(추정, PERF-03과 같은 현상의 다른 면).
- **제안**: `og_reach` 에 PostgreSQL의 **planner support function**
  (`CREATE FUNCTION … SUPPORT`)을 붙여, `maxhop` 인자와 `og_node.reltuples` 로
  `min(reltuples, Σ degreeⁱ)` 를 반환하게 한다.
  `og_vlp` 도 같은 방식으로 `Σ degreeⁱ` 를 반환하게 한다.
  (지원 함수는 Rust로 작성 가능하며 `og_reach` 가 이미 Rust다.)
- **예상 효과**: 상위 조인이 해시/머지로 바뀔 여지. 정량 미측정.
- **리스크**: 추정이 커지면 다른 계획(해시 조인 + 큰 `work_mem`)이 오히려 나쁠 수 있다.
  PERF-03을 먼저 적용하면 이 조인 자체가 사라지므로 **순서를 PERF-03 → PERF-21 로 두는 것이 낫다.**
- **검증 방법**:
  ```sql
  SELECT og_cypher_explain('benchg',
    $$MATCH (a:P {val:7})-[:K*1..6]->(b:P) RETURN count(DISTINCT b)$$, true);
  -- plan 안에서 og_reach 노드의 "Plan Rows" 와 "Actual Rows" 비교
  ```

### PERF-22 — 벡터 검색이 뷰 위에서 실행되고 `ef_search` 를 설정하지 않는다

- **심각도**: Med
- **근거**: [`vector/mod.rs:112`](../../../engine/src/vector/mod.rs) (`ensure_view` — `UNION ALL` 뷰),
  [`:126-132`](../../../engine/src/vector/mod.rs) (`ORDER BY v.{col} {op} $1 LIMIT {k}`),
  [`:115-118`](../../../engine/src/vector/mod.rs) (`filter` 텍스트 보간),
  [`:56-64`](../../../engine/src/vector/mod.rs) (HNSW 인덱스는 **구체 테이블**에 만들어진다),
  [`:113,127`](../../../engine/src/vector/mod.rs) (`og_node_json(v.id)` 를 결과 행마다 호출).
- **현상**:
  1. 인덱스는 `og_data.n_<sub>` 에 있고 질의는 `og_data.v_<tid>` 뷰를 스캔한다.
     서브타입이 하나면 뷰가 브랜치 하나라 인덱스가 살아 있을 가능성이 높지만,
     여럿이면 `Append`/`MergeAppend` 를 거쳐야 하고, 컬럼이 없는 브랜치는
     `NULL::vector(N)` 상수라 인덱스 스캔이 불가능하다
     ([`views.rs:114`](../../../engine/src/cypher/views.rs)).
  2. `hnsw.ef_search` 를 설정하지 않는다. pgvector 기본값 40이므로 `k > 40` 이면 recall이 떨어진다.
  3. `filter` 는 `AND ({f})` 로 붙는다. pgvector의 HNSW는 (반복 스캔 기능이 없는 버전에서)
     인덱스가 낸 후보를 **뒤에서** 거른다. 선택도가 높은 필터는 `k` 개보다 적은 결과를 내거나
     플래너가 시퀀셜 스캔으로 되돌아간다.
     모듈 주석([`vector/mod.rs:9-12`](../../../engine/src/vector/mod.rs))이 주장하는
     *"filter push-down structural rather than aspirational"* 은 **같은 릴레이션 위에 있다**는 뜻이지,
     HNSW가 필터를 인덱스 내부에서 처리한다는 뜻이 아니다.
  4. 반환 행마다 `og_node_json` (plpgsql + 동적 `EXECUTE`) — PERF-30.
- **정량적 추정**: **미측정.** 벡터 벤치가 저장소에 없다. pgvector 버전도 고정되어 있지 않다
  ([`docker/Dockerfile.dev:2`](../../../docker/Dockerfile.dev) — `pgvector/pgvector:pg16` 태그).
- **제안**:
  1. `concrete_tables(tid, is_edge)` 가 테이블 하나만 내면 뷰 대신 그 테이블을 직접 쓴다.
  2. `SET LOCAL hnsw.ef_search = greatest(40, k * 4)` 를 질의 앞에 붙인다.
  3. `filter` 가 주어졌을 때는 `EXPLAIN` 으로 인덱스 사용 여부를 확인할 수 있게
     `og_vector_search_explain` 같은 진단 함수를 추가하거나, `og_cypher_explain` 처럼 계획을 노출한다.
  4. `og_node_json(v.id)` 대신 뷰 컬럼으로 jsonb를 조립한다(`compile.rs:1095-1113` 과 같은 방식).
- **예상 효과**: HNSW 인덱스 유지 + recall 제어. 정량은 벤치를 만든 뒤에.
- **리스크**: pgvector 버전에 따라 동작이 다르다. 버전을 고정하고 문서화해야 한다.
- **검증 방법**:
  ```sql
  SELECT extversion FROM pg_extension WHERE extname = 'vector';
  -- 계획 확인 (뷰 이름은 og_data.v_<tid>)
  EXPLAIN (ANALYZE, BUFFERS)
    SELECT v.id, (v.p_embedding <=> '[...]'::vector) FROM og_data.v_7 v
     WHERE v.p_embedding IS NOT NULL ORDER BY 2 LIMIT 10;
  -- Index Scan using hnsw_... 인지 Seq Scan 인지
  -- recall: og_vector_search 와 og_vector_search_exact 의 상위 k 교집합 크기
  ```

### PERF-23 — `og_hybrid_search` 가 근접도를 `og_vlp` 로 계산한다

- **심각도**: Med
- **근거**: [`vector/mod.rs:251-256`](../../../engine/src/vector/mod.rs)
  ```rust
  "prox AS (SELECT node, min(depth) AS hops FROM og_vlp({a}::int8, NULL, 'b'::\"char\", 0, 3) GROUP BY node)"
  ```
- **현상**: 양방향(`'b'`), 전 관계 타입(`NULL`), 깊이 3의 **트레일 열거**다.
  `min(depth)` 로 그룹핑하므로 **다중도는 관측되지 않는다** — 정확히 `og_reach` 가 답할 수 있는 질문이다.
- **정량적 추정**: 평균 차수 20의 양방향은 실효 차수 40이므로
  `Σ_{i=1..3} 40ⁱ = 65,640` 개의 워크를 만들어 최대 `|V|` 개의 답을 낸다(추정).
  단방향 dense 픽스처의 깊이 3 측정으로는 `og_vlp` 6.85 ms 대 `og_reach` 1.61 ms — **4.3배**
  ([`bench/csr/results/deep-dense-20260817T021522Z.json`](../../../bench/csr/results/deep-dense-20260817T021522Z.json)).
  양방향에서는 격차가 더 클 것으로 추정.
- **제안**: `og_vlp(...)` 를 `og_reach(...)` 로 바꾼다. `og_reach` 는 이미 각 노드를 최초 깊이로 한 번만 내므로
  `min(depth)` 그룹핑도 필요 없다 (다만 `minhop = 0` 이므로 PERF-20의 문제도 없다).
- **예상 효과**: 4.3배 이상 (측정된 비율에서 유도).
- **리스크**: 없음 — 출력 집합과 `hops` 값이 같다(`minhop = 0` 이므로 §PERF-20의 함정 밖).
- **검증 방법**:
  ```sql
  -- 답이 같은지 먼저
  SELECT count(*) FROM (
    SELECT node, min(depth) AS h FROM og_vlp(:anchor, NULL, 'b'::"char", 0, 3) GROUP BY node
    EXCEPT
    SELECT node, depth FROM og_reach(:anchor, NULL, 'b'::"char", 0, 3)) x;
  \timing on
  SELECT count(*) FROM og_vlp(:anchor, NULL, 'b'::"char", 0, 3);
  SELECT count(*) FROM og_reach(:anchor, NULL, 'b'::"char", 0, 3);
  ```

### PERF-24 — 병렬 질의를 사실상 쓰지 못한다

- **심각도**: Med
- **근거**: [`cypher/mod.rs:83`](../../../engine/src/cypher/mod.rs) — `#[pg_extern]` 에 parallel 속성 없음
  (pgrx가 `PARALLEL` 절을 생성하지 않으면 PostgreSQL 기본값은 `PARALLEL UNSAFE`).
  `grep -rn "#\[pg_extern" engine/src | grep -v parallel | wc -l` → **61** (전체 78개 중).
  대조: [`access.sql`](../../../engine/sql/access.sql) 의 9개는 `PARALLEL SAFE`,
  [`traverse.rs:80,359,442`](../../../engine/src/storage/traverse.rs) 는 `parallel_restricted`.
- **현상**:
  1. `SELECT og_cypher(...)` 를 포함한 계획은 절대 병렬화되지 않는다.
  2. 컴파일된 안쪽 SQL이 SPI를 통해 병렬 계획을 받을 수 있는지는 **미확인**.
  3. `og_is_subtype` / `og_subtypes` / `og_supertypes` 는 `parallel_safe` 로 선언되어 있으나
     내부에서 SPI를 호출한다 ([`labeling.rs:192-244`](../../../engine/src/catalog/labeling.rs)).
     SPI를 쓰는 함수는 일반적으로 `PARALLEL RESTRICTED` 여야 한다 — **정확성 위험, 미확인.**
- **정량적 추정**: 병렬화가 도움이 되는 워크로드(큰 라벨 스캔, 큰 집계)를 재는 벤치가 없다.
  1홉/3홉 워크로드는 시작 노드 1개라 병렬 이득이 없다.
- **제안**:
  1. 먼저 §2를 확인한다. 안쪽 SQL이 병렬 계획을 받는다면 `og_cypher` 의 선언은 문제가 아니다.
  2. 읽기 전용 진단 함수(`og_cypher_sql`, `og_schema`, `og_graph_stats` 등)에 적절한 표기를 붙인다.
  3. `og_is_subtype` / `og_subtypes` / `og_supertypes` 를 `parallel_restricted` 로 내린다
     (컴파일 시점 호출이 대부분이라 손해가 거의 없다).
- **예상 효과**: 큰 스캔에서 `max_parallel_workers_per_gather` 만큼 (미측정).
- **리스크**: 3번은 이 함수들을 방출하는 계획에서 병렬을 막는다.
  단, `compile.rs` 는 `og_is_subtype` 를 스칼라 바인딩 경로에서만 방출한다
  ([`compile.rs:744-746`](../../../engine/src/cypher/compile.rs)).
- **검증 방법**:
  ```sql
  SELECT proname, proparallel FROM pg_proc WHERE proname LIKE 'og\_%' ORDER BY proparallel, proname;
  -- 'u'(unsafe) 인 것 목록을 본다
  SET max_parallel_workers_per_gather = 4;
  SELECT og_cypher_explain('benchg', $$MATCH (a:P) RETURN count(a)$$, false);
  -- plan 안에 "Gather" 가 있는지
  ```

### PERF-25 — 빌드 설정이 `bolt` 크레이트에 적용되지 않았다

- **심각도**: Low
- **근거**: [`engine/Cargo.toml`](../../../engine/Cargo.toml)
  ```toml
  [profile.release]
  panic = "unwind"
  opt-level = 3
  lto = "fat"
  codegen-units = 1
  ```
  [`bolt/Cargo.toml`](../../../bolt/Cargo.toml)
  ```toml
  [profile.release]
  opt-level = 3
  ```
- **현상**: Bolt 게이트웨이는 LTO도 `codegen-units = 1` 도 받지 못한다.
  PackStream 인코딩·`serde_json` 파싱이 이 크레이트의 핫루프다(PERF-17).
- **정량적 추정**: 직렬화 위주 코드에서 LTO + `codegen-units=1` 은 통상 한 자릿수 % 수준(추정).
  **미측정.**
- **제안**:
  1. `bolt/Cargo.toml` 의 `[profile.release]` 에 `lto = "fat"`, `codegen-units = 1` 을 추가한다.
  2. 그 외 빌드 측면 여지 (전부 미검증):
     - `engine` 은 `cdylib` + `panic = "unwind"` 가 pgrx 요구사항이므로 바꾸지 않는다.
     - `target-cpu=native` 는 배포 이식성을 깨므로 개발 벤치 전용으로만.
     - `og_reach` / `og_csr_reach` 의 내부 루프는 슬라이스 인덱싱이라 경계 검사가 있다
       ([`traverse.rs:174-176,383-395`](../../../engine/src/storage/traverse.rs)).
       반복자 기반으로 바꾸면 검사가 사라질 수 있다 — `unsafe` 없이 가능한 범위에서만.
- **예상 효과**: 미측정.
- **리스크**: 빌드 시간 증가.
- **검증 방법**:
  ```bash
  cd bolt && cargo build --release && ls -l target/release/ontological-bolt
  # 처리량: 같은 Bolt 워크로드를 변경 전후로 측정
  ```

### PERF-26 — `chunk_size` / `supernode_threshold` 설정이 코드에 반영되지 않는다

- **심각도**: Low
- **근거**: [`bootstrap.sql:256-260`](../../../engine/sql/bootstrap.sql) 이
  `chunk_size=256`, `supernode_threshold=4096` 을 넣지만,
  `grep -rn "supernode_threshold\|chunk_size" engine/src/` 결과는
  [`storage/stats.rs:77`](../../../engine/src/storage/stats.rs) 에서 **하드코딩 상수를 보고하는 것** 뿐이다.
  실제 값은 [`adjacency.rs:15`](../../../engine/src/storage/adjacency.rs) 의 `pub const CHUNK: i32 = 256`.
- **현상**: 설정 테이블의 두 키는 아무것도 하지 않는다. 운영자가 바꿔도 동작이 변하지 않는다.
- **정량적 추정**: 해당 없음.
- **제안**: 둘 중 하나.
  (a) `CHUNK` 를 설정에서 읽고 백엔드-로컬로 캐시한다 (PERF-09의 꼬리 청크와 함께 설계).
  (b) 설정 행을 지우고 문서에서 컴파일 시점 상수라고 명시한다.
- **예상 효과**: PERF-09의 튜닝 가능성 확보.
- **리스크**: 세그먼트 크기를 바꾸면 기존 데이터와 섞인다. 읽기 경로는 `n` 을 보므로 호환되지만
  `og_reorganize` 의 기준이 달라진다.
- **검증 방법**:
  ```sql
  SELECT * FROM og_catalog.setting;
  SELECT og_set_setting('chunk_size','64');
  -- 새 엣지를 만들고 세그먼트 최대 n 을 본다
  SELECT max(n) FROM og_data.og_adj;   -- 여전히 256까지 차면 설정이 무시된 것
  ```

### PERF-27 — 회귀 게이트가 페이지 수와 `reach*` 워크로드를 보지 않는다

- **심각도**: High
- **근거**: [`harness.py:1221-1241`](../../../bench/harness.py),
  [`bench/results/baseline.json`](../../../bench/results/baseline.json) (2026-08-06, `1hop/2hop/3hop/prop_scan` 만).
- **현상**: [`06_regression_guard.md` §4](06_regression_guard.md) 의 G1~G7.
  특히 (a) `buffers` 미비교, (b) 베이스라인에 없는 질의는 조용히 통과,
  (c) `reach*` 워크로드 전체가 게이트 밖, (d) `correctness` 와 `integrity_violations` 미검사.
- **정량적 추정**: 해당 없음.
- **제안**:
  ```python
  # compare() 안에
  #  - 베이스라인에 있는데 현재에 없는 질의를 실패로 처리
  #  - buffers 를 별도 임계값(예: 1.10)으로 비교
  #  - median_ms 비교에 하한(예: 0.5 ms 미만이면 절대차 0.3 ms 기준)
  #  - current["correctness"] 중 agree=false 가 하나라도 있으면 실패
  #  - current["integrity_violations"] > 0 이면 실패
  ```
  그리고 `reach` 워크로드 베이스라인을 새로 만들어 커밋한다.
- **예상 효과**: 저장 구조 회귀(페이지 수)와 깊은 순회 회귀가 실제로 잡힌다.
- **리스크**: 서브밀리초 셀의 노이즈로 인한 오탐. 하한 규칙이 필요하다.
- **검증 방법**:
  ```bash
  python3 bench/harness.py --scale 50000 --degree 20 --workload reach \
      --hops 1,2,3,4,5,6,8 --systems ontological,ontological_raw,cte
  # 결과를 bench/results/baseline-reach.json 으로 커밋
  python3 bench/harness.py … --compare-baseline bench/results/baseline-reach.json
  # 일부러 og_reach 를 느리게 만들고 게이트가 실패하는지 확인
  ```

### PERF-28 — 라벨 스캔의 `UNION ALL` 브랜치 수가 서브타입 수에 비례한다

- **심각도**: Low (미확인)
- **근거**: [`views.rs:102-135`](../../../engine/src/cypher/views.rs).
- **현상**: `MATCH (v:Thing)` 은 `Thing` 의 모든 구체 서브타입에 대해 `SELECT … FROM og_data.n_<sub>` 를
  `UNION ALL` 로 잇는다. 서브타입 100개면 `Append` 브랜치 100개다.
  서브타입이 갖지 않은 프로퍼티는 `NULL::type AS col` 상수로 채워지므로
  그 브랜치는 해당 컬럼에 대해 통계가 없다.
- **정량적 추정**: **미측정.** 벤치 그래프의 타입은 2개뿐이고, 데모 그래프도 작다.
  큰 온톨로지(수백 타입)에서의 계획 시간과 실행 시간은 아무도 측정하지 않았다.
- **제안**: 먼저 측정한다. 문제가 확인되면
  (a) 상속 계층을 PostgreSQL 파티션 테이블로 매핑하거나,
  (b) `og_data.og_node` 를 앵커로 `type_id = ANY(ARRAY[...])` 로 좁힌 뒤 필요한 타입 테이블만 조인하는
  대안 경로를 두고 서브타입 수에 따라 고른다.
- **예상 효과**: 미측정.
- **리스크**: (b)는 실컬럼 통계와 인덱스를 쓰기 어려워져 이 설계의 핵심 이점을 잃는다.
- **검증 방법**:
  ```sql
  -- 100개 서브타입을 만들고
  DO $$ BEGIN FOR i IN 1..100 LOOP
    PERFORM og_create_type('t','Sub'||i,'entity', ARRAY['Root']); END LOOP; END $$;
  \timing on
  SELECT og_cypher_sql('t', $$MATCH (v:Root) RETURN count(v)$$);   -- 컴파일 시간
  SELECT og_cypher_explain('t', $$MATCH (v:Root) RETURN count(v)$$, true);  -- Planning Time
  ```

### PERF-29 — `og_csr_build` 가 자동이 아니고 무효화 전략이 없다

- **심각도**: Med
- **근거**: [`traverse.rs:205-210`](../../../engine/src/storage/traverse.rs) (`thread_local` CSR),
  [`:241-292`](../../../engine/src/storage/traverse.rs) (`compile`),
  [`:295-313`](../../../engine/src/storage/traverse.rs) (`og_csr_build`).
  컴파일러는 CSR로 라우팅하지 않는다 ([`compile.rs:865-873`](../../../engine/src/cypher/compile.rs)).
- **현상 (그리고 그것이 옳은 이유)**: CSR은 (1) 백엔드-로컬, (2) 빌드 시점에 동결된 스냅샷,
  (3) RLS 미적용, (4) `PARALLEL RESTRICTED` 다.
  Cypher가 조용히 이것을 쓰면 MVCC와 RLS를 사용자 모르게 포기하는 것이 된다.
  **현재의 "자동이 아님"은 결함이 아니라 결정이다.**
  문제는 자동화 조건과 무효화 전략이 **정의되어 있지 않다**는 것이다.
- **정량적 추정 (측정)**:
  - 이득: dense 6홉 71.42 → 4.86 ms (**14.7배**), sparse 20홉 205.81 → 21.13 ms (9.7배).
  - 비용: dense 8.39 MiB / 119 ms, sparse 9.16 MiB / 229 ms,
    **chain-1M 22.9 MiB / 935 ms, grid-1M 30.5 MiB / 968 ms** — 백엔드마다.
    출처: [`bench/csr/results/`](../../../bench/csr/results/).
  - 즉 100만 노드 규모에서 연결마다 약 1초와 23~31 MB다.
    커넥션 풀 100개면 2.3~3.1 GB.
- **제안**: 자동화하되 **명시적 옵트인**으로.
  1. GUC `ontological.use_csr = off | when_built | always` 를 두고,
     `when_built` 일 때만 컴파일러가 CSR 경로를 고려한다.
  2. **무효화**: `og_csr_build` 시점의 `og_data.og_edge` 의 `pg_class.reltuples` 와
     `og_catalog.schema_version` 을 함께 저장하고, `og_csr_stats()` 에 노출한다.
     둘 중 하나라도 바뀌면 스테일로 표시하고 CSR 경로를 쓰지 않는다.
     정확한 무효화는 아니지만(같은 수의 삽입/삭제는 놓친다) 비용이 0인 근사다.
  3. RLS가 켜져 있으면(`og_enable_rls` 를 쓴 타입이 있으면) 옵트인이어도 CSR을 쓰지 않는다.
  4. 트랜잭션 안에서 미커밋 쓰기가 있으면 쓰지 않는다.
- **예상 효과**: 옵트인 배치 워크로드에서 dense 6홉 기준 14.7배 (**측정**).
- **리스크**: 이 기능의 존재만으로도 "MVCC를 지킨다"는 주장이 조건부가 된다.
  문서에 반드시 명시해야 한다.
- **검증 방법**:
  ```sql
  SELECT * FROM og_csr_build(ARRAY[<etype>]::int4[], 'o');
  SELECT * FROM og_csr_stats();
  \timing on
  SELECT count(*) FROM og_csr_reach(<start>, 1, 6);
  SELECT count(*) FROM og_reach(<start>, ARRAY[<etype>]::int4[], 'o'::"char", 1, 6);
  -- 스테일 감지 테스트: 다른 세션에서 엣지를 넣고 커밋한 뒤
  SELECT * FROM og_csr_stats();   -- stale 로 보이는가
  ```

### PERF-30 — `og_node_json` / `og_edge_json` 이 plpgsql + 동적 `EXECUTE`

- **심각도**: High
- **근거**: [`access.sql:208-235`](../../../engine/sql/access.sql) (`og_node_json`),
  [`:237-264`](../../../engine/sql/access.sql) (`og_edge_json`).
  방출 지점: [`compile.rs:991`](../../../engine/src/cypher/compile.rs) (프로퍼티 읽기),
  [`:1013`](../../../engine/src/cypher/compile.rs) (JSON 보존 읽기),
  [`:1087`](../../../engine/src/cypher/compile.rs), [`:1111`](../../../engine/src/cypher/compile.rs) (노드/엣지 전체),
  [`:1184`](../../../engine/src/cypher/compile.rs).
  그 외: [`access.sql:267-270`](../../../engine/sql/access.sql) (`og_prop`),
  [`access.sql:311`](../../../engine/sql/access.sql) (**`og_typeql_attribute` 뷰가 행마다 호출**),
  [`vector/mod.rs:113,127,172,182,276`](../../../engine/src/vector/mod.rs).
- **현상**: 한 번의 호출이 (1) `og_node` ⋈ `og_catalog.type` 조회, (2) `format()` + 동적 `EXECUTE`,
  (3) `og_catalog.property` 집계 — SQL 문 **3개**를 돈다. `LANGUAGE plpgsql` 이라 인라인되지 않고
  옵티마이저 장벽이 된다. 그리고 이것이 **행마다** 실행된다.
  발동 조건은 "타입이 컴파일 시점에 확정되지 않은 노드/엣지":
  라벨 없는 패턴(`MATCH (n)`), 프로시저가 yield 한 노드, `UNWIND` 로 들어온 노드,
  관계 변수만 있고 타입이 하나로 좁혀지지 않은 관계.
- **정량적 추정**: **미측정.** `MATCH (n) RETURN n.name` 을 50,000 노드에 실행하면
  150,000개의 SQL 문이 백엔드 안에서 실행된다(추정).
  `og_typeql_attribute` 뷰는 `og_edge` 전체에 대해 같은 일을 한다.
- **제안**:
  1. 컴파일러가 타입을 확정할 수 있는 모든 경로에서 `og_node_json` 을 절대 방출하지 않는다
     (현재도 대부분 그렇지만, `Bind::Scalar` 경로와 `Bind::Rel{alias: None}` 경로가 남아 있다).
  2. 확정 불가한 경우, 실행 시점에 `type_id` 로 분기하는 대신
     "모든 구체 타입의 `UNION ALL` 뷰"를 하나 더 만들어 `og_data.og_node` 대신 쓴다
     (`ensure_view(root_type)` 와 같은 방식).
  3. 최소한 `og_node_json` 을 `LANGUAGE sql` 로 재작성할 수 있는 부분만이라도 분리한다
     (동적 `EXECUTE` 가 필요한 것은 `to_jsonb(x)` 부분뿐이다).
  4. `og_typeql_attribute` 뷰의 `og_node_json(e.dst) ->> 'val'` 을
     속성 타입 테이블 조인으로 바꾼다.
- **예상 효과**: 라벨 없는 패턴과 TypeQL 뷰에서 큰 폭 (미측정).
- **리스크**: 타입 미상 노드에 대한 일반성을 유지해야 한다.
  (2)안은 타입이 추가될 때마다 뷰를 다시 만들어야 한다(이미 `drop_all_views` 가 그렇게 한다).
- **검증 방법**:
  ```sql
  SELECT og_cypher_sql('benchg', $$MATCH (n) RETURN n.name LIMIT 10$$);
  -- og_node_json 이 들어 있는지 확인
  EXPLAIN (ANALYZE, BUFFERS) SELECT og_node_json(id) FROM og_data.og_node LIMIT 10000;
  EXPLAIN (ANALYZE, BUFFERS) SELECT to_jsonb(x) FROM og_data.n_2 x LIMIT 10000;
  -- 두 시간의 비율이 이 항목의 크기다
  ```

---

## 3. 권장 착수 순서

정확성을 먼저, 그다음 게이트, 그다음 큰 성능 항목 순이다.
게이트가 없는 상태에서 성능을 고치면 무엇이 좋아졌는지 증명할 수 없다.

| 순서 | 항목 | 이유 |
|---|---|---|
| 1 | **PERF-20** | 지금 틀린 답을 낼 수 있다 |
| 2 | **PERF-19, PERF-27** | 게이트가 없으면 이후 모든 변경을 검증할 수 없다 |
| 3 | **PERF-14** | 벤치의 페이지 열이 무엇을 재는지부터 확정해야 한다 |
| 4 | **PERF-02, PERF-03** | 측정된 179,032 페이지 / 174 ms — 가장 큰 단일 항목 |
| 5 | **PERF-01** | 라벨 스캔이 인덱스를 쓰는지 아닌지가 모든 표의 전제 |
| 6 | **PERF-10, PERF-06** | 카탈로그 캐시 + 무효화 (읽기·쓰기 양쪽) |
| 7 | **PERF-08, PERF-09, PERF-11** | 쓰기 경로. 단 먼저 기준선을 만들어야 한다 |
| 8 | **PERF-30, PERF-12, PERF-18** | 결과 전달 경로 |
| 9 | 나머지 | |

<!-- affects: backend, api, frontend, ops, data -->
<!-- requires-update: docs/deep-traversal.md, bench/README.md, docs/01_architecture/09_performance/02_measured_baselines.md -->
