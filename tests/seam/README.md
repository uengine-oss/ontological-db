# 생성 SQL 금형 (`tests/seam/`)

P1(저장 계층 이음매, `reports/ontological-v2/P1-CARDS.md`)의 카드 K2~K7 은 컴파일러와 저장 코드가 물리 이름을
쓰는 자리를 `engine/src/layout/` 의 `Layout` 뒤로 옮긴다. 옮기는 동안 **엔진이 만드는 SQL 이 글자 하나 달라지면
안 된다.** 금형은 그것을 재는 견본이다. 리팩터링 전 트리(`v2/main` f76d4d9 의 엔진)에서 떠 두었고, 카드마다
자기 트리에서 다시 떠서 견본과 맞춰 본다.

```
tests/seam/
  capture.sh       금형 뜨기: 엔진 빌드 → PostgreSQL 사본에 엔진·추적기 설치 → 임시 클러스터 → capture.py → 클러스터 삭제
  capture.py       무엇을 어떤 순서로 돌려 무엇을 남기나 (아래 「뜨는 것」)
  compare.py       두 금형을 사례 단위로 비교
  check.sh         관문: 이 트리를 떠서 golden/ 과 비교
  tracer/          og_mold_trace — 세션이 실행하는 모든 문장을 파일에 적는 시험 전용 PostgreSQL 모듈 (C, PGXS)
  golden/          견본. sql/ 은 그대로, trace/ 는 .xz 로 압축. SOURCE 에 어디서 떴는지
scripts/mold.sh    한 명령: check | capture | compare | bless
```

## 쓰는 법

필요한 것: PostgreSQL **16** 의 `pg_config`(견본이 16 으로 떠졌다. 17 로 떠도 같은지는 아래 「한계」), cargo-pgrx
0.19.2(엔진을 빌드할 때), C 컴파일러(추적기, PGXS), psycopg2 와 neo4j 드라이버가 있는 Python
(`tests/compat/run.py` 를 모듈로 불러 쓴다). Docker 는 쓰지 않는다.

```sh
# 관문 — 카드의 PG 포트를 준다(K2 28822, K3 28823 …). 엔진은 이 트리에서 빌드한다.
scripts/mold.sh check --pg-config <PG16>/bin/pg_config --port 28822 --python <venv>/bin/python

# scripts/test-all.sh 를 막 돌렸으면 그 엔진 패키지를 그대로 쓴다(빌드 생략)
scripts/mold.sh check --pg-config <PG16>/bin/pg_config --port 28822 --engine-pkg <test-all 작업 폴더>/pkg

# 다를 때 더 보기: 뜬 금형은 <work>/mold 에 남는다 (기본 <work> = ${TMPDIR:-/tmp}/og-mold-<port>)
scripts/mold.sh compare tests/seam/golden <work>/mold --show 20 --width 2000
scripts/mold.sh compare tests/seam/golden <work>/mold --layer sql          # sql 층만
```

종료 코드: **0** 같음, **1** sql 층이 다름, **3** sql 층은 같고 trace 층만 다름, **2** 사용법 오류이거나 뜨기 실패.
뜨는 데 이 맥에서 약 100초(엔진 빌드 별도), 클러스터는 끝나면 지운다(`--keep` 이면 남긴다).

포트는 카드마다 정해진 PG 포트를 쓴다. 기본값이 없다(통합 작업 트리·다른 카드와 부딪치지 않게). 금형은 Bolt
게이트웨이를 띄우지 않으므로 Bolt 포트는 쓰지 않는다.

`bless`(견본 다시 뜨기)는 관문 자체를 바꾸는 일이라 `OG_MOLD_BLESS=yes` 없이는 거절한다. **K2~K9 는 견본을 고치지
않는다**(카드 공통 규칙 3: 안 맞으면 코드가 틀린 것이다).

## 두 층

**sql 층** (`golden/sql/`) — 컴파일러가 내놓는 것. 카드가 「생성 SQL 이 글자까지 같다」고 할 때 재는 것이 이것이다.

- 정답집의 문장마다: `CHECK` = `og_cypher_check`, `COLS` = `og_cypher_columns`, `RUN` = `og_cypher` 실행 결과의
  행 수 또는 오류(SQLSTATE 와 메시지), `STATS` = `og_cypher_stats`, `SQL` = `og_cypher_sql`(쓰기 문장은
  「write queries are not compiled to a single statement」 오류가 정답이다). Bolt 게이트웨이가 부르는 순서 그대로
  부른 뒤 `og_cypher_sql` 을 부른다.
