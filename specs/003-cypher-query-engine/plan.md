# Implementation Plan: 네이티브 사이퍼 질의 엔진

**Branch**: `003-cypher-query-engine` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

Cypher를 자체 파서로 AST로 만들고, **PostgreSQL 플래너가 그래프 패턴 전체를 볼 수 있는
평범한 SQL로 컴파일**한다. 실행 계획·조인 순서·병렬성·통계는 PostgreSQL 옵티마이저가
결정한다.

**핵심 설계 결정**

1. **컴파일 타깃은 SQL이지 함수 파이프라인이 아니다.** `MATCH (a:Person)-[:KNOWS]->(b)` 는
   `og_data.v_<person>` 뷰와 `og_data.og_adj` 위의 조인이 된다. 옵티마이저가 어느 쪽에서
   시작할지, 해시 조인일지 중첩 루프일지 **실제 통계로** 고른다. 우리가 조인 순서 규칙을
   따로 만들지 않는 이유다 — PostgreSQL이 우리보다 잘한다.
2. **라벨은 컴파일 타임에 사라진다.** 002의 구간 인덱스로 후손 타입 목록을 한 번 뽑아
   구체 타입 테이블들의 `UNION ALL` 뷰로 바꾼다. 실행 시점에 계층 판정 비용은 **0**이며,
   플래너는 각 구체 테이블의 통계를 개별적으로 본다.
3. **읽기는 SQL 한 문장, 쓰기는 절차적.** `CREATE`/`MERGE`/`SET`/`DELETE` 는 읽기 부분이
   만든 바인딩 위에서 Rust가 순차 적용한다. 저장 계층(001)이 세 구조를 한 트랜잭션 안에서
   맞춰야 하기 때문이며, 정확성이 성능보다 우선한다.
4. **파라미터는 바인딩된다.** 사용자 `$param` 은 단일 jsonb 인자에서 추출되며, 비교 대상
   컬럼의 선언 타입으로 캐스팅된다 → 인덱스가 살아 있고, 주입이 불가능하다.

## Technical Context

**Language**: Rust / pgrx. 파서·컴파일러 전부 Rust.

**Testing**: `cargo test`(파서 단위), SQL 회귀, openCypher TCK 부분집합

**Performance Goals**: AGE 대비 기하평균 10배, 2회차 실행에서 파싱·컴파일 오버헤드 1% 미만

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I | ✅ | 확장 함수 + 표준 SQL. 커널 패치 0 |
| **II. Cypher 1급** | ⚠️ 부분 | AST/계획/옵티마이저 통합은 달성. **최상위 문장 문법은 미달** — 아래 Complexity Tracking |
| III | ✅ | 인접 세그먼트를 직접 조인 |
| IV | ✅ | 상속은 컴파일 타임 구간 인덱스로 해소. 재귀 0 |
| V | ✅ | `vector.*` 함수가 pgvector 연산자로 컴파일 |
| VIII | ✅ | 미지원 구문·미지 라벨에 후보 제안 오류 |
| IX | ✅ | 쓰기는 호출자 트랜잭션에서 실행 |

## Architecture

```
engine/src/cypher/
├── lexer.rs     # 토크나이저 (유니코드 문자열·주석·파라미터)
├── ast.rs       # AST + 집계 판별 + 컬럼 이름 규칙
├── parser.rs    # 재귀 하강. 미지원 구문은 명시적 오류
├── views.rs     # 타입 → 구체 테이블 UNION 뷰 (라벨 해소의 물리적 형태)
├── compile.rs   # AST → SQL. 타입 힌트 기반 파라미터 캐스팅
├── eval.rs      # 쓰기 절의 표현식 평가기
└── mod.rs       # 실행, 컴파일 캐시, EXPLAIN, 감사
```

**컴파일 예시** (실제 출력)

```cypher
MATCH (a:Person)-[:KNOWS]->(b:Person) WHERE a.age > 30 RETURN b.name
```
```sql
SELECT jsonb_build_object('b.name', t.c0) AS row FROM (
SELECT n2.p_name AS c0 FROM og_data.v_2 n1
  CROSS JOIN og_data.v_2 n2
  CROSS JOIN LATERAL (SELECT u.nbr, u.eid FROM og_data.og_adj adj3,
                      LATERAL unnest(adj3.nbr, adj3.eid) AS u(nbr, eid)
                      WHERE adj3.src = n1.id AND adj3.dir = 'o'::"char"
                        AND adj3.etype = ANY(ARRAY[4]::int4[])) u4
 WHERE n2.id = u4.nbr AND (n1.p_age > 30)
) t
```

`p_age > 30` 이 실제 컬럼 술어라는 점, 라벨이 이미 `v_2` 로 해소되었다는 점, 인접 확장이
플래너에게 보이는 조인이라는 점이 AGE와의 차이 전부다.

**컴파일 캐시**: `(graph, query)` → SQL. 파싱·컴파일이 반복 비용의 대부분이고, 결과 SQL의
플랜 캐싱은 PostgreSQL이 담당한다.

## Phasing

| Phase | 내용 | 상태 |
|-------|------|------|
| P0 | 렉서·파서·AST | ✅ |
| P1 | MATCH/WHERE/RETURN/ORDER/LIMIT, 프로퍼티·파라미터 | ✅ |
| P2 | 관계 패턴, 방향, 가변 길이(trail) | ✅ |
| P3 | 집계, DISTINCT, 전체 노드 투영 | ✅ |
| P4 | CREATE/MERGE/SET/DELETE | ✅ |
| P5 | OPTIONAL MATCH, UNWIND | 부분 |
| P6 | WITH 체이닝, UNION | 미착수 |
| P7 | 리소스 상한, 취소 지점 | 008에서 |
| P8 | Custom Scan 전환 | v2 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| **원칙 II: 진입점이 여전히 함수 호출** (`og_cypher('g', $$…$$)`) | PostgreSQL 16에는 최상위 문장 문법을 확장이 교체할 수 있는 지원 훅이 없다. raw parser 교체는 커널 패치를 요구하며 원칙 I(NON-NEGOTIABLE)과 정면 충돌한다. 원칙 I이 이긴다 | 그러나 AGE와 달리 **원칙 II의 실질(옵티마이저 가시성, 파라미터 바인딩, 계획 캐시, 표준 타입 반환)은 모두 달성**했다. `og_cypher_sql()` 이 컴파일된 SQL을 그대로 내주므로 사용자는 그것을 뷰·CTE·조인에 직접 넣어 완전한 SQL 통합을 얻는다. v2에서 `GRAPH_TABLE` 스타일 SQL 내장 문법(SQL/PGQ)을 추가해 이 간극을 좁힌다 |
| **WITH 미지원** | 다중 스코프 체이닝은 컴파일러에 스코프 스택을 요구하며, P0~P4 정확성을 먼저 확보하는 편이 낫다고 판단 | 조용히 잘못 해석하는 것보다 명시적 오류가 낫다(원칙 VIII). 오류 메시지가 대안(SQL 서브쿼리)을 안내한다 |
| 쓰기 절의 절차적 실행 | 001 FR-012가 인접 양방향 갱신을 같은 트랜잭션에 요구. 단일 SQL로 표현하면 트리거나 CTE 부작용에 의존하게 되어 검증이 어려워진다 | 대량 쓰기 성능은 `og_create_node`/`og_create_edge` 벌크 경로로 별도 대응 |
