# 10. 데이터 모델 / 인덱스 / 성능 개선 포인트

> **이 문서가 답하는 질문**
> - 현재 스키마와 쓰기 경로에서 무엇이 실제로 문제인가?
> - 각 문제의 근거(파일:라인)와 구체적인 수정안은 무엇인가?
> - 어떤 순서로 손대야 하는가?

**전제**: 아래 모든 항목은 **실제 코드를 읽고** 도출했다. 근거 없는 일반론은 없다.
예상 효과에 수치가 붙은 것은 근거를 명시했고, 측정하지 않은 것은 **"미측정"**으로 표시했다.

**심각도 기준**
- **High** — 데이터 손실/오염, 확장성 상한, 또는 흔한 운영 시나리오에서의 심각한 성능 저하
- **Med** — 특정 워크로드에서 문제, 또는 운영 부담
- **Low** — 정합성/명료성 개선, 소폭 최적화

---

## 요약 표

| ID | 제목 | 심각도 | 근거 (파일:라인) | 현상 | 제안 | 예상 효과 | 리스크 |
|---|---|---|---|---|---|---|---|
| DATA-01 | `og_id_alloc` UPSERT의 행 단위 직렬화 | **High** | `engine/src/storage/mod.rs:25-32` | 같은 타입의 동시 삽입이 커밋까지 완전 직렬화 | 타입별 시퀀스 또는 블록 예약 | 동일 타입 병렬 쓰기 확장 | 롤백 시 id 갭 발생 |
| DATA-02 | `og_adj`에 `dir`/길이 CHECK 제약 없음 | Med | `engine/sql/bootstrap.sql:197-206` | 코드 버그가 조용히 손상된 세그먼트를 남김 | `CHECK` 두 개 추가 (`NOT VALID` → `VALIDATE`) | 손상이 쓰기 시점에 실패 | 기존 손상 데이터가 있으면 VALIDATE 실패 |
| DATA-03 | `og_adj(etype)` 인덱스 부재 | Med | `engine/src/catalog/types.rs:706` | `og_drop_type`가 서브타입마다 전체 스캔 | `etype = ANY($subs)`로 1회 스캔 (권장) 또는 인덱스 추가 | N회 스캔 → 1회 | 인덱스 추가 시 쓰기 비용↑ |
| DATA-04 | `type_label_lft_idx`가 `range_idx`의 접두사 | Low | `engine/sql/bootstrap.sql:79-80` | 중복 인덱스 — 읽기 이득 0, 쓰기 비용만 | `bootstrap.sql`에서 제거 | 라벨 재계산 쓰기 감소 | 확장 멤버라 기존 설치는 `ALTER EXTENSION ... DROP INDEX` 필요 |
| DATA-05 | FK 참조 컬럼 4개에 인덱스 없음 | Med | `engine/sql/bootstrap.sql:111,119,138,152` | 타입 삭제 시 자식 테이블 전체 스캔 + `NO ACTION` 실패 | 부분 인덱스 4개 추가 | 카탈로그 삭제 잠금 범위 축소 | 거의 없음 (작은 테이블) |
| DATA-06 | `og_drop_graph`가 `og_data`를 남김 | **High** | `engine/src/catalog/types.rs:321-340` | 그래프 삭제 후 노드/엣지/인접 세그먼트 전부 잔존 | 함수에 `og_data` 정리 추가 | 디스크 회수, 고아 제거 | 잘못 구현하면 다른 그래프 데이터 삭제 |
| DATA-07 | 삭제 경로가 부수 테이블을 정리 안 함 | Med | `engine/src/storage/mod.rs:355-383`, `engine/src/catalog/types.rs:700-709` | `og_embedding_state`/`og_source`/`og_iri`/`og_role_player` 고아 누적 | 삭제 경로 확장 + 정리 함수 노출 | 무한 증가 차단 | `og_history`는 의도적으로 유지해야 함 |
| DATA-08 | `__ext` jsonb에 인덱스가 전혀 없음 | **High** | `engine/src/cypher/compile.rs:989-991` | 미선언 프로퍼티 필터가 항상 전체 스캔 | 동등 비교를 `@>`로 컴파일 + GIN, 또는 표현식 인덱스 API | 미선언 프로퍼티 필터 인덱스화 | 컴파일러 변경 범위 큼 |
| DATA-09 | `og_adj` 파티셔닝 미적용 | Med | `engine/sql/bootstrap.sql:197-206` | 단일 거대 테이블 — VACUUM/락/스캔이 전역 | `LIST(dir)` 2파티션(안전) 또는 `HASH(src)` | 유지보수 병렬화, 프루닝 | `pg_extension_config_dump`를 파티션마다 등록해야 함 |
| DATA-10 | 라벨링이 항상 전체 재계산 (`insert_between` 부재) | Med | `engine/src/catalog/labeling.rs:114-116, 154-167` | 타입 하나 추가 = 그래프 전체 재라벨 + 전 뷰 드롭 + 라벨행당 SPI 1문장 | 다중 행 INSERT로 우선 축소, 이후 증분 구현 | 대규모 온톨로지 DDL 지연 감소 | 증분 구현은 정합성 위험 큼 |
| DATA-11 | 18비트 타입 공간 소진 경로가 열려 있음 | Med | `engine/sql/bootstrap.sql:47`, `engine/src/catalog/types.rs:210-231` | Cypher가 미지 라벨을 자동 타입화 → 262,143 소진 가능 | 소프트 임계 경고 + 압박 뷰 노출 | 조용한 전면 중단 방지 | 자동 타입화를 막으면 Neo4j 호환성 훼손 |
| DATA-12 | text widening의 `ALTER` 실패가 무시됨 | Med | `engine/src/storage/mod.rs:138-140` | 카탈로그는 `text`, 컬럼은 `int8`로 어긋남 | `unwrap_or_else(|e| error!(...))`로 변경 | 조용한 스키마 불일치 제거 | 실패 시 쓰기가 중단됨 (올바른 동작) |
| DATA-13 | HNSW 파라미터(`m`, `ef_construction`) 설정 불가 | Med | `engine/src/vector/mod.rs:58-61` | 항상 pgvector 기본값. 튜닝 불가 | `og_add_embedding`에 옵션 인자 추가 | 리콜/지연 튜닝 가능 | 시그니처 변경 (기본값으로 흡수 가능) |
| DATA-14 | `og_history_valid_idx` 사용처 없음 | Low | `engine/sql/bootstrap.sql:322` | 이력 쓰기마다 유지되지만 읽는 질의가 없음 | `(entity_id) WHERE valid_to IS NULL` 부분 인덱스로 교체 | 트리거 UPDATE 가속 + 인덱스 축소 | 사용자 SQL이 쓰고 있을 수 있음 |
| DATA-15 | 임베딩 `metric` 변경 시 인덱스 미갱신 | Med | `engine/src/vector/mod.rs:56-74` | 카탈로그만 바뀌고 opclass는 그대로 → 조용히 순차 스캔 | 지표 변경 감지 시 `DROP INDEX` 후 재생성 | 조용한 성능 붕괴 제거 | 인덱스 재구축 시간 |
| DATA-16 | `og_catalog.setting` 시드 4키가 읽히지 않음 | Low | `engine/sql/bootstrap.sql:256-260` vs `engine/src/storage/adjacency.rs:15` | `chunk_size`를 바꿔도 아무 효과 없음 | 실제로 읽거나, 시드 제거하고 문서화 | 잘못된 기대 제거 | 없음 |
| DATA-17 | 물리 컬럼 이름 충돌 | Low | `engine/src/catalog/types.rs:53-66` | `created-at`/`created.at`/`created at`가 모두 `p_created_at` | 충돌 시 접미사 부여 + 카탈로그 UNIQUE | 프로퍼티 조용한 병합 방지 | 기존 컬럼명 유지 필요 (마이그레이션) |
| DATA-18 | `og_edge_src_idx` / `og_edge_dst_idx` 저활용 | Low | `engine/sql/bootstrap.sql:240-241` | 순회는 `og_adj`를 쓰므로 이 두 인덱스가 거의 안 쓰임 | `(type_id, src, dst)` 복합으로 교체 검토 | 인덱스 2개 → 1개 | 사용자 SQL이 쓰고 있을 수 있음 |
| DATA-19 | `find_role`이 `rel_type_id DESC`로 구체성 판단 | Low | `engine/src/typeql/schema.rs:400-409` | id 순서 = 상속 깊이라는 가정. 반례 가능 | `type_label.depth DESC`로 정렬 | 역할 해석 정확도 | 라벨이 최신이어야 함 |
| DATA-20 | TypeQL 속성 인터닝 경합 | Med | `engine/src/typeql/write.rs:242-263` | SELECT-then-INSERT, `ON CONFLICT` 없음 → 동시 삽입 시 트랜잭션 실패 | `INSERT ... ON CONFLICT (val) DO UPDATE ... RETURNING id` | 동시 쓰기 안정화 | `DO UPDATE`가 불필요한 행 버전 생성 |
| DATA-21 | `og_audit` / `og_history` 보존 정책 없음 | Med | `engine/sql/bootstrap.sql:310-322, 380-390` | 무한 증가. 파티션도 TTL도 없음 | `at` / `recorded_at` 기준 범위 파티셔닝 + 정리 잡 | 디스크 예측 가능 | 파티션도 dump 등록 필요 |
| DATA-22 | `$has` 저장 테이블에 `__ext` 컬럼 없음 | Low | `engine/src/typeql/schema.rs:538-540` vs `engine/src/cypher/views.rs:117` | 뷰 빌더가 `__ext`를 투영하므로 `ve_<$has>` 생성이 실패할 수 있음 (미확인) | `$has` 테이블에 `__ext jsonb` 추가 | 두 언어의 테이블 형태 일관화 | 거의 없음 |
| PERF-01 | `og_adj` fillfactor 80이 큰 세그먼트에 무효 | Med | `engine/sql/bootstrap.sql:206` | 4.2KB 튜플은 HOT 갱신 불가 → 80%는 낭비 | PERF-02 해결 후 `fillfactor = 100` | 페이지 20% 회수 | 차수 분포에 따라 역효과 |
| PERF-02 | `og_adj` append의 쓰기 증폭 | **High** | `engine/src/storage/adjacency.rs:22-30` | 이웃 1개 추가 = 세그먼트 전체 재작성. 256개 채우기에 약 552KB 기록 | 배치 append API + `og_rebuild_adjacency()` 노출 | 벌크 로드 쓰기량 약 1/100 (계산치, 미측정) | API 추가, 기존 경로 유지 필요 |
| PERF-03 | append마다 `max(seq)` 상관 서브쿼리 | Med | `engine/src/storage/adjacency.rs:25-26, 37-38` | 이웃 1개당 추가 인덱스 스캔 1~2회 | 꼬리 `seq`를 백엔드 로컬 캐시 또는 `n < CHUNK` 조건만으로 갱신 | 엣지 생성 지연 감소 (미측정) | 동시성 하에서 캐시 무효화 필요 |
| PERF-04 | `remove`가 무조건 두 번째 DELETE 실행 | Low | `engine/src/storage/adjacency.rs:66-71` | 빈 세그먼트가 없어도 매번 문장 1개 | UPDATE의 `RETURNING n`이 0일 때만 실행 | 엣지 삭제 문장 2개 → 1개 | 없음 |
| PERF-05 | `og_reorganize`가 단일 트랜잭션 + 대상당 1문장 | Med | `engine/src/storage/stats.rs:143-165` | 대규모 재구성이 긴 트랜잭션 + 대량 죽은 튜플 | 배치 크기 인자 + 집합 기반 재작성 | 트랜잭션 길이 제어 | 중간 상태 노출 (읽기는 MVCC로 안전) |
| PERF-06 | `v_<tid>` 합집합 뷰의 폭 | Med | `engine/src/cypher/views.rs:99-137` | 서브타입 × 프로퍼티 만큼 `NULL::<type>` 표현식 | 참조된 컬럼만 담는 좁은 뷰 변형 | 계획 시간 감소 (미측정) | 컴파일러가 필요한 컬럼을 알아야 함 |
| PERF-07 | 순회 함수의 `ROWS` 추정치 하드코딩 | Low | `engine/sql/access.sql:16,31,140,197` | `og_reach`(Rust)는 인라인 불가라 항상 100으로 추정 | 플래너 support 함수 또는 통계 기반 조정 | 조인 순서 개선 (미측정) | 잘못 조정하면 회귀 |
| PERF-08 | `og_node_json()`이 행마다 4개 서브질의 | **High** | `engine/sql/access.sql:208-235`, `engine/src/cypher/compile.rs:991` | 타입 미상 프로퍼티 읽기·`og_typeql_attribute` 스캔이 행당 plpgsql 호출 | 타입이 알려진 경로를 넓히고, 뷰는 `val` 직접 읽기로 | 해당 질의 형태에서 큰 폭 개선 (미측정) | 컴파일러 변경 |
| PERF-09 | `ANALYZE`를 아무도 실행하지 않음 | **High** | `engine/src/` 전체에 `ANALYZE` 호출 0건 | 통계 없이 `unnest` 행 수·`reltuples` 추정 실패 → 깊은 순회 판단이 폴백 | DDL 후 자동 `ANALYZE` + 통계 목표 상향 | 계획 품질 전반 (미측정) | 대형 테이블 DDL 시간 증가 |
| PERF-10 | 벡터 검색이 `UNION ALL` 뷰 위에서 실행 | Low | `engine/src/vector/mod.rs:112, 126-132` | 서브타입 분기마다 HNSW 스캔 후 재정렬 | 계획 확인 후 필요 시 단일 테이블 경로 추가 | 미측정 | 서브타입 포함 의미론을 깨면 안 됨 |
| PERF-11 | widening이 전체 테이블 재작성 | Med | `engine/src/storage/mod.rs:127-145` | 한 번의 `CREATE`가 `ACCESS EXCLUSIVE` + 전 인덱스 재구축 유발 | 임계 초과 시 거부하고 명시적 마이그레이션 요구 | 예측 불가능한 장기 락 제거 | Neo4j 호환성 일부 손실 |
| PERF-12 | 이력 payload가 행 전체(벡터 포함) | Med | `engine/sql/access.sql:285`, `engine/src/vector/mod.rs:56-64` | 임베딩 타입에 이력을 켜면 갱신마다 벡터 전체 직렬화 | 큰 컬럼 제외 목록 또는 컬럼 화이트리스트 | 이력 크기 대폭 감소 | 이력 완전성 저하 (명시 필요) |
| PERF-13 | TypeQL 값이 SQL 리터럴로 보간 | Med | `engine/src/typeql/write.rs:242-245` | 값마다 다른 SQL 문자열 → 계획 캐시 재사용 불가 | 바인딩 파라미터로 전환 | 계획 시간 감소, 캐시 오염 제거 | 리터럴 조립 코드 전면 수정 |
| PERF-14 | 하이브리드 검색이 `og_vlp`(트레일 열거)를 사용 | Med | `engine/src/vector/mod.rs:253` | 3홉 근접성에 `degree^3` 행을 만들고 `GROUP BY`로 압축 | `og_reach`로 교체 | 평균 차수 20에서 앵커당 8,000행 → 수백 행 (계산치) | `og_reach`는 `parallel_restricted` |
| PERF-15 | 공개 벌크 로드 경로 부재 | **High** | `bench/harness.py:322-355`, `engine/src/`에 `COPY` 0건 | 벤치는 내부 테이블에 직접 INSERT — 사용자에게 그 경로가 없음 | `og_rebuild_adjacency()` + 문서화된 COPY 레시피 | 로드 처리량 (벤치 기준 124,580 edges/s) | 사용자가 레지스트리를 직접 다루게 됨 |
| PERF-16 | `delete_node_inner`가 차수당 6문장 | Med | `engine/src/storage/mod.rs:355-383, 501-528` | 차수 10,000 노드 삭제 = 6만 문장/1 트랜잭션 | 집합 기반 세그먼트 재작성으로 대체 | O(D) → O(1) 문장 | 카운터 의미 변경, 빈 세그먼트 처리 필요 |
| PERF-17 | `og_check_integrity()`가 O(&#124;E&#124;) | Med | `engine/src/storage/stats.rs:202-222` | `LIMIT 100`은 출력만 제한 — 건강하면 전부 스캔 | 샘플링 모드 인자 추가 | 정기 점검을 온라인화 | 샘플링은 완전성을 잃음 |

---

## 우선순위

| 순위 | 항목 | 이유 |
|---|---|---|
| 1 | **PERF-15 + PERF-02** | 벌크 로드 경로가 없다는 것이 가장 큰 실사용 장벽. 두 개는 같은 수정으로 해결된다 |
| 2 | **DATA-06** | 그래프 드롭이 데이터를 남기는 것은 정합성 결함이다 |
| 3 | **PERF-09** | 통계 없이는 깊은 순회 최적화 자체가 폴백으로 떨어진다 |
| 4 | **DATA-01** | 동시 쓰기 확장성의 하드 상한 |
| 5 | **DATA-08 / PERF-08** | Cypher 앱의 가장 흔한 질의 형태가 인덱스를 못 탄다 |
| 6 | DATA-03, DATA-05, DATA-04 | 인덱스 정리 — 저비용 고효율 |
| 7 | 나머지 | |

---

## 상세

### PERF-15 + PERF-02 — 벌크 로드 경로와 쓰기 증폭

**현상**

`og_create_edge()` 하나가 최소 6개의 SPI 문장을 실행하고, 그중 2개는
`og_adj` 튜플 전체 재작성이다(`engine/src/storage/mod.rs:402-452`).
256개짜리 세그먼트를 한 개씩 채우면:

```
Σ(k=1..256) (튜플 크기 ≈ 100 + 16k) ≈ 552 KB 기록 / 548 KB 죽은 튜플
최종 살아 있는 데이터: 4.2 KB
→ 약 130배 쓰기 증폭. 양방향이므로 엣지 하나당 2회.
```

저장소는 이 문제를 **이미 알고 있고 우회한다**:

> "Bulk load through SQL rather than one Cypher CREATE per row: the
> per-statement overhead would dominate and tell us nothing."
> (`bench/harness.py:322-323`)

`docs/benchmark.md:325`가 보고하는 124,580 edges/s는 **저 우회 경로의 수치**이며,
`og_create_edge()`의 처리량은 이 저장소 어디에도 없다.

**제안 1 — `og_rebuild_adjacency()` 노출**

벤치 하네스가 쓰는 SQL을 그대로 확장 함수로 만든다.

```sql
-- 새 함수 (Rust #[pg_extern] 또는 LANGUAGE sql):
--   og_rebuild_adjacency(graph text, rel_type text) RETURNS int8
CREATE OR REPLACE FUNCTION og_rebuild_adjacency(p_rid int4)
RETURNS int8 LANGUAGE sql AS $$
    WITH wipe AS (DELETE FROM og_data.og_adj WHERE etype = p_rid),
    fwd AS (
        INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
        SELECT src, p_rid, 'o', chunk, count(*)::int4,
               array_agg(dst ORDER BY id), array_agg(id ORDER BY id)
          FROM (SELECT src, dst, id,
                       ((row_number() OVER (PARTITION BY src ORDER BY id)) - 1)::int4 / 256 AS chunk
                  FROM og_data.og_edge WHERE type_id = p_rid) x
         GROUP BY src, chunk
        RETURNING 1),
    rev AS (
        INSERT INTO og_data.og_adj (src, etype, dir, seq, n, nbr, eid)
        SELECT dst, p_rid, 'i', chunk, count(*)::int4,
               array_agg(src ORDER BY id), array_agg(id ORDER BY id)
          FROM (SELECT src, dst, id,
                       ((row_number() OVER (PARTITION BY dst ORDER BY id)) - 1)::int4 / 256 AS chunk
                  FROM og_data.og_edge WHERE type_id = p_rid) x
         GROUP BY dst, chunk
        RETURNING 1)
    SELECT (SELECT count(*) FROM fwd) + (SELECT count(*) FROM rev);
$$;
```

**제안 2 — 배치 append**

`adjacency::append_many(src, etype, dir, &[(nbr, eid)])`를 추가해
`create_edge_inner`가 여러 엣지를 한 번에 만들 때 세그먼트를 한 번만 쓰게 한다.

**제안 3 — 문서화된 COPY 레시피**

[`08_data_lifecycle.md`](08_data_lifecycle.md) 1절의 6단계를 공식 절차로 만든다.
특히 4단계(`og_id_alloc` 워터마크 복구)는 하네스에 없어 그대로 흉내 내면
**id가 충돌한다**.

**예상 효과**: 벌크 로드 쓰기량 약 1/100 (계산치 — 552KB → 4.2KB per segment).
실제 처리량은 **미측정**.

**리스크**: `og_rebuild_adjacency`는 해당 관계 타입의 인접을 **전부 지우고 다시 만든다.**
운영 중 호출하면 그 사이 순회가 빈 결과를 본다. `LOCK TABLE` 또는 유지보수 창이 필요하다.

---

### DATA-06 — `og_drop_graph`가 `og_data`를 남긴다

**현상**

```rust
fn og_drop_graph(name: &str) {
    let gid = graph_id(name);
    // storage_table들을 DROP TABLE CASCADE
    Spi::run_with_args("DELETE FROM og_catalog.graph WHERE graph_id = $1", &[gid.into()])
}
```
(`engine/src/catalog/types.rs:321-340`)

`og_data`에는 FK가 하나도 없으므로(`engine/sql/bootstrap.sql:227-247`)
`og_node` / `og_edge` / `og_adj` / `og_id_alloc` / `og_role_player` /
`og_embedding_state` / `og_source` / `og_iri` 행이 **전부 남는다.**

`og_check_integrity()`의 검사 4가 이를 `orphan_node`로 보고한다
(`engine/src/storage/stats.rs:244-259`) — 즉 **엔진 스스로가 이 상태를 결함으로 정의한다.**

`og_reorganize()`는 `og_node → og_catalog.type` 조인을 통과하지 못해
이 세그먼트들을 정리하지 못한다(`engine/src/storage/stats.rs:128-130`).

**제안**

`og_drop_graph`에 다음을 추가한다 (타입 목록을 미리 뽑아 두고 `DELETE`).

```sql
-- 카탈로그 삭제 전에 type_id 목록을 확보
-- let tids: Vec<i32> = SELECT type_id FROM og_catalog.type WHERE graph_id = $1;

DELETE FROM og_data.og_adj           WHERE etype     = ANY($tids);
DELETE FROM og_data.og_edge          WHERE type_id   = ANY($tids);
DELETE FROM og_data.og_role_player   WHERE role_id IN
       (SELECT role_id FROM og_catalog.role WHERE rel_type_id = ANY($tids));
DELETE FROM og_data.og_embedding_state WHERE entity_id IN
       (SELECT id FROM og_data.og_node WHERE type_id = ANY($tids));
DELETE FROM og_data.og_source        WHERE entity_id IN
       (SELECT id FROM og_data.og_node WHERE type_id = ANY($tids));
DELETE FROM og_data.og_iri           WHERE entity_id IN
       (SELECT id FROM og_data.og_node WHERE type_id = ANY($tids));
DELETE FROM og_data.og_node          WHERE type_id   = ANY($tids);
DELETE FROM og_data.og_id_alloc      WHERE type_id   = ANY($tids);
-- og_history / og_audit 는 의도적으로 남긴다 (감사 기록)
DELETE FROM og_catalog.graph WHERE graph_id = $1;   -- 기존 동작
```

`og_drop_type`에도 같은 부수 정리를 넣어야 한다(`DATA-07`).

**예상 효과**: 그래프 삭제 후 `og_data` 디스크가 실제로 회수된다.
`og_check_integrity()`가 다시 0행을 낸다.

**리스크**
- **`$tids` 계산을 틀리면 다른 그래프 데이터를 지운다.** 반드시 카탈로그 삭제 **전에**
  `graph_id`로 필터해 얻어야 한다.
- `og_adj` DELETE는 `etype` 인덱스가 없어 순차 스캔이다 (→ `DATA-03`을 먼저 고치면 좋다).
- 대형 그래프에서는 하나의 긴 트랜잭션이 된다. 배치 처리 옵션이 필요하다.

**즉시 적용 가능한 임시 대응**: [`08_data_lifecycle.md`](08_data_lifecycle.md) "운영 레시피 A".

---

### PERF-09 — `ANALYZE`를 아무도 실행하지 않는다

**현상**

`engine/src/` 전체에서 `ANALYZE` 문자열의 유일한 매치는
`engine/src/cypher/mod.rs:682`의 `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` 옵션 문자열이다.
DDL 뒤에도, 백필 UPDATE 뒤에도, 인덱스 생성 뒤에도 `ANALYZE`가 없다.

이것이 실제로 무엇을 망가뜨리는지 코드가 직접 말한다:

```rust
/// Degree cannot be estimated on an unanalysed table, so depth alone decides.
const DEEP: u32 = 4;

let est = crate::spiu::two::<f32, f32>(
    "SELECT (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_node'::regclass),
            (SELECT reltuples FROM pg_class WHERE oid = 'og_data.og_edge'::regclass)", &[]);
let (nodes, edges) = match est {
    Ok((Some(n), Some(e))) if n > 0.0 && e > 0.0 => (n as f64, e as f64),
    _ => return max >= DEEP,          // ← 통계가 없으면 여기로 떨어진다
};
```
(`engine/src/cypher/compile.rs:43-54`)

`pg_class.reltuples`는 `ANALYZE` 또는 `VACUUM`이 채운다. 갓 적재한 그래프에서는
비어 있고, 깊은 순회 전환이 **"깊이 4 이상"이라는 고정 규칙**으로 폴백한다.
주석이 인정하듯 무조건 적용은 2홉을 더 느리게 만든 이력이 있다
(`engine/src/cypher/compile.rs:38-42`).

추가로, `og_adj`의 배열 컬럼에 통계가 없으면 `unnest(a.nbr, a.eid)`의 행 수 추정이
실제 세그먼트 길이를 반영하지 못한다. PostgreSQL은 배열 컬럼에 대해
원소 빈도(MCELEM)와 **원소 개수 히스토그램(DECHIST)**을 수집하며,
14 이후 `unnest`의 planner support가 이를 사용한다.

> **미측정**: 이 저장소에서 `ANALYZE` 전후의 계획 차이를 실측하지 않았다.

**제안 1 — DDL 후 자동 `ANALYZE`**

```rust
// engine/src/catalog/types.rs — og_add_property의 __ext 백필 직후 (:570 부근)
Spi::run(&format!("ANALYZE {table}")).ok();

// engine/src/catalog/types.rs — og_create_index 직후 (:611 부근)
Spi::run(&format!("ANALYZE {table}")).ok();
```

**제안 2 — 인접 테이블의 통계 목표 상향**

```sql
ALTER TABLE og_data.og_adj ALTER COLUMN nbr   SET STATISTICS 1000;
ALTER TABLE og_data.og_adj ALTER COLUMN etype SET STATISTICS 1000;
ANALYZE og_data.og_adj;
```

**제안 3 — autovacuum/autoanalyze 강화** (`og_adj`는 갱신 집약적이다)

```sql
ALTER TABLE og_data.og_adj SET (
    autovacuum_vacuum_scale_factor  = 0.02,
    autovacuum_analyze_scale_factor = 0.02
);
```

**제안 4 — 진단 노출**

`og_graph_stats()`에 "마지막 ANALYZE 시각"과 "reltuples 유효 여부"를 추가한다.
현재는 `og_diagnose_empty` / `og_estimate`가 있어도 통계 부재를 알려주지 않는다.

**예상 효과**: 깊은 순회 전환이 실제 차수를 보고 판단하게 된다.
`docs/deep-traversal.md`의 6홉 49,334ms → 71ms 개선은 **전환이 일어났을 때**의 수치다.

**리스크**: 큰 테이블에 대한 `ANALYZE`는 DDL 지연을 늘린다.
`ANALYZE`는 `SHARE UPDATE EXCLUSIVE`라 읽기/쓰기를 막지는 않는다.

---

### DATA-01 — `og_id_alloc`의 행 단위 직렬화

**현상**

```rust
"INSERT INTO og_data.og_id_alloc (type_id, next_id) VALUES ($1, 2)
 ON CONFLICT (type_id) DO UPDATE SET next_id = og_id_alloc.next_id + 1
 RETURNING next_id - 1"
```
(`engine/src/storage/mod.rs:26-29`)

`DO UPDATE`는 `(type_id)` 행에 **배타적 행 락**을 잡고, 그 락은 **커밋까지** 유지된다.
따라서 같은 타입의 노드를 만드는 두 트랜잭션은 문장 단위가 아니라 **트랜잭션 단위로**
직렬화된다. 트랜잭션이 길수록(예: Cypher 배치 쓰기) 대기 시간이 그대로 늘어난다.

`og_create_edge`도 같은 경로를 탄다(`engine/src/storage/mod.rs:418`).

**제안 A — 타입별 시퀀스** (권장)

```sql
-- og_create_type 시:
CREATE SEQUENCE og_data.idseq_<type_id> AS bigint START 1 MAXVALUE 68719476735;
-- alloc_id:
SELECT nextval('og_data.idseq_<type_id>')
```
`nextval()`은 트랜잭션 락을 잡지 않는다.

**리스크**
- **롤백해도 id가 반납되지 않는다.** 현재 구현은 반납한다. 다만 id는 이미
  삭제로 인해 갭이 생기므로 의미 있는 후퇴는 아니다.
- 시퀀스는 런타임 생성 사용자 객체이므로 `pg_dump`가 자동으로 덤프한다 (문제없음).
- `og_drop_type`이 시퀀스도 드롭해야 한다.
- 기존 설치의 마이그레이션 경로가 필요하다 (`setval`로 `og_id_alloc.next_id` 이식).

**제안 B — 블록 예약** (변경 최소)

백엔드가 한 번에 N개(예: 1,000)를 예약하고 로컬에서 소비한다.
`og_id_alloc`에 대한 UPSERT 빈도가 1/N로 줄어든다.

**리스크**: 백엔드 종료 시 미사용 블록이 버려진다 (갭 발생, 무해).
`storage::traverse`의 `thread_local!` CSR과 같은 패턴이라 구현 선례가 있다.

**예상 효과**: 동일 타입에 대한 동시 삽입이 실제로 병렬화된다.
**미측정** — 현재 저장소에 동시 쓰기 벤치가 없다.

---

### DATA-08 / PERF-08 — `__ext`와 타입 미상 프로퍼티 읽기

**현상**

```rust
match tid {
    Some(_) => (format!("({alias}.__ext->>{})", sql_str(prop)), None),
    None    => (format!("(og_node_json({alias}.id)->>{})", sql_str(prop)), None),
}
```
(`engine/src/cypher/compile.rs:986-992`)

- 첫 갈래: `__ext`에 인덱스가 없다. 확인: `engine/src/`에서 `gin` 매치는
  `engine/src/compat/ddl.rs:265`의 전문 검색 표현식 인덱스뿐. → 전체 스캔.
- 둘째 갈래: `og_node_json()`은 `LANGUAGE plpgsql`이라 **인라인되지 않고**
  행마다 4개의 서브질의를 돈다(`engine/sql/access.sql:208-235`).
  `to_jsonb(x)`는 `vector(N)`과 큰 `__ext`의 TOAST를 펼친다.

**제안 1 — 동등 비교를 `@>`로 컴파일하고 GIN 인덱스**

```sql
-- 컴파일러 변경: (__ext->>'k') = 'v'  →  __ext @> jsonb_build_object('k','v')
CREATE INDEX IF NOT EXISTS gin_<tid>_ext ON og_data.n_<tid> USING gin (__ext jsonb_path_ops);
```
`jsonb_path_ops`는 `@>`만 지원하며 `?`(키 존재)를 지원하지 않는다.
`og_add_property`의 백필이 `WHERE __ext ? 'prop'`를 쓰므로
(`engine/src/catalog/types.rs:563`) 그것까지 인덱스로 받으려면 `jsonb_ops`가 필요하다.

**제안 2 — 표현식 인덱스 API 노출** (더 안전, 컴파일러 변경 불필요)

```sql
-- 새 함수: og_create_ext_index(graph, type_name, prop)
CREATE INDEX IF NOT EXISTS ixe_<sub>_<slug> ON og_data.n_<sub> ((__ext ->> 'prop'));
```
현재 컴파일러가 내는 `(__ext->>'prop') = 'v'`를 **그대로** 탄다.

**제안 3 — 타입 미상 경로 축소**

`og_node_json()` 호출은 정말로 타입을 모를 때만 쓰고,
`MATCH (n:Label)` 처럼 라벨이 있으면 항상 첫 갈래로 가게 한다.
현재도 그렇게 동작하지만, 라벨 없는 `MATCH (n)`는 피할 수 없다.
`og_typeql_attribute` 뷰의 `og_node_json(e.dst) ->> 'val'`은
속성 타입별 `a_<tid>.val`을 직접 읽는 형태로 바꿀 수 있다
(`engine/sql/access.sql:311`).

**예상 효과**: 미선언 프로퍼티 필터가 인덱스를 탄다. **미측정.**

**리스크**: 제안 1은 컴파일러 변경 범위가 크고, `>`/`<` 같은 비등가 비교에는
적용되지 않는다. 제안 2가 위험 대비 효과가 가장 낫다.

---

### DATA-03 / DATA-04 / DATA-05 — 인덱스 정리

**DATA-03 — `og_adj(etype)`**

```rust
for sub in subs {
    ...
    Spi::run_with_args("DELETE FROM og_data.og_adj WHERE etype = $1", &[sub.into()]).ok();
```
(`engine/src/catalog/types.rs:706`)

`etype`은 PK `(src, etype, dir, seq)`의 2번 컬럼이므로 경계 조건이 될 수 없다.
**서브타입 수만큼 `og_adj` 전체 스캔**이 돈다.

**권장 수정 (쓰기 비용 0)** — 루프 밖에서 한 번만:
```sql
DELETE FROM og_data.og_adj WHERE etype = ANY($subs);
```
N회 스캔이 1회가 된다. 인덱스를 추가하지 않으므로 쓰기 경로에 부담이 없다.

**대안 (읽기 최적)**:
```sql
CREATE INDEX IF NOT EXISTS og_adj_etype_idx ON og_data.og_adj (etype);
```
**리스크**: `og_adj`는 가장 쓰기가 잦은 테이블이다. 인덱스 하나가
모든 append/remove의 비용을 늘린다. `DATA-09`의 `LIST(etype)` 파티셔닝이
같은 문제를 인덱스 없이 푸는 대안이다.

**DATA-04 — 중복 인덱스 제거**

```sql
CREATE INDEX type_label_range_idx ON og_catalog.type_label (graph_id, lft, rgt);  -- :79
CREATE INDEX type_label_lft_idx   ON og_catalog.type_label (graph_id, lft);       -- :80
```
후자는 전자의 **진부분 접두사**다. 전자가 할 수 있는 일을 후자가 더 잘하는 경우가 없다.

**수정**: `bootstrap.sql:80`을 삭제한다(신규 설치).
기존 설치는 확장 멤버라 그냥 `DROP INDEX`가 거부되므로:
```sql
ALTER EXTENSION ontological DROP INDEX og_catalog.type_label_lft_idx;
DROP INDEX og_catalog.type_label_lft_idx;
```

**리스크**: 거의 없다. `relabel_graph`가 라벨 행마다 INSERT를 하므로
(`engine/src/catalog/labeling.rs:160-167`) 인덱스 하나를 줄이면 재라벨링이 빨라진다.

**DATA-05 — FK 컬럼 인덱스 추가**

```sql
CREATE INDEX IF NOT EXISTS role_player_type_idx
    ON og_catalog.role (player_type_id) WHERE player_type_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS role_parent_role_idx
    ON og_catalog.role (parent_role_id) WHERE parent_role_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS og_constraint_type_idx
    ON og_catalog.og_constraint (type_id);
CREATE INDEX IF NOT EXISTS rule_target_type_idx
    ON og_catalog.rule (target_type_id) WHERE target_type_id IS NOT NULL;
```

근거: `og_catalog.role.player_type_id`(`engine/sql/bootstrap.sql:111`)와
`role.parent_role_id`(`:119`), `og_constraint.type_id`(`:138`),
`rule.target_type_id`(`:152`)는 전부 FK인데 인덱스가 없다.

**추가 위험**: `role.player_type_id`는 `ON DELETE` 절이 없어 **`NO ACTION`**이다.
어떤 역할의 참여자 타입으로 지정된 타입을 `og_drop_type`으로 지우면
FK 위반으로 함수가 실패한다. 인덱스 추가와 별개로,
`og_drop_type`이 이 참조를 먼저 정리하거나 `ON DELETE SET NULL`로 바꿔야 한다.
`og_constraint.type_id`도 마찬가지로 `kind` 조회가 전체 스캔이다
(`engine/src/typeql/write.rs:266-269`).

**예상 효과**: 타입 삭제 시 자식 테이블 전체 스캔 제거, 잠금 범위 축소.
카탈로그는 작으므로 절대 시간 이득은 작지만 **잠금 범위**가 실제 이득이다.

---

### DATA-09 — `og_adj` 파티셔닝

**현재**: 단일 힙 테이블. 100만 엣지 = 최소 200만 개의 인접 항목 = 수만 세그먼트.

**후보 A — `LIST (dir)`, 2파티션** (가장 안전)

PK가 `(src, etype, dir, seq)`이므로 `dir`은 이미 모든 유니크 제약에 포함되어 있다.
파티션 키 조건을 만족한다.

```sql
CREATE TABLE og_data.og_adj (...) PARTITION BY LIST (dir);
CREATE TABLE og_data.og_adj_o PARTITION OF og_data.og_adj FOR VALUES IN ('o');
CREATE TABLE og_data.og_adj_i PARTITION OF og_data.og_adj FOR VALUES IN ('i');
```

**이득**: 모든 순회 질의가 `dir`을 알고 있으므로 프루닝이 완벽하다.
파티션당 인덱스가 절반이 되고, VACUUM이 두 관계로 나뉜다.
파티션 수가 고정이라 DDL 처닝이 없다.

**한계**: `dir IN ('o','i')`(양방향 매치, `engine/src/cypher/compile.rs:897`)와
`og_csr_build`의 전체 스캔(`engine/src/storage/traverse.rs:246`)은 두 파티션을 다 읽는다.

**후보 B — `HASH (src)`, 8~32파티션**

**이득**: autovacuum이 파티션별로 병렬 실행된다. 락 경합이 분산된다.
파티션 프루닝은 `src = ?`일 때 정확하다.

**한계**: `src = ANY($frontier)`(`engine/src/storage/traverse.rs:95`)는 여러 파티션에 걸친다.

**후보 C — `LIST (etype)`**

`DATA-03`을 인덱스 없이 해결한다 (`DETACH` + `DROP`이 O(1)).
**하지만** 타입 생성마다 파티션 DDL이 필요하고, 부모에 `ACCESS EXCLUSIVE`가 걸린다.
Cypher가 라벨을 자동 타입화하는 이 시스템에서는 위험하다. **권장하지 않는다.**

**공통 리스크 — `pg_dump` 등록**

`og_adj`는 `pg_extension_config_dump('og_data.og_adj', '')`로 등록되어 있다
(`engine/sql/bootstrap.sql:426`). 파티션 테이블로 바꾸면 **각 파티션을 개별로
등록해야 한다** — 그러지 않으면 덤프가 조용히 빈 인접을 복원한다.
이것이 이 개선의 가장 큰 위험이며, 반드시 복원 왕복 테스트가 선행되어야 한다.

---

### PERF-16 — `delete_node_inner`의 문장 수

**현상**: 노드 하나 삭제 = `6D + 4` 문장 (D = 차수).
근거: `engine/src/storage/mod.rs:355-383`(4문장 + 루프),
`engine/src/storage/mod.rs:501-528`(엣지당 6문장, 그중 `adjacency::remove` ×2 = 4문장).

**제안 — 집합 기반 세그먼트 재작성**

인접 엣지 id 목록 `$victims`와 영향받는 노드 목록 `$srcs`를 먼저 모은 뒤:

```sql
-- 1) 살아남는 원소만으로 세그먼트를 다시 만든다 (두 배열의 정렬을 ORDINALITY로 보존)
UPDATE og_data.og_adj a
   SET nbr = s.nbr, eid = s.eid, n = s.n
  FROM (SELECT x.src, x.etype, x.dir, x.seq,
               array_agg(u.nbr ORDER BY u.ord) AS nbr,
               array_agg(u.eid ORDER BY u.ord) AS eid,
               count(*)::int4                  AS n
          FROM og_data.og_adj x,
               LATERAL unnest(x.nbr, x.eid) WITH ORDINALITY AS u(nbr, eid, ord)
         WHERE x.src = ANY($srcs)
           AND NOT (u.eid = ANY($victims))
         GROUP BY x.src, x.etype, x.dir, x.seq) s
 WHERE a.src = s.src AND a.etype = s.etype AND a.dir = s.dir AND a.seq = s.seq;

-- 2) 전부 제거되어 위 GROUP BY에 나타나지 않은 세그먼트를 지운다
DELETE FROM og_data.og_adj a
 WHERE a.src = ANY($srcs)
   AND NOT EXISTS (SELECT 1 FROM unnest(a.eid) e WHERE NOT (e = ANY($victims)));

-- 3) 나머지는 집합 삭제
DELETE FROM og_data.og_edge        WHERE id      = ANY($victims);
DELETE FROM og_data.og_role_player WHERE edge_id = ANY($victims);
-- 타입 테이블은 etype별로 한 번씩
```

**중요**: 2번 문장이 반드시 필요하다. 1번의 `GROUP BY`는 남는 원소가 0인
세그먼트를 아예 출력하지 않으므로, 그 세그먼트는 옛 내용을 그대로 유지한다.

**예상 효과**: O(D) 문장 → O(타입 수) 문장. **미측정.**

**리스크**
- `crate::stats` 카운터가 엣지마다 증가하는 현재 의미를 유지하려면
  삭제된 행 수를 집계해 한 번에 더해야 한다(`engine/src/storage/mod.rs:526`).
- `$srcs` / `$victims`가 매우 크면 `= ANY(array)`의 계획이 나빠진다.
  임계 이상에서는 임시 테이블 + 조인이 낫다.

---

### DATA-11 — 18비트 타입 공간

**현상**
- `og_catalog.type_id_seq START 1 MAXVALUE 262143`, `CYCLE` 없음
  (`engine/sql/bootstrap.sql:47`).
- Cypher 쓰기가 **모르는 라벨을 자동으로 타입화**한다
  (`engine/src/catalog/types.rs:210-231`).
- `CREATE INDEX FOR (n:Whatever)`도 같다(`engine/src/compat/ddl.rs:203`).
- `og_drop_type`은 id를 반납하지 않는다(반납하면 안 된다 — id에 박혀 있다).
- 소진되면 `nextval`이 오류를 내고 **모든 타입 생성과 자동 라벨 생성이 막힌다.**

**제안 1 — 압박 노출**
```sql
CREATE OR REPLACE VIEW og_catalog.og_id_pressure AS
SELECT 'type_id' AS space, last_value AS used, 262143 AS limit_,
       round(last_value::numeric / 262143 * 100, 2) AS pct
  FROM og_catalog.type_id_seq
UNION ALL
SELECT 'local:' || COALESCE(t.name, a.type_id::text), a.next_id, 68719476735,
       round(a.next_id::numeric / 68719476735 * 100, 4)
  FROM og_data.og_id_alloc a LEFT JOIN og_catalog.type t USING (type_id);
```

**제안 2 — 소프트 임계 경고**

`create_type_inner`에서 `nextval` 결과가 임계(예: 200,000)를 넘으면
`pgrx::warning!`을 낸다. 오류가 아니라 경고여야 한다 — 정당한 대형 온톨로지가 있다.

**제안 3 — 자동 타입화 제한 설정**

`og_catalog.setting`에 `auto_label = 'on'|'off'` 키를 두고,
`off`면 미지 라벨에 대해 `error!`를 낸다.

**리스크**: 제안 3은 Neo4j 호환성을 훼손한다 (Neo4j는 쓰기 시 라벨을 만든다).
기본값은 반드시 `on`이어야 하고, 운영 환경에서 명시적으로 끄는 용도다.

---

### DATA-02 — `og_adj` 제약 추가

**현상**: `dir`이 `'o'|'i'`인지, `n`이 배열 길이와 같은지를 강제하는 제약이 없다
(`engine/sql/bootstrap.sql:197-206`). 그래서 `og_check_integrity()`의 검사 3이
존재한다(`engine/src/storage/stats.rs:225-241`) — **제약으로 막을 수 있는 것을
사후 검사로 잡고 있다.**

**제안**
```sql
ALTER TABLE og_data.og_adj
  ADD CONSTRAINT og_adj_dir_ck CHECK (dir IN ('o','i')) NOT VALID;
ALTER TABLE og_data.og_adj
  ADD CONSTRAINT og_adj_len_ck
  CHECK (n = COALESCE(array_length(nbr, 1), 0)
     AND n = COALESCE(array_length(eid, 1), 0)) NOT VALID;

-- 온라인 검증 (ACCESS EXCLUSIVE 없이 SHARE UPDATE EXCLUSIVE)
ALTER TABLE og_data.og_adj VALIDATE CONSTRAINT og_adj_dir_ck;
ALTER TABLE og_data.og_adj VALIDATE CONSTRAINT og_adj_len_ck;
```

**예상 효과**: 세그먼트 손상이 **쓰기 시점에** 실패한다.
`og_check_integrity()`의 검사 3이 사실상 불필요해진다.

**리스크**
- `array_length()`는 배열 헤더만 읽으므로 O(1)이다. 쓰기 비용은 무시할 만하다.
- 기존 데이터가 이미 손상되어 있으면 `VALIDATE`가 실패한다. 그때는 먼저
  `og_check_integrity()`로 손상 범위를 확인하고 `og_reorganize()`로 복구해야 한다.
- `NOT VALID`로 먼저 추가하면 **새 쓰기부터** 강제되므로 즉시 이득이 있다.

---

## 부록 — 확인 스크립트

```sql
-- 안 쓰이는 인덱스 (DATA-04, DATA-14, DATA-18 검증)
SELECT schemaname, relname, indexrelname, idx_scan,
       pg_size_pretty(pg_relation_size(indexrelid)) AS size
  FROM pg_stat_user_indexes
 WHERE schemaname IN ('og_data','og_catalog') AND idx_scan = 0
 ORDER BY pg_relation_size(indexrelid) DESC;

-- 인덱스 없는 FK (DATA-05 검증)
SELECT c.conrelid::regclass AS child, a.attname AS fk_column,
       c.confrelid::regclass AS parent
  FROM pg_constraint c
  JOIN unnest(c.conkey) WITH ORDINALITY AS k(attnum, ord) ON true
  JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
 WHERE c.contype = 'f'
   AND c.connamespace IN ('og_catalog'::regnamespace, 'og_data'::regnamespace)
   AND NOT EXISTS (
        SELECT 1 FROM pg_index i
         WHERE i.indrelid = c.conrelid AND i.indkey[0] = k.attnum);

-- 세그먼트 조각화 (PERF-01, PERF-05 검증)
SELECT etype, dir, count(*) AS segments, avg(n)::numeric(6,1) AS avg_fill,
       round(avg(n)/256*100, 1) AS packing_pct
  FROM og_data.og_adj GROUP BY etype, dir ORDER BY 3 DESC;

-- 죽은 튜플 (PERF-02 검증)
SELECT relname, n_live_tup, n_dead_tup,
       round(n_dead_tup::numeric / GREATEST(n_live_tup,1) * 100, 1) AS dead_pct
  FROM pg_stat_user_tables WHERE schemaname = 'og_data'
 ORDER BY n_dead_tup DESC LIMIT 10;

-- 통계 신선도 (PERF-09 검증)
SELECT c.relname,
       (SELECT reltuples FROM pg_class WHERE oid = c.oid) AS reltuples,
       s.last_analyze, s.last_autoanalyze
  FROM pg_class c
  JOIN pg_namespace n ON n.oid = c.relnamespace AND n.nspname = 'og_data'
  LEFT JOIN pg_stat_user_tables s ON s.relid = c.oid
 WHERE c.relkind = 'r' ORDER BY 2 DESC NULLS LAST LIMIT 20;

-- 고아 데이터 (DATA-06, DATA-07 검증)
SELECT * FROM og_check_integrity();
SELECT count(*) AS orphan_role_players FROM og_data.og_role_player rp
 WHERE NOT EXISTS (SELECT 1 FROM og_catalog.role r WHERE r.role_id = rp.role_id);
SELECT count(*) AS orphan_embedding_state FROM og_data.og_embedding_state s
 WHERE NOT EXISTS (SELECT 1 FROM og_data.og_node n WHERE n.id = s.entity_id);
```

---

## 금지 / 필수

**금지**
- 위 DDL 스니펫을 **측정 없이** 프로덕션에 적용하는 것.
  각 항목의 "리스크"를 먼저 읽을 것.
- `og_adj`에 인덱스를 추가하면서 쓰기 벤치를 돌리지 않는 것.
  이 테이블은 쓰기 경로의 병목이다.
- 확장 소유 인덱스를 `ALTER EXTENSION ... DROP`을 거치지 않고 지우려 하는 것 (거부된다).
- 파티셔닝을 적용하면서 `pg_extension_config_dump` 등록을 갱신하지 않는 것.
  **덤프가 조용히 빈 그래프를 복원한다.**

**필수**
- 개선을 적용하면 이 문서의 해당 행에 "적용 커밋"을 기록하고,
  영향받는 문서([`01`](01_physical_schema.md), [`09`](09_query_access_paths.md))를 갱신할 것.
- 인덱스를 추가/삭제하면 [`09_query_access_paths.md`](09_query_access_paths.md) 1절의
  전수 목록을 갱신할 것.
- 스키마를 바꾸면 `pg_dump` → `pg_restore` 왕복 테스트를 할 것.
  현재 그 테스트는 **존재하지 않는다** (`engine/tests/sql/`에 관련 항목 없음).

---

<!-- affects: data, backend, performance, ops -->
<!-- requires-update: docs/06_data/01_physical_schema.md, docs/06_data/09_query_access_paths.md -->