- SQL 회귀 파일·튜토리얼이 리터럴로 쓴 `og_cypher('<그래프>', <문자열>)` 마다 `og_cypher_sql` (파일이 끝난 뒤의
  그 DB 상태에서).
- TypeQL 스위트가 보내는 질의마다 `og_typeql_sql` (그 질의를 실행하기 바로 전 상태에서) 과 실행 성공 여부.

**trace 층** (`golden/trace/`) — 엔진이 그 일을 하며 **실행한 모든 문장**. 추적기(`tracer/og_mold_trace.c`)가
실행기 훅과 유틸리티 훅에서 적는다. 한 라인이 한 문장이다:

```
<깊이> <종류> <문장>
0 E SELECT og_cypher('social', $$MATCH (a:Person) …$$)      ← 클라이언트가 보낸 문장 (깊이 0)
1 E SELECT name, column_name, data_type FROM og_catalog.property …   ← 엔진의 SPI (깊이 1)
1 U CREATE OR REPLACE VIEW og_data.v_3 WITH (security_invoker = true) AS …
1 E SELECT jsonb_build_object(…) FROM og_data.v_3 n1 …        ← 컴파일된 SQL 이 실행될 때
2 E SELECT ty.type_id, … WHERE n.id = og_node_json.id  ×4      ← 그 안의 SQL 함수, 같은 문장 4번 연속
0 X 42P01 relation "…" does not exist                          ← 보고된 오류
```

종류: `E` 실행기 문장(SELECT/INSERT/UPDATE/DELETE), `U` 유틸리티 문장(CREATE/ALTER/SET/DO …), `X` 보고된 오류·경고
(SQLSTATE 메시지), `L` LOG 메시지(엔진의 `log!`). 줄바꿈·탭·역슬래시는 `\n` `\t` `\\` 로 적는다. 같은 라인이
연달아 나오면 한 번만 적고 `  ×N` 을 붙인다. `CREATE EXTENSION` 이 도는 동안은 적지 않는다.

trace 층에는 sql 층이 볼 수 없는 것이 들어 있다: **쓰기**(저장 API 가 실행하는 INSERT·UPDATE 와 그 순서 — K3·K4),
컴파일 중의 카탈로그 조회와 플래너 통계 조회, 뷰 만들기(`CREATE VIEW` 본문 — K2), 순회 함수 안의 SQL(K5), 실행
시점의 컴파일된 SQL(SQL 회귀 파일 안 DO 블록이나 `format()` 으로 만든 질의 포함).

## 뜨는 것

모두 PostgreSQL 16, 빈 클러스터에서 고정된 순서로 돈다. 같은 트리를 두 번 뜨면 파일 하나하나가 같다
(`scripts/mold.sh capture … --repeat 2`, 견본은 그렇게 확인하고 저장했다). 포트·작업 폴더가 달라도 같다.

| 원천 | 어떻게 | 사례 |
|---|---|---|
| 정답집 `tests/compat` | `targets/v1/bootstrap.sql` → 준비 데이터 → 읽기 → 쓰기 시나리오. run.py 와 같은 순서·같은 매개변수 규칙(그 모듈을 불러 쓴다). Bolt 대신 SQL 연결 하나로. 시나리오마다 트랜잭션 하나, 끝에 롤백, 실패한 단계는 세이브포인트로 되돌림 | 준비 638 + 읽기 652 + 쓰기 502 단계(시나리오 148) |
| SQL 회귀 `tests/common/sql` + `tests/v1/sql` | `tests/run.sh` 와 같은 순서로 파일마다 새 `og_test` 에 `CREATE EXTENSION` 후 `psql -f` (파일은 그대로) | 파일 30, 클라이언트 문장 1,356 (그중 og_cypher 호출 600), 리터럴 og_cypher 883 개 컴파일 |
| TypeQL `tests/v1/typeql` | run.py 를 그대로 돌리며 각 질의 앞에 `og_typeql_sql` | 질의 28 + 스키마·데이터 적재 |
| 튜토리얼 `examples/tutorials` | `build_tutorial.py --check --all` (dev 로, `tests/tutorials.sh` 와 같게) | 클라이언트 문장 511, Cypher 블록·리터럴 97 개 컴파일 |

사례 수: sql 층 2,951, trace 층 3,894(`golden/summary.json`). 견본 크기 2.8 MB(sql 층 2.0 MB 는 그대로, trace 층은
압축 전 약 78 MB 를 xz 로 0.8 MB).

