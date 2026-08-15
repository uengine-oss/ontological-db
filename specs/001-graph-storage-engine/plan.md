# Implementation Plan: 네이티브 그래프 저장 엔진

**Branch**: `001-graph-storage-engine` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `/specs/001-graph-storage-engine/spec.md`

## Summary

인접 정보를 **CSR 계열 인접 세그먼트**로, 프로퍼티를 **타입 카탈로그가 아는 고정 슬롯**으로
저장하는 그래프 저장 계층을 PostgreSQL 확장으로 구현한다. Apache AGE의 "힙 테이블 2개 +
agtype" 구조를 명시적으로 대체한다.

**핵심 설계 결정 3가지**

1. **식별자**: 노드/엣지 ID는 `int8` 단일 값에 `[shard:10][type:18][local:36]` 비트필드로
   인코딩. 트래버설 전체가 정수 연산이며, 007의 샤드 ID를 이미 수용한다.
2. **인접 저장**: `(src_id, edge_type_id, direction)` 키에 대해 이웃을 **정렬된 int8 배열**로
   묶어 저장하는 인접 세그먼트. 한 노드의 한 타입 이웃 전체가 1~2 페이지 안에서 순차 읽힌다.
   차수가 큰 노드는 `seq` 로 청크 분할된다.
3. **프로퍼티**: 타입별 전용 테이블(`og_node_<type_id>`)에 선언된 프로퍼티를 **실제 컬럼**으로
   보관. 이것이 "타입 기반 슬롯 저장"의 PostgreSQL 상 실현이며, 컬럼 프루닝·통계·인덱스가
   전부 공짜로 따라온다. 미선언 프로퍼티만 오버플로 `jsonb` 컬럼으로 간다.

## Technical Context

**Language/Version**: Rust 1.83+ / pgrx (PostgreSQL 확장 프레임워크)

**Primary Dependencies**: PostgreSQL 16, pgrx, pgvector 0.8+ (004에서 사용)

**Storage**: PostgreSQL 자체 (힙 + 인덱스 + WAL). 별도 저장 계층 없음

**Testing**: `cargo pgrx test` (in-DB 통합 테스트), SQL 회귀 테스트, criterion 마이크로벤치

**Target Platform**: Linux/macOS, PostgreSQL 16+, Docker 개발 이미지

**Project Type**: PostgreSQL 확장 (C ABI 공유 라이브러리)

**Performance Goals**: 1-hop 확장 AGE 대비 5배↑, 3-hop 10배↑ (SC-001/002)

**Constraints**: 표준 PG16에 `CREATE EXTENSION` 설치, superuser 커널 기능 금지

**Scale/Scope**: 단일 노드 10억 엣지

### 구현 언어 결정 (헌법 Technology Constraints 요구사항)

**Rust + pgrx 채택.** 근거:

- 저장 계층보다 **Cypher 파서/플래너(003)의 코드량이 압도적**이다. 재귀 하강 파서, AST,
  논리 계획 재작성은 Rust에서 C 대비 3~4배 적은 코드로, 메모리 안전하게 작성된다.
- pgrx가 `panic` → `ereport(ERROR)` 변환, 메모리 컨텍스트 연동, SRF/FDW/타입 등록을 안전하게
  래핑한다. 헌법의 "확장 내부 panic이 서버를 죽이면 안 된다" 요구를 프레임워크가 보장한다.
- 저장소 전체 단일 언어 원칙 준수. C 혼용 없음.

**거부한 대안**: C — 파서/플래너 작성 비용과 메모리 버그 리스크가 저장 계층의 이득을 상쇄한다.
PL/pgSQL — 성능 목표 달성 불가.

## Constitution Check

| 원칙 | 상태 | 근거 |
|------|------|------|
| I. 확장, 포크 아님 | ✅ | 표준 PG16 `CREATE EXTENSION`. 커널 패치 0 |
| II. Cypher 1급 | N/A | 003 범위 |
| III. 네이티브 저장 | ⚠️ 부분 | 인접 세그먼트·타입별 컬럼 저장은 달성. **TableAM은 v2로 연기** (아래 Complexity Tracking) |
| IV. 온톨로지 우선 | ✅ | 002 카탈로그를 소비해 물리 레이아웃 결정 |
| V. 벡터 | ✅ | 프로퍼티 슬롯이 `vector` 타입 컬럼을 수용 |
| VI. 코어 하나 | ✅ | 단일 저장 모델 |
| VII. 분산 준비 | ✅ | ID에 shard 비트 예약 |
| VIII. 에이전트 | ✅ | 통계·카탈로그를 introspection에 노출 |
| IX. ACID | ✅ | 힙/WAL/MVCC 그대로 상속. 인접 갱신은 동일 트랜잭션 |
| X. 벤치마크 | ✅ | 009 하네스 대상 |

## Architecture

### 식별자 인코딩

```
int8 (64bit):  [ 0 ][ shard: 9 ][ type_id: 18 ][ local_id: 36 ]
                bit63 = 0 (양수 보장)
```
- `type_id` 추출만으로 어느 타입 테이블인지 결정 → 조인 없이 라우팅
- `shard` 는 007 이전까지 항상 0, 재분배 시에도 local_id 불변

