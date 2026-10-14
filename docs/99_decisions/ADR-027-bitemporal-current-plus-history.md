# ADR-027: 이중 시간은 현재 테이블 + `h_<type_id>` 이력 테이블로 저장하고, 시점은 생성 뷰가 읽는 세션 설정으로 지정한다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-09-30 (작성일. 커밋 전) |
| 영향 범위 | storage, cypher, typeql, bolt, catalog |
| 근거 | `engine/src/temporal/mod.rs`, `engine/sql/access.sql`(`og__temporal`, `og_valid_at`, `og_system_at`, `og_asof_active`, `og_require_current`), `engine/src/cypher/views.rs`(`build_view`), `engine/src/cypher/compile.rs`(`join_rel`, `guard_untyped_nodes`), `engine/tests/sql/09_bitemporal.sql`, `engine/tests/sql/10_evolution_temporal_coverage.sql`, [spec 013](../../specs/013-bitemporal-graph/plan.md) |

> **이 문서가 답하는 질문**
> - 유효 시각과 기록 시각은 어디에 저장되는가?
> - 질의는 시점을 어떻게 말하고, 왜 Cypher 문법이 아닌가?
> - 과거를 볼 수 없는 질의는 어떻게 되는가?

## 배경

스펙 008 의 이력(`og_enable_history`, `og_history`, `og_as_of`)은 기록 시각 하나만 가진 엔티티
단위 변경 로그였고, "엔티티 하나를 과거 시각으로" 되돌려 볼 수 있을 뿐 **질의 전체**를 과거에
실행할 수 없었다. 요구는 이것이었다: 사용자는 "이 시점이다"라는 옵션만 주고, 같은 Cypher 가
그 시점의 그래프에 답하며, 유효 시각(현실에서 참이던 때)과 기록 시각(우리가 그렇게 알던 때)이
분리된다([spec 013](../../specs/013-bitemporal-graph/spec.md) 입력).

## 고려한 선택지

**저장**

1. **한 테이블에 모든 버전.** 행마다 유효·기록 구간. 단순하지만 현재만 묻는 대부분의 질의가
   과거 버전을 거르고, 인덱스와 통계가 이력에 희석되며, `og_node`/`og_adj` 와 1:1 이던 저장
   테이블의 의미가 깨진다.
2. **현재 테이블 + 이력 테이블**(SQL:2011 시스템 버전 테이블의 모양). 현재 테이블은 이전과 같은
   행 수와 의미를 유지하고, 대체된 믿음과 여전히 믿는 과거 유효 구간만 `h_<type_id>` 로 간다.
3. **Fact 노드로 모델링.** 가이드가 Neo4j 의 관행으로 소개한 방식. 사용자 모델이 시간 때문에
   바뀌고, 질의가 시점을 직접 다뤄야 한다 — 요구와 반대다.

**시점 지정**

- **a. Cypher 확장 문법**(`MATCH … AT TIME t`). 파서·컴파일러·Bolt·TypeQL 모두 바뀌고, 시점이
  SQL 텍스트에 들어가 계획 캐시가 시점마다 갈라진다.
- **b. 컴파일 시 시점을 SQL 에 상수로 삽입.** 같은 문제(캐시).
- **c. 세션 설정.** `og.valid_at` / `og.system_at` 을 STABLE 함수 `og_valid_at()` /
  `og_system_at()` 가 읽고, 생성 뷰의 필터가 그 함수를 부른다.

## 결정

**2안 + c.**

- `og_enable_temporal(graph, type)` 은 타입과 하위 타입(이후 생성 포함)의 저장 테이블에
  `__valid tstzrange`, `__sys_from timestamptz` 를 추가하고(기존 행: 유효 `[-∞, ∞)`, 기록 시작 =
  활성화 시각), 같은 컬럼 + `__sys_to`, `__op` 를 가진 `og_data.h_<type_id>` 를 만들고, BEFORE
  트리거 `og__temporal()`(SECURITY DEFINER)을 건다.
- 갱신·삭제는 옛 행을 기록 구간을 닫아 이력에 넣고(`u`/`d`), 변경 시점 X 이전의 유효 구간을
  현재 믿음으로 다시 넣는다(`v`). X 는 `og.valid_from` 또는 `now()`; `'-infinity'` 는 정정.
- 생성 타입 뷰는 temporal 타입에 대해 세 분기의 합집합이다: `현재 테이블 WHERE __valid @>
  og_valid_at() AND __sys_from <= og_system_at()` ∪ `이력 WHERE __sys_to = 'infinity' AND
  __valid @> og_valid_at() AND …`(열린 행 — 여전히 믿는 유효 구간 조각, 늘 읽는다) ∪
  `이력 WHERE og_asof_active() AND __sys_to <> 'infinity' AND … AND __sys_to > og_system_at()`
  (닫힌 행 — 대체된 믿음, 시점이 있을 때만). 열린 행은 부분 GiST 인덱스
  `ON h_<tid> USING gist (__valid) WHERE __sys_to = 'infinity'` 로 찾는다. **컴파일된 SQL 은
  시점과 무관하다** — 09 가 이를 단언한다.
