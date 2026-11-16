# Bolt/Cypher 호환 정답집 (`tests/compat/`)

v2(Apache AGE 모양 저장)는 v1 과 **함께 유지되는 판**이다. 클라이언트 셋 — Ontology Studio, Robo Architect
Lite, Robo Analyzer — 이 Bolt 로 보내는 Cypher 가 v2 에서도 v1 과 같은 답을 내는지 상시로 대조하려고 만든
모음과 그 v1 정답이다(설계서 `reports/ontological-v2/DESIGN.md` 7.4절, 8절 P0-c).

```
tests/compat/
  extract/            클라이언트 소스에서 Cypher 를 정적으로 뽑아 corpus/ 를 만드는 스크립트
    extract.py          진입점
    pysrc.py jssrc.py   파이썬(문자열·f-string·연결·.format) / JS(문자열·템플릿) 읽기
    prepare.py          그래프 배정 · 준비 데이터 유도 · 매개변수 · 시나리오
    clients.json        어디를 읽나 (클라이언트별 루트·글롭)
    assembly.json       조립 규칙 (자리표시자 값 · 손으로 쓴 대표 꼴 · 제외 · 매개변수 덮어쓰기 · 준비 데이터 설정)
  shape.py            Cypher 텍스트를 가볍게 읽는 규칙 (뽑기와 실행기가 같이 쓴다)
  corpus/<client>/    모음
    queries/<id>.cypher   한 문장씩, Bolt 로 나가는 텍스트 그대로
    params/<id>.json      매개변수 (값, 또는 실행기가 그래프에서 찾아 넣는 {"$ref": …})
    fixture/*.cypher      준비 데이터 (파일 이름 순서로 실행)
    manifest.json         쿼리마다 종류·그래프·출처(파일:줄)·자리표시자·조립 규칙, 그리고 시나리오
    excluded.json         뽑혔지만 모음에 넣지 않은 것과 그 까닭
  corpus/_shared/fixture/ 모든 클라이언트가 같이 쓰는 준비 데이터 (패브릭 데이터소스)
  targets/<target>/bootstrap.sql   대상 판의 시작점 (v1 은 있음, age 는 자리만)
  targets/source.sql  패브릭이 읽는 원천 시스템(작은 ERP, 일반 PostgreSQL 테이블)
  targets/stats.sql   준비 데이터를 넣은 뒤 한 번: 통계를 떠 두고 autovacuum 을 꺼 실행 중 계획이 바뀌지 않게 한다
  normalize.py        결과 정규화 규칙
  run.py              실행기 — 모음을 Bolt 로 돌려 결과를 저장하거나 정답과 비교한다
  unstable.json       같은 v1 에서도 실행마다 답이 달라 정답에서 뺀 쿼리 (지금은 없음)
  expected/v1/        v1 정답
  expected/known-differences.json   v2 가 v1 과 달라도 되는 것과 그 사유 (P3 부터 채운다)
```

## 쓰는 법

필요한 것: `psql`, Python 3.9 이상(3.9.6 과 3.12 에서 바이트까지 같은 결과를 확인했다)과 neo4j 드라이버
(5.28 로 확인; Robo 는 6.0.3 을 쓴다), 대상 판의 PostgreSQL 과 Bolt 게이트웨이 실행 파일. 실행기는
`--pg-database`(기본 `og_compat`)를 **지우고 새로 만들고**, 그 데이터베이스의 세션 TimeZone 을 `UTC` 로 고정한다
(`ALTER DATABASE … SET TimeZone`). 그래서 클러스터의 시간대(UTC, Asia/Seoul …)와 포트가 달라도 결과 폴더는 같다.
준비 데이터를 넣은 뒤에는 `targets/stats.sql` 로 `ANALYZE` 를 한 번 하고 그 데이터베이스 테이블들의 autovacuum 을
끈다. autovacuum 이 실행 중간에 통계를 바꾸면 계획이 바뀌어, 30홉 가변 길이 쿼리 하나가 0.01초에서 30초 제한을
넘긴 일이 있었다.