시간 여행(`og.valid_at`·`og.system_at` 과 이력 테이블 `h_`), 패브릭(가상 노드·관계, 외부 테이블), 넓은 라벨 뷰
(`vp_`), 가변 길이(`og_vlp`·`og_reach`)가 모두 들어 있다(대부분 SQL 회귀 09·13·14·18·21·22·23·30 과 튜토리얼에서).

## 정규화

뜰 때마다 달라지지만 엔진 탓이 아닌 것만 지운다(`capture.py` 의 `Norm`):

- 클러스터 포트 → `<pgport>`, 작업 폴더·데이터 폴더 경로 → `<work>` (패브릭 서버 정의에 들어간다)
- 실행 시각 앞뒤 이틀 안의 시각(날짜와 시:분이 함께 있는 것) → `<now>`, 그 범위의 epoch 밀리초 → `<now:ms>`
  (회귀 파일이 `SELECT now() \gset` 으로 받은 값을 다음 문장에 넣는다)

클러스터는 TimeZone UTC, `autovacuum = off` 로 띄운다. 컴파일러가 가변 길이 홉을 `og_vlp` 로 할지 `og_reach` 로 할지
플래너 통계(`reltuples`)를 보고 정하므로, 도중에 autovacuum 이 돌면 생성 SQL 이 달라질 수 있다. 통계는 시험이
스스로 바꿀 때(ANALYZE, CREATE INDEX)만 바뀐다.

id·타입 id·OID 는 지우지 않는다. 빈 클러스터에서 같은 순서로 만들면 같기 때문이다. 그래서 **같은 순서**가 중요하다:
준비 순서를 바꾸거나 원천을 빼고 뜨면(`--only`) 그 뒤 사례의 id 가 달라질 수 있다. 관문은 언제나 전체로 뜬다.

## 다를 때

- **sql 층이 다르다(종료 1)**: 생성 SQL 이 바뀌었다. 카드의 「동작 변화 0」을 어긴 것이니 코드를 고친다. 견본은
  고치지 않는다.
- **trace 층만 다르다(종료 3)**: 컴파일러가 내놓는 SQL 은 같고, 엔진이 그 과정에서 실행한 문장이 다르다. 흔한 까닭은
  카탈로그 조회의 횟수·순서(같은 조회를 한 번 더 하거나 캐시해서 덜 함), 쓰기 문장의 순서, 뷰 정의.
  - K3 은 「쓰기의 SPI 문장 수·순서·잠금 순서를 바꾸지 않는다」가 카드에 있으니 같아야 한다.
  - K4 는 카드가 「`write.rs` 가 저장 API 를 거치면 문장 순서나 수가 달라질 수 있다. 문장 순서가 달라지는 곳은
    보고서에 적는다」고 했다. 다른 사례를 `compare --show` 로 뽑아 보고서에 붙이고 조정자의 판단을 받는다.
  - 그 밖의 카드는 까닭을 찾아 보고한다. 결과가 같다는 것은 정답집 `--compare` 와 실행기 요약으로 보인다.
- `compare.py` 는 사례(정답집의 `=== ` 블록, 또는 깊이 0 문장과 그 아래)를 짝지어 처음 다른 사례 몇 개를 보인다.
  사례의 목록 자체가 다르면(문장이 빠지거나 늘면) 그 위치를 보인다.

## 한계

- 견본은 PostgreSQL 16 으로 떴다. 관문은 16 으로 돈다(17 로 떠도 같은지는 K1 보고서에 적었다).
- Bolt 를 거치지 않는다. Python 스위트(`tests/common/bolt-*` 등)의 문장은 들어 있지 않다. Bolt 가 부르는 엔진
  함수(`og_cypher_check`·`og_cypher_columns`·`og_cypher`·`og_cypher_stats`)는 정답집 사례마다 부른다.
- trace 층은 SQL 함수 본문(`engine/sql/*.sql`)의 문장 텍스트도 담는다. 함수 본문 **안**에 주석을 넣으면 그 함수가
  인라인되지 않고 실행될 때 trace 가 달라질 수 있다(K5 의 분류 주석은 `CREATE` 문장 앞에 단다).
- 같은 클러스터 안에서 원천들은 차례로 돈다. TypeQL 스위트는 `/tmp/og_typeql_dump.tql` 에 스키마를 적었다가
  읽는다(스위트에 박혀 있다). 같은 시각에 다른 실행기가 같은 파일을 쓰면 엉킬 수 있다.
- 추적기는 USERSET 설정 하나로 아무 경로에나 파일을 쓰게 한다. 임시 클러스터 밖에 깔지 않는다.
