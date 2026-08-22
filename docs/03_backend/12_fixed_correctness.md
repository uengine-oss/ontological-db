# 반영된 수정 — 조용히 틀린 답을 내던 다섯 가지

> **이 문서가 답하는 질문**
> - 오류 없이 잘못된 답을 내던 결함 중 무엇이 고쳐졌나?
> - 각 수정이 동작을 어떻게 바꾸나 — 특히 **전에는 통과하던 질의가 이제 오류가 되는 곳**은?
> - 어떻게 검증했나?

> 개선 포인트 문서(`08_improvements_architecture.md`, `11_improvements_code.md`,
> `09_performance/07_improvements_performance.md`)는 감사 커밋 `7d60c82` 의
> 스냅샷이다. 아래 다섯 항목의 **현재 상태는 이 문서다.**

---

## 0. 요약

| ID | 항목 | 이전 동작 | 지금 |
|---|---|---|---|
| [ARCH-01](../01_architecture/08_improvements_architecture.md#arch-01) | `UNION` 이 파싱되고 무시됨 | 첫 분기만, 오류 없이 | 두 분기 모두 |
| [ARCH-02](../01_architecture/08_improvements_architecture.md#arch-02) · CODE-01 | 플랜 캐시가 스키마 변경에 무효화 안 됨 | 폐기된 뷰 참조 / 승격된 컬럼 누락 | 스키마 카운터가 캐시 키에 포함 |
| CODE-33 | 쓰기 질의에서 `WITH` 무시 | `LIMIT 1` 이 사라지고 전부 삭제 | 지켜지거나, 거절됨 |
| CODE-34 | `count(DISTINCT)` 가 `Vec::dedup()` | 연속 중복만 제거 → 틀린 수 | 값 단위 중복 제거 |
| [PERF-20](../01_architecture/09_performance/07_improvements_performance.md) | `min > 1` 에서 재작성이 다른 답 | 노드 누락 | `min > 1` 이면 재작성하지 않음 |

**모두 회귀 테스트가 붙었다** — [`engine/tests/sql/06_correctness_regressions.sql`](../../engine/tests/sql/06_correctness_regressions.sql).
수정 전 코드에서 다섯 단언이 **전부 실패하는 것을 확인했다**(6절). 회귀 테스트로서
의미가 있으려면 그것이 먼저다.

---

## 1. `UNION` — ARCH-01

파서는 처음부터 `Query.union` 을 채웠고, 그것을 읽는 코드가 없었다. 그래서
`MATCH … RETURN … UNION MATCH … RETURN …` 은 **첫 분기의 행만, 오류 없이**
돌려줬다. 버그가 취할 수 있는 최악의 형태다 — 답처럼 보이는 답이라서.

`compile_read` 가 이제 각 분기를 컴파일해 잇는다. 두 가지가 중요하다.

**분기를 각각 서브쿼리로 감싼다.** `build_select` 은 선행 `WITH` 를 낼 수
있는데 `WITH … SELECT … UNION WITH … SELECT …` 는 PostgreSQL 이 파싱하는
문장이 아니다. `FROM` 의 서브쿼리는 자기 `WITH` 를 가질 수 있으므로, 감싸는
것이 이 조합을 합법으로 만든다. 덤으로 분기별 생성 별칭이 서로에게 새지 않는다.

**분기마다 새 `Compiler`.** 별칭 카운터와 바인딩은 분기별 상태다.

**컬럼이 다르면 오류다.** 이름과 순서가 일치해야 하며, 아니면
`all branches of a UNION must return the same columns in the same order` 를 낸다.

> `bolt/README.md:75` 는 "UNION 은 실패한다"고 적혀 있었다. 실패하지 않았다.
> 이제 실패하지도, 틀리지도 않는다.

## 2. 플랜 캐시 — ARCH-02 / CODE-01

캐시 키가 `(graph, query)` 뿐이라 컴파일된 계획이 자기가 컴파일된 스키마보다
오래 살았다.

**드러나는 실패는 "없는 뷰" 오류보다 나쁘다.** 프로퍼티가 `__ext` 에서 자기
컬럼으로 승격되면, 캐시된 계획은 계속 `__ext ->> 'x'` 를 읽는데 새 쓰기는
`p_x` 로 간다. 질의는 성공하고 값만 조용히 사라진다. **승격은 평범한 쓰기에서
일어나므로 DDL 이 없어도 발생한다.**

`bump_schema_version` 에서 캐시를 비우는 방법은 **DDL 을 실행한 백엔드만**
고친다 — 캐시는 thread-local, 즉 백엔드마다 하나다. 그래서 버전을 키에 넣었다.

비용은 질의당 시퀀스 `last_value` 조회 1회다. O(1) 이고, `max(version)` 을
테이블에서 읽는 방식과 달리 스키마 변경 횟수에 따라 커지지 않는다. **캐시 히트
경로가 더 이상 무료가 아니다** — 정확성의 값이고, 숨기지 않는다.

> 이 때문에 `og_grant` 의 `read` 레벨이 `og_catalog` 시퀀스에 `SELECT` 를
> 받는다. `USAGE` 가 아니라 `SELECT` 다 — `last_value` 를 읽는 것은 시퀀스를
> 당길 권한이 아니다.

## 3. 쓰기 경로의 `WITH` — CODE-33

읽기 부분을 `take_while(Match | Unwind)` 로 모았으므로 `WITH` 에서 멈췄고,
`WITH` 는 쓰기 루프의 `_ => {}` 로 떨어졌다. **`MATCH (n) WITH n LIMIT 1
DELETE n` 이 라벨 전체를 지웠다.**

지금은 `WITH` 가 읽기 부분에 포함된다. 지켜지는 것과 거절되는 것의 경계는 이렇다.

| 지켜진다 | 거절된다 |
|---|---|
| `DISTINCT`, `ORDER BY`, `SKIP`, `LIMIT` | 표현식 (`WITH n.name AS x`) |
| 뒤따르는 `WHERE` | 별칭 변경 (`WITH n AS m`) |
| 평범한 변수 나열 (`WITH n, m`) | 집계 (`WITH count(n) AS c`) |
| | `WITH` 두 개 이상 |
| | `WITH` 뒤에 또 `MATCH` |

경계의 근거: 뒤따르는 쓰기 절은 **패턴 변수**를 가리킨다. 변수를 이름 바꾸거나
계산하는 투영은 `DELETE n` 이 가리킬 대상을 남기지 않는다.

**거절이 핵심이다.** 위 오른쪽 열은 전부 예전에 옆에 있던 `LIMIT` 과 함께 조용히
버려지던 것들이다. 이제는 오류다.

## 4. `count(DISTINCT)` — CODE-34

`Vec::dedup` 은 **연속된** 중복만 지운다. 행은 계획이 낸 순서대로 오므로
`count(DISTINCT x)` 는 값이 아니라 **구간(run)** 을 셌다. `[a,b,a,b]` 에서 4.

`Value` 는 `Hash` 도 `Ord` 도 구현하지 않는다. serde_json 의 맵은
`preserve_order` 없이는 `BTreeMap` 이라 직렬화가 정규형이므로, 텍스트 형태를
동일성으로 쓴다. 첫 등장 순서는 보존된다 — `collect(DISTINCT x)` 가 돌려줘야
하는 것이 그것이다.

## 5. `*min..max` 에서 `min > 1` — PERF-20

`og_vlp` 는 트레일을 열거하므로 "길이가 `[min, max]` 인 걸음이 있는가"에
답한다. `og_reach` 는 방문집합 BFS 라 노드를 **최단 거리에서 한 번** 낸다.
한 홉 거리이면서 세 홉으로도 닿는 노드는 깊이 1 에서 visited 로 표시되고,
`1 >= 2` 가 거짓이라 방출되지 않고, 다시 고려되지 않는다. **재작성이 노드를
조용히 잃는다.**

이제 `min <= 1` 일 때만 재작성한다. `min = 0` 과 `min = 1` 에서는 두 함수가
같은 답을 낸다 — 최단 거리 `d` 로 닿는 노드는 길이 `d` 의 걸음으로도 닿고,
`d` 는 구간 안에 있다.

**회귀 스위트가 `*1..k` 만 써서 이것은 한 번도 관측된 적이 없었다.** 그래서
새 테스트가 `*2..4` 를 쓴다.

성능 대가는 실재한다: `*2..k` 깊은 순회는 이제 항상 트레일 열거다. 답을 바꾸는
최적화는 최적화가 아니므로 그것이 옳은 거래이지만, `og_reach` 가 `min` 을
제대로 다루도록 고치는 것이 후속 작업으로 남아 있다 — 노드마다 "`min` 이상의
어떤 깊이에서 닿는가"를 추적해야 하고, 그건 다른(더 어려운) 계산이다.

---

## 6. 검증

컨테이너에서 PostgreSQL 15 + pgvector 에 확장을 실제로 빌드·설치하고
(`cargo pgrx install`) 스위트를 돌렸다. 기준선(`e19cfbb`, 수정 직전)과
수정본을 같은 데이터베이스에서 번갈아 설치해 비교했다.

**수정 전 — 다섯 단언이 전부, 예측한 증상 그대로 실패한다:**

```
ERROR:  UNION ALL: expected 6 rows, got 4
ERROR:  count(DISTINCT) on the write path: expected 2, got 4
ERROR:  relation "og_data.v_1" does not exist
ERROR:  *2..4 lost t1: reachable in four hops but marked visited at one
ERROR:  WITH ... LIMIT 1 DELETE removed 2 of 2 nodes
```

**수정 후 — 여섯 파일 전부 통과** (`01`~`06`, 기대 오류 표시와 정확히 일치).

`RAISE EXCEPTION` 이 곧 스위트 실패인 이유는 `tests/run.sh` 가 ERROR 줄 수를
기대 오류 표시 개수와 비교하기 때문이다. 즉 **기존 하네스를 고치지 않고도 이
파일은 게이트로 동작한다.** `04_neo4j_compat.sql` 도 같은 방식으로 단언하고
있었다 — 하네스가 단언을 못 본다는 서술은 정확히는 *대부분의 파일이 단언하지
않는다* 는 뜻이다. `01`, `02`, `03`, `05` 는 여전히 출력만 한다.

> **함정 하나를 기록해 둔다.** 이 검증 중 `04_neo4j_compat.sql` 이 실패했는데,
> 원인은 코드가 아니라 **데이터베이스 로케일**이었다. `initdb` 가 C 로케일로
> 돌면 `to_tsvector('simple', …)` 가 한글을 토큰화하지 못해
> `db.index.fulltext.queryNodes` 가 0행을 낸다. UTF-8 로케일로 다시 세우면
> 통과한다. 테스트도 문서도 이 의존성을 어디에도 적어두지 않았다.

관련: [11_improvements_code.md](11_improvements_code.md) ·
[04_cypher_compiler.md](04_cypher_compiler.md) ·
[../07_security/10_fixed.md](../07_security/10_fixed.md)