```sh
# v1 정답 다시 만들기 (게이트웨이는 실행기가 띄우고 끈다)
python3 tests/compat/run.py --psql <pg>/bin/psql --pg-port 28830 \
    --bolt-bin <target-bolt>/release/ontological-bolt --bolt-listen 127.0.0.1:28731 \
    --out tests/compat/expected/v1

# 인수 조건: 두 번 돌려 같은 결과 (<out> 과 <out>.2 를 diff -r)
python3 tests/compat/run.py … --out /tmp/compat --repeat 2

# 지금 엔진을 정답과 비교
python3 tests/compat/run.py … --out /tmp/compat --compare tests/compat/expected/v1
```

게이트웨이를 따로 띄워 두었으면 `--bolt-bin` 대신 `--bolt-uri bolt://…` 를 준다(그 게이트웨이의
`OG_BOLT_PGDATABASE` 가 `--pg-database` 와 같아야 한다). 문장마다 제한 시간은 `--timeout`(기본 30초)이다.
한 번 도는 데 v1 에서 약 70초(쿼리 1,154개, 시나리오 148개) 걸린다.

결과 폴더:

- `_setup.json` — 준비 데이터 파일마다 문장 수, 실패 수, 실패한 문장의 줄과 오류. 클라이언트의 스키마 스크립트
  가운데 v1 이 받지 않는 문장도 여기 남는다(아래 「v1 에서도 실패」).
- `<client>/<id>.json` — 읽기 하나의 답: `columns`, `row_order`, `row_count`, `rows`, (쓰기 카운터가 있으면)
  `counters`, 실행할 때 넣은 `params`(정규화한 값), 실패면 `error`.
- `<client>/scenarios/<이름>.json` — 쓰기 시나리오: 단계마다 위 내용 + `effect`(그 단계가 그래프에 낸 변화).
- `summary.json` — 클라이언트마다 읽기·단계 수, 성공 수, 오류 코드별 수, 실패한 쿼리 목록.

## v2 에 대조하는 법 (`--target age`)

`--target` 은 시작점(`targets/<target>/bootstrap.sql`)만 고른다. 모음·준비 데이터·실행·정규화는 대상과 상관없이
Bolt 로만 돈다. v2 를 대조하려면:

1. `targets/age/bootstrap.sql` 을 채운다 — v1 의 것과 같은 출발점이어야 한다: `:graphs` 의 그래프들, Studio
   그래프(`ontology`)의 추상 루트 타입 `_Entity`, 원천 시스템(`\ir ../source.sql`), `postgres_fdw`. 지금은 자리만 있다(P2).
2. v2 의 게이트웨이로 돌린다: `run.py --target age --bolt-bin <v2-bolt> … --out /tmp/age --compare tests/compat/expected/v1`.
3. 다른 것마다 고치거나, 달라도 되는 까닭이 있으면 `expected/known-differences.json` 에 `{"id", "reason"}` 으로 적는다.

비교 규칙(`run.py` 의 `same`): 실패는 **오류 코드만** 같으면 같다(메시지는 사람이 보라고 남긴다). `subset` 이 붙은
답은 열과 행 수만 본다. 나머지는 `columns`·`row_count`·`rows`·`rows_sha1`·`effect` 가 모두 같아야 한다. 준비 데이터도 비교한다:
`_setup.json` 에서 파일마다 실패한 문장(줄 번호)과 그 오류 코드가 같아야 한다.
`row_order` 가 `server, ties server` 인 답(정렬 키를 반환하지 않는 ORDER BY)은 동점 행의 순서가 판마다 다를 수
있으니, 다르게 나오면 그 쿼리를 보고 판단한다.

## 새 쿼리 추가법

모음은 손으로 고치지 않는다. 생성물(`queries/`, `params/`, `manifest.json`, `excluded.json`, `fixture/1?_client_*`,
`fixture/90_derived*`)은 `extract.py` 가 매번 새로 쓴다.

1. 클라이언트 소스가 바뀌었으면 그냥 다시 뽑는다: `python3 tests/compat/extract/extract.py [client …]`.
   체크아웃 위치가 다르면 `clients.json` 의 `env` 이름(`COMPAT_STUDIO_APP`, `COMPAT_ROBO` …)으로 바꾼다.
   `--check` 는 아무것도 쓰지 않고, 풀리지 않은 자리표시자와 조각만 보여 준다.