### 인접 세그먼트

```sql
og_adj (
  src        int8,     -- 시작 노드
  etype      int4,     -- 관계 타입
  dir        "char",   -- 'o' outgoing / 'i' incoming
  seq        int4,     -- 슈퍼노드 청크 번호
  n          int4,     -- 이 청크의 유효 원소 수
  nbr        int8[],   -- 이웃 노드 ID (정렬)
  eid        int8[],   -- 대응 엣지 ID
  PRIMARY KEY (src, etype, dir, seq)
)
```
- 한 노드의 한 타입 이웃 = **배열 1개 = 페이지 1~2개 순차 읽기**. AGE의 차수 N회 인덱스 조회
  대비 구조적 우위.
- `CHUNK_SIZE = 1024`. 차수 1,000만 슈퍼노드는 청크 스트리밍으로 첫 결과 즉시 반환.
- `dir` 분리 저장으로 방향별 프루닝, `etype` 분리로 타입별 프루닝(FR-003).

### 타입별 프로퍼티 테이블

```sql
og_node_<type_id> ( id int8 PRIMARY KEY, <declared props as real columns...>, __ext jsonb )
og_edge_<type_id> ( id int8 PRIMARY KEY, src int8, dst int8, <props...>, __ext jsonb )
```
002가 타입을 생성/변경할 때 DDL로 생성·진화한다. 선택적 컬럼 추가는 PG11+ 에서 테이블
재작성 없이 즉시 완료된다(FR-022 / SC-004의 근거).

### 접근 경로 API (003이 호출)

| 함수 | 용도 |
|------|------|
| `og_expand(src, etype[], dir, limit)` → SRF | 인접 확장 (스트리밍) |
| `og_expand_batch(src[], etype[], dir)` → SRF | 배치 확장 (다중 시작점) |
| `og_scan_type(type_id, include_subtypes)` → SRF | 타입 스캔 (상속 인덱스 사용) |
| `og_node_get(id)` / `og_edge_get(id)` | 단건 조회 |
| `og_degree(src, etype, dir)` | 비용 추정용 차수 |

## Project Structure

```text
engine/
├── Cargo.toml
├── ontological.control
├── sql/                          # 확장 부트스트랩 SQL
└── src/
    ├── lib.rs
    ├── id.rs                     # 식별자 인코딩/디코딩
    ├── storage/
    │   ├── mod.rs
    │   ├── adjacency.rs          # 인접 세그먼트 읽기/쓰기/청크
    │   ├── nodes.rs              # 노드 CRUD, 타입 테이블 라우팅
    │   ├── edges.rs              # 엣지 CRUD + 양방향 인접 갱신
    │   ├── bulk.rs               # 벌크 적재 (FR-017)
    │   ├── reorg.rs              # 온라인 재조직 (FR-018)
    │   └── stats.rs              # 통계/차수분포/단편화 (FR-019)
    └── ...
tests/
├── sql/001_storage/              # SQL 회귀 테스트
└── integration/
bench/001_storage/                # 마이크로벤치
```

**Structure Decision**: 단일 Rust 크레이트(`engine/`)에 스펙별 모듈. 확장은 하나의 `.so` 로
배포되어야 하므로 크레이트 분리는 하지 않는다.

## Phasing

| Phase | 내용 | 스펙 커버 |
|-------|------|-----------|
| **P0** | ID 인코딩, 타입 테이블 DDL 생성기, 노드/엣지 CRUD | FR-001,004,005,006 |
| **P1** | 인접 세그먼트 읽기/쓰기, 청크, `og_expand` SRF | FR-002,003,014,020 |
| **P2** | 통계/차수/비용 추정, 벌크 적재 | FR-015,017,019 |
| **P3** | 재조직, vacuum 연동, 단편화 지표 | FR-011,018 |
| **P4(v2)** | Table Access Method 전환 | FR-001 완전 충족 |

## Complexity Tracking

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| **원칙 III: TableAM 미사용 (v1)** | TableAM 구현은 튜플 직렬화·가시성·vacuum·WAL rmgr을 전부 자체 구현해야 하며, 이것만으로 수개월 규모다. v1은 PG 힙 위에 **인접 세그먼트 + 타입별 컬럼 저장**으로 III의 실질적 목표(순차 인접 접근, 파싱 없는 프로퍼티 접근, 고정폭 ID)를 달성한다 | JSONB 프로퍼티 저장은 III가 명시적으로 금지. 일반 엣지 테이블 + B-tree 인덱스 방식(AGE)은 성능 목표 미달. 선택한 구조는 **금지된 안티패턴을 모두 피하면서** 힙의 MVCC/WAL/vacuum을 재사용한다 |
| **`int8[]` 배열 기반 인접** | PG 배열은 TOAST 임계 이하에서 인라인 저장되어 순차 접근 지역성을 준다. CHUNK_SIZE 1024 × 8바이트 = 8KB 로 페이지 경계에 맞춘다 | 개별 행 저장은 차수 N회 인덱스 조회(=AGE 문제). GIN/GiST는 순차성 없음 |

**v2 마이그레이션 계획**: `og_expand` 등 접근 경로 API가 안정 인터페이스다. TableAM 전환 시
003 이상 계층은 변경되지 않는다.