- **현재만 볼 수 있는 질의 모양은 거부한다.** 레지스트리(`og_node`)와 인접 세그먼트(`og_adj`)는
  현재만 안다. 레이블 없는 노드 패턴(temporal 노드 타입이 있는 그래프), temporal 관계 위 가변
  길이 경로, 단일 타입이 아닌 관계 변수, TypeQL 에는 `og_require_current(what)` 이 들어가고,
  시점이 설정되어 있으면 `OG_TEMPORAL_UNSUPPORTED`(0A000). temporal 관계 타입의 단일 홉은
  세그먼트 대신 간선 테이블(같은 세 분기)을 읽어 **답한다.**

## 근거

- **틀린 답 대신 오류.** 008 FR-021("이력이 없으면 현재 값을 대신 반환해서는 안 된다")의 질의
  전체 판이다. 오늘의 레지스트리로 과거 질문에 답하면 그럴듯하게 틀린다.
- **계획 캐시가 시점을 모른다.** 캐시 키는 스키마 버전이고(003/008), 시점은 실행 시 함수
  호출로만 들어간다.
- **트리거라서 원시 SQL 도 버전이 남는다.** 09 는 원시 `UPDATE og_data.n_1 …` 이 이력에
  남고 그 이전 기록 시각에서 옛 값이 보임을 단언한다.
- **이력은 기록이다.** 쓰기 역할은 `h_*` 에 DML 을 갖지 않고(`catalog/privileges.rs`
  `protect_records`, `apply_to_table` 의 `record_only`), 트리거가 소유자 권한으로 쓴다.
- **Bolt 에서도 같은 설정.** 드라이버가 이미 가진 `tx_metadata` 를 `set_config(…, true)` 로
  옮기므로 새 API 가 없다(`tests/bolt-context/run.py`).

## 결과

**긍정적**

- Cypher 한 글자도 바뀌지 않는다. 시점이 없으면 닫힌 이력 분기는 한 번의 거짓 필터로 잘린다.
- 미래 유효 시각의 갱신·삭제가 현재를 가리지 않는다. 그때까지 유효한 옛 값은 열린 `v` 이력
  행에 있고, 열린 행은 현재의 일부로 읽힌다(09 가 단언).
- 이력 테이블은 일반 힙 테이블이라 MVCC·WAL·`pg_dump` 를 그대로 상속한다(원칙 IX).

**부정적 / 감수한 대가**

- **temporal 관계 타입의 홉은 인접 세그먼트를 쓰지 않는다** — 시점 설정 여부와 무관하게.
  세그먼트로 되돌리는 분기는 "컴파일된 SQL 이 시점과 무관"과 충돌한다. 원칙 III 에서의 부분
  이탈이며 비용은 측정하지 않았다.
- **현재 분기에도 필터 비용이 있다**(`__valid @> now()`).
- **시점이 없는 질의도 열린 이력 분기를 탐색한다.** temporal 타입마다 부분 GiST 인덱스 탐색
  한 번이다. 열린 행은 유효 구간을 쪼갠 변경의 앞 조각뿐이라 보통 소수다. 처음 설계는 이력
  전체를 `og_asof_active()` 로 잘랐는데, 그러면 미래 유효 시각의 변경 뒤 그 엔티티가 시점 없는
  질의에서 사라졌다 — 이 비용은 그 결함을 없앤 대가다.
- **가변 길이 경로, 레이블 없는 스캔, TypeQL 은 시점 질의를 지원하지 않는다**(거부).
- 보존 정책이 없다. 이력은 무한히 자란다.
- 과거 유효 구간(`v` 행)을 소급 편집할 방법이 없다.

## 재검토 조건

- 가변 길이 경로의 시점 질의가 요구될 때 — `og_reach`/`og_vlp` 가 세그먼트 대신 시점 필터된
  간선 관계를 받아야 하며, [ADR-012](ADR-012-visited-set-bfs-rewrite.md)/[ADR-013](ADR-013-conservative-bfs-rewrite.md)
  의 손익분기를 다시 계산해야 한다.
- temporal 관계 타입의 홉 비용이 측정되어 세그먼트 대비 유의미하게 나쁠 때 — 시점이 없을 때의
  별도 계획(스키마 버전 대신 "시점 활성 여부"를 캐시 키에 포함)을 검토한다.
- PostgreSQL 이 SQL:2011 시스템 버전 테이블을 기본 제공하게 되면 트리거를 대체한다.

<!-- affects: storage, cypher, typeql, bolt -->