2. 「unresolved placeholder」 가 나오면 `assembly.json` 에 값을 적는다 — `placeholders.<client>` (클라이언트 전체),
   `file_placeholders."<root>:<path>"` (파일 하나), 그 안의 `"@<줄>:<식>"` (그 줄에서 시작하는 후보 하나).
   값은 준비 데이터에 있는 라벨·속성으로 고른다.
3. 코드가 조각을 이어 붙여 만드는 쿼리(리스트 join, `+=`, `str.replace`)는 `assembled` 에 대표 꼴을 손으로 쓰고
   `rule` 에 조립 규칙을, `covers` 에 그 조각들의 출처를 적는다.
4. 쿼리가 아닌데 뽑힌 텍스트(로그·메시지)는 `skip` 에 까닭과 함께 넣는다.
5. 규칙으로 못 고르는 매개변수 값은 `params`(쿼리 id 나 출처로) 또는 `param_names`(이름으로)에 적는다.
6. 쿼리가 기대하는 데이터가 유도로 안 생기면 `corpus/<client>/fixture/2?_*.cypher` 에 손으로 쓴다(첫 줄
   `// graph: <그래프>`). 고객 데이터는 쓰지 않는다.
7. 정답을 다시 만들고(`--repeat 2` 로 같은지 확인) 바뀐 정답 파일을 같이 커밋한다.

## 뽑는 규칙

