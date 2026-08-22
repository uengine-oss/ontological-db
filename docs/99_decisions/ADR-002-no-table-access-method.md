# ADR-002: v1에서 Table Access Method를 구현하지 않는다

| 항목 | 값 |
|---|---|
| 상태 | Accepted (v2에서 재평가 예정) |
| 날짜 | 2026-08-06 (`specs/001-graph-storage-engine/plan.md` 기준일) |
| 영향 범위 | storage |
| 근거 | `specs/001-graph-storage-engine/plan.md` Complexity Tracking, `.specify/memory/constitution.md` 원칙 III, `README.md` "Governance" 문단 |

> **이 문서가 답하는 질문**
> - 헌법 원칙 III("성능은 저장구조에서 나온다")를 내걸고도 왜 TableAM을 안 썼는가?
> - 그 대가로 무엇을 잃었고, 무엇으로 대신했는가?

## 배경

헌법 원칙 III는 "그래프 DB인데 사실은 정규 테이블 두 개"인 구조를 거부한다. 이를 문자
그대로 달성하는 길은 PostgreSQL의 **Table Access Method(TableAM)** 를 직접 구현해 힙이
아닌 그래프 전용 물리 레이아웃을 갖는 것이다. 원칙 I이 허용하는 확장점 목록에도 TableAM이
가장 먼저 적혀 있다.

## 고려한 선택지

1. **TableAM 자체 구현** — 튜플 직렬화, 가시성 판정, vacuum, WAL rmgr을 전부 직접 작성.
   장점: 물리 배치를 완전히 통제. 단점: `plan.md`가 기록한 대로 *"이것만으로 수개월 규모"*
   이며, 그 기간 동안 상위 계층(002~011)이 검증되지 않은 저장 계층 위에 얹힌다.
2. **JSONB/agtype 프로퍼티 + 일반 엣지 테이블(Apache AGE 방식)** — 구현이 가장 싸다.
   헌법이 **명시적으로 금지한 안티패턴**이며, 원칙 III의 측정 기준(1-hop 확장 비용 AGE 대비
   5배 이상 개선)을 충족하지 못한다.
3. **PG 힙 위의 인접 세그먼트 + 타입별 실컬럼** — 힙의 MVCC/WAL/vacuum을 재사용하되,
   물리 레이아웃은 그래프에 맞춘다.

## 결정

**3안.** v1은 TableAM이 아니다. `og_data.og_adj`(CSR형 인접 세그먼트, ADR-004)와
타입별 프로퍼티 테이블(ADR-005)로 원칙 III의 **실질적 목표** — 순차 인접 접근, 파싱 없는
프로퍼티 접근, 고정폭 ID — 를 달성하고, TableAM 전환은 v2로 연기한다.

이 이탈은 `specs/001-graph-storage-engine/plan.md`의 Constitution Check에 `⚠️ 부분`으로,
Complexity Tracking에 정식 위반 항목으로 기록되어 있다. README도 이를 숨기지 않는다:
*"this release is not a Table Access Method"*.

## 근거

- `specs/001-graph-storage-engine/plan.md` Complexity Tracking 원문:
  > TableAM 구현은 튜플 직렬화·가시성·vacuum·WAL rmgr을 전부 자체 구현해야 하며, 이것만으로
  > 수개월 규모다. v1은 PG 힙 위에 **인접 세그먼트 + 타입별 컬럼 저장**으로 III의 실질적
  > 목표(순차 인접 접근, 파싱 없는 프로퍼티 접근, 고정폭 ID)를 달성한다
- 같은 표의 대안 기각 사유: *"선택한 구조는 **금지된 안티패턴을 모두 피하면서** 힙의
  MVCC/WAL/vacuum을 재사용한다."*
- 힙을 유지한 덕에 얻은 것이 나중에 실제로 결정적이었다.
  `docs/deep-traversal.md`는 6홉 49,334ms → 71ms 개선이 **저장 구조를 바꾸지 않고**
  이루어졌음을 기록한다: *"Nothing about the storage needed to change."* 저장 계층을
  갈아엎는 중이었다면 이 개선은 그 뒤에나 가능했을 것이다.

## 결과

**긍정적**
- MVCC·WAL·vacuum·`pg_dump`를 한 줄도 쓰지 않고 얻는다 (`engine/sql/bootstrap.sql:9`).
- 접근 경로 API(`og_expand` 등)가 안정 인터페이스이므로, TableAM 전환 시 003 이상 계층은
  변경되지 않는다 (`plan.md` "v2 마이그레이션 계획").

**부정적 / 감수한 대가**
- 튜플 헤더 오버헤드와 힙 페이지 레이아웃을 우리가 통제하지 못한다.
- `docs/deep-traversal.md`가 측정한 대로, 힙을 벗어나면 같은 질문이 약 15배 빨라진다
  (dense 6홉: `og_reach` 71.42ms vs `og_csr_reach` 4.86ms). 그 차이가 힙의 가격이다.
- 원칙 III를 완전히 충족했다고 말할 수 없다. `plan.md`가 `⚠️ 부분`으로 적고 있다.

## 재검토 조건

- 프로파일이 **튜플 디폼/가시성 판정**을 지배적 비용으로 지목하고, 그 비용이
  `og_csr_reach`가 보여준 약 15배 격차의 대부분을 설명할 때.
- 접근 경로 API(`og_expand`, `og_expand_batch`, `og_reach`)가 안정화되어 상위 계층을
  건드리지 않고 저장 계층만 교체할 수 있음이 회귀 스위트로 보장될 때.
- 반대로, CSR 백엔드-로컬 캐시(ADR-014)가 힙 격차를 실무적으로 메운다고 판단되면 TableAM
  전환은 더 미뤄질 수 있다.

<!-- affects: storage, architecture -->
<!-- requires-update: docs/99_decisions/ADR-004-csr-adjacency-segments.md, docs/99_decisions/ADR-014-csr-not-automatic.md -->