- **대상**: `clients.json` 의 글롭 안의 파이썬·JS 파일, Studio 스킬 문서의 ```` ```cypher ```` 블록(LLM 이 쓰는 열린
  쿼리의 기록된 예시), Robo 의 스키마 스크립트(`docs/cypher/schema/*.cypher`, 준비 데이터로 쓴다). 시험 폴더는 뺀다.
- **Studio 백엔드는 배포되는 판을 읽는다**: `runtime/studio/app` = `sources/ontology-studio/backend` 에
  `patches/studio-backend` 를 적용한 것(`scripts/build-studio.sh`). 손으로 패치를 적용한 트리와 바이트까지 같은 것을
  확인했다. 줄 번호도 그 판의 것이다. 없으면 `sources/ontology-studio` 를 읽는다.
- **문자열 하나**: 다른 문자열 식의 일부가 아닌 가장 큰 문자열 식(리터럴, f-string, 암묵 연결, `+`, `%`, `.format`,
  `textwrap.dedent`, `.strip()`). 독스트링은 뺀다. 안의 이름은 조립 규칙 → 함수 안의 단일 대입 → 모듈 상수 →
  같은 루트의 다른 모듈에서 import 한 상수 순으로 푼다. 클라이언트 코드는 import 하지도 실행하지도 않는다.
- **Cypher 인가**: 첫 단어가 절 키워드(MATCH, MERGE, CREATE, UNWIND, WITH, CALL, RETURN, SHOW, DROP, …)여야 하고,
  SQL(`CREATE TABLE`, `SELECT … FROM`, CTE), 자연어, 한 단어, 정규식은 뺀다.
- **온전한 문장인가**: RETURN 이나 쓰기 절이 있거나, `CALL`·`SHOW`·스키마 DDL·패브릭 문장이어야 한다. 아니면
  「fragment」 로 빼고 `assembled` 로 대표 꼴을 쓴다.
- **클라이언트 변환**: Robo Analyzer 는 Ontological 로 보낼 때 `MATCH (x:ANALYZER_OWNED {…})` 의 라벨을 지운다
  (`graph/neo4j_client.py` 의 `for_ontological`). 모음에는 변환한 뒤의 텍스트, 곧 실제로 나가는 텍스트를 넣는다.
- **합치기**: 공백을 하나로 줄인 텍스트가 같으면 한 쿼리다. 출처(`root:파일:줄`)는 모두 `origins` 에 남는다.
  id 는 그 텍스트의 SHA-1 앞 10자리(`<client>-xxxxxxxxxx`)라 소스가 바뀌지 않으면 그대로다.
- **그래프**: Studio 는 `ontology`, Robo 는 `robo`, Analyzer 는 `analyzer_run`. Robo 가 분석 그래프를 읽는 쿼리
  (대문자 라벨 `TABLE`·`FUNCTION`… 이나 `'analyzer'` 소유 표시가 있는 것)는 `robo_analysis` 로 보낸다 — 실제로는
  분석기의 그래프를 읽지만, 두 클라이언트의 준비 데이터가 섞이지 않게 따로 둔다.

## 준비 데이터 규칙

파일 이름 순서로, 문장마다 자동 커밋으로 실행한다. 실패한 문장은 `_setup.json` 에 남고 다음 문장으로 간다.

1. `_shared/fixture` — 패브릭 데이터소스 `erp`, `erp_old` (원천은 같은 DB 의 `compat_erp` 스키마).
2. 손으로 쓴 것(`2?_*.cypher`) — Studio: `_Entity` 아래 Customer·Supplier·PurchaseOrder 인스턴스, 발행 레지스트리
   (`_OntologyModel` …), 색인된 문서(Document·Chunk, 전문·벡터 색인), 가상 노드 둘과 가상 관계 하나.
   Analyzer: 분석 그래프의 가상 노드.
3. 클라이언트 스키마 스크립트를 그대로 옮긴 것(`1?_client_*.cypher`, Robo) — 바꾼 것은 하나: `randomUUID()` 를
   (파일, 문장, 순번)의 uuid5 로 고정했다. 그래야 실행마다 같은 데이터다.
4. 유도한 것(`90_derived*.cypher`) — 모음의 쿼리들이 쓰는 라벨·라벨 사슬·속성·관계 패턴에서 만든다.
   - 손으로 쓴 데이터에 없는 라벨마다 노드 둘(`id: '<Label>-1'`, `'<Label>-2'`). 라벨 사슬은 그 라벨이 끝에 오는
     가장 긴 사슬(`:ANALYZER_OWNED:RULE`, `:_Entity:MfgProduct`)이고, 부모로만 쓰이는 라벨에는 노드를 만들지 않는다.
   - 속성: 쿼리가 그 라벨에서 읽거나 쓰는 속성 전부. 값은 쿼리에 나온 리터럴(`{status: 'APPROVED'}`,
     `n.kind = 'x'`)이 먼저, 다음은 `coalesce(n.p, false|0|[])` 의 기본값이 알려 주는 타입, 다음은 이름 규칙
     (`*Id` 는 그 라벨 노드의 id, `*At` 은 고정 시각, `is*` 는 참/거짓, `*count` 는 정수, `embedding` 은 8차원 벡터 …).
     예약 속성(`_id`, `_type`, `_src`, `_dst`, `__*`)은 쓰지 않는다.
   - `fixture_defaults`(assembly.json): Analyzer 노드에는 언제나 `graph_owner`·`_owner = 'analyzer'`, Robo 의
     `session_id` 는 모두 `'sess-1'`. Robo·Analyzer 는 손으로 쓴 데이터의 라벨에도 모든 속성을 가진 노드 둘을 더한다.
   - 관계: 양 끝 라벨이 알려진 `(A)-[:T]->(B)` 패턴마다 유도 노드 1→1, 2→2. 한쪽이 라벨 없는 패턴은 같은 관계
     타입의 다른 패턴에서 그 끝의 라벨을 빌린다.
   - 가상 노드 이름(`CREATE VIRTUAL NODE X`)은 저장 노드로 만들지 않는다.

## 매개변수 규칙

`params/<id>.json` 은 쿼리 텍스트에서 `$p` 가 놓인 자리를 보고 고른다(`prepare.py` 의 `params_for`).

- `LIMIT $p` → 10, `SKIP $p` → 0. 벡터 프로시저의 k → 3, 벡터 → 8차원 고정 벡터.
- `(v:L {k: $p})` 가 MATCH 에 있으면 **있는 노드**의 값, CREATE·MERGE(쓰기)에 있으면 새 값(`compat-new-…`).
- `v.k = $p`, `toString(v.k) = $p`, `… CONTAINS toLower($p)` → 있는 노드의 값. `SET v.k = $p` → 새 값.
- `v.k IN $p` → 있는 노드 둘의 값. `elementId(v) = $p` / `id(v) = $p` → 그 라벨의 첫 노드 id.
- `UNWIND $p AS x` (또는 `$p AS xs … UNWIND xs AS x`) → 원소 둘의 리스트. `x.f` 를 쓰는 자리마다 위 규칙.
- `$p IN labels(v)` → 라벨 이름. 라벨 없는 변수는 같은 관계 타입의 패턴에서 라벨을 빌린다.
- 그래도 모르면 `param_names`(이름별 값), 그다음 이름 규칙, 끝으로 `compat-<이름>`.
- 「있는 노드의 값」 은 대부분 `{"$ref": "prop", "label", "prop", "nth"}` 로 남겨, **실행할 때** 그래프에서 찾는다:
  `MATCH (n:L) WHERE n.p IS NOT NULL RETURN DISTINCT n.p AS v ORDER BY v SKIP nth LIMIT 1`. id 는 내용 키
  (`id`, `_source_id`, `key` …) 순으로 고른 노드의 것이다. 그래서 v2 에서도 같은 노드를 고른다.
  한 쿼리에서 같은 것을 찾는 매개변수가 둘이면 서로 다른 노드(nth 0, 1)를 받는다. `{"$env": …}` 는 접속 정보다.

## 쓰기 시나리오

쓰기·스키마·패브릭 쓰기 쿼리는 **출처 파일마다 시나리오 하나**로 묶는다. 단계 순서: 스키마·패브릭 생성 →
CREATE/MERGE → SET/REMOVE → DELETE/DROP, 같은 단계 안에서는 소스의 줄 순서. 시나리오는 명시 트랜잭션 하나에서 돌고
끝에 **롤백**한다(시나리오끼리 서로 영향이 없다). 단계가 실패하면 오류를 적고, 롤백한 뒤 새 트랜잭션에서 앞서 성공한
단계들을 다시 실행해 같은 상태로 돌아가 다음 단계를 이어 간다.

단계마다 `effect` 는 그 단계 앞뒤의 그래프(`MATCH (n) RETURN n`, `MATCH ()-[r]->() RETURN r`)를 내용으로 비교한
것이다: `nodes_added`·`nodes_removed`·`nodes_changed`·`rels_added`·`rels_removed`. 노드는 라벨+속성, 관계는
타입+양 끝 노드+속성으로 나타낸다(id 를 쓰지 않으므로 판마다 같다).

## 정규화 규칙

`normalize.py` 와 `run.py` 의 `shape_result`.

1. **값**: 노드는 `{"~node": @nK, "labels": 정렬, "props"}`, 관계는 `{"~rel": @rK, "type", "start", "end", "props"}`,
   경로는 `{"~path": [...]}`. 시간 타입은 `{"~temporal": 타입, "v": ISO}`.
2. **행 순서 판단**: 마지막 최상위 RETURN 뒤에 ORDER BY 가 있으면 서버 순서를 지킨다(`row_order: "server, …"`).
   그때 ORDER BY 의 키가 모두 반환 열(별칭이나 같은 식)이면 키가 같은 행끼리는 정렬한다(`ties sorted`) — 동점의
   순서는 정해져 있지 않기 때문이다. 키가 반환되지 않으면 동점을 가릴 수 없어 서버 순서 그대로다(`ties server`).
   ORDER BY 가 없으면 행 전체를 id 를 가린 JSON 으로 정렬한다(`sorted`).
3. **LIMIT 이 결과를 자른 경우**: ORDER BY 없이 LIMIT 이 있고 행 수가 LIMIT 에 닿았으면 어떤 행이 왔는지가 정해져
   있지 않다. 그런 답은 `subset` 을 달고 열과 행 수만 남긴다(행이 LIMIT 보다 적으면 전부 온 것이라 행을 남긴다).
4. **collect()·labels()·keys()** 로 만든 열의 리스트는 정렬한다. 단 RETURN 앞에 `WITH … ORDER BY` 가 있으면
   그 순서를 지킨다(collect 가 순서를 받기 때문).
5. **id**: 노드·관계의 id/elementId 는 처음 본 순서로 `@n1`, `@n2` … / `@r1` … 가 된다(2~4 를 적용한 뒤의 순서로).
   번호는 읽기 하나마다, 시나리오 하나마다 새로 매기며, 시나리오 안에서는 단계 사이에 이어진다. 그냥 정수나
   문자열이라도 그 순간 그래프에 있는 노드·관계의 id/elementId 이고, 열의 식에 `id(`/`elementId(` 가 있거나
   (정수는) 2^31 이상이면 id 로 본다. 실행 때 찾아 넣은 매개변수 값도 같은 규칙으로 바뀐다.
6. **시계**: 실행 시각 앞뒤 이틀 안의 시간 값과 ISO-8601 문자열(날짜만 있는 것도)은 `<now>`, epoch 밀리초는
   `<now:ms>`. ISO 문자열은 직접 파싱한다 — `datetime.fromisoformat` 은 Python 3.11 미만에서 소수 자리가 3·6자리가
   아닌 시각(PostgreSQL 은 뒤의 0 을 지운다: `.3238`)과 `Z` 를 못 읽어 결과가 Python 에 따라 달라졌다.
   준비 데이터의 고정 시각은 그대로 남는데, 세션 TimeZone 이 `UTC` 라 엔진이 돌려주는 시각은 늘 `+00:00` 이다.
7. **randomUUID()**: UUID 꼴 문자열은 `<uuid1>`, `<uuid2>` … (5와 같은 순서로).
8. **실수**는 유효 숫자 6자리.
9. **실패**는 `{"error": {"code", "message"}}`, 메시지의 9자리 이상 숫자는 `<n>`(id·oid).
10. **접속**: 실행에 쓴 PostgreSQL 포트·Bolt 포트·`host:port`·데이터베이스 이름은 `<pg-port>`, `<bolt-port>`,
    `<pg-host>:<pg-port>`, `<pg-database>` 가 된다(`SHOW CREDENTIALS`·`SHOW DATASOURCES` 와 데이터소스 매개변수에
    나온다). 오류 메시지에도 같은 규칙을 쓴다.
11. 200행이 넘는 답은 앞 200행과 전체의 SHA-1(`rows_sha1`)을 남긴다. 단계 변화도 종류마다 100개까지.

두 번 실행해 다른 것이 나오면, 까닭을 찾아 위 규칙에 넣거나 `unstable.json` 에 사유와 함께 적는다
(그 쿼리는 여전히 실행되지만 답 대신 사유만 남는다).

## v1 에서도 실패하는 쿼리

실패도 정답이다. 그 답에 `error.code` 가 들어 있고, v2 는 같은 코드로 실패해야 같다. 대표:

- Studio `db.schema.nodeTypeProperties`, `db.schema.visualization` — 엔진의 프로시저 등록부에 없다
  (`Neo.ClientError.Statement.ArgumentError`).
- `CALL db.index.*.queryNodes(…) YIELD node, score WHERE …` — YIELD 바로 뒤 WHERE 를 받지 않는다(SyntaxError).
- (실패하지 않는 것 하나) Robo 의 시험이 막는 `ORDER BY <맵 투영의 별칭>.prop` 은 10-31 에는 거부됐지만 이 v1
  (`v2/base`, Studio 패치 0001~0015 포함)에서는 **돈다**. 클라이언트는 더 보내지 않지만 기록으로 넣었다(assembled,
  `robo-7e31d0bd95`). 설계서 7.4 의 「아는 비호환」은 이 판에서는 해소된 것이다.
- Robo 스키마 스크립트 일부 — 아직 없는 속성에 색인·제약(`column "p_status" does not exist`), `when`·`then` 을
  변수 이름으로 쓴 문장(SyntaxError). `_setup.json` 에 남는다.
- 그 밖에 패브릭 자격 증명 참조(`ontological.fabric_credentials_dir` 가 없는 서버), 가상 관계가 걸린 가상 노드 DROP,
  만든 뒤 라벨을 더하는 `SET n:X`, 30홉 무방향 가변 길이 EXISTS 의 시간 초과 등. 목록은 `expected/v1/summary.json`
  의 `failing`.

## 한계

- 정적 추출이라 실행 경로에 따라 달라지는 쿼리는 대표 꼴 하나(또는 몇 개)만 있다. 자리표시자 값은 준비 데이터에
  맞춰 고른 것이고 클라이언트가 실제로 보내는 값의 전부가 아니다.
- LLM 이 만드는 열린 쿼리(nl2cypher, 에이전트의 Cypher 도구)는 소스·스킬 문서에 기록된 예시만 들어 있다.
- 준비 데이터는 규칙으로 유도한 것이라, 비어 있는 답이 나오는 읽기가 남아 있다(`summary.json` 과 각 답의
  `row_count` 로 보인다). 비어 있어도 열·오류 여부는 비교된다.
