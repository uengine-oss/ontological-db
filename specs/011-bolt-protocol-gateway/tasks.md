# Tasks: Bolt 프로토콜 게이트웨이

**Spec**: [spec.md](spec.md) · **Plan**: [plan.md](plan.md) · **Updated**: 2026-08-06

체크된 항목은 이 저장소에서 실제로 동작하며 테스트로 덮여 있다.
검증은 우리가 만든 클라이언트가 아니라 **Neo4j 공식 드라이버**가 한다.

## Phase 0 — PackStream
- [x] T001 값 인코더/디코더 — null·bool·int(전 폭)·float·string·list·map·struct — `bolt/src/packstream.rs`
- [x] T002 청크 프레이밍 (2바이트 길이 + `0x0000` 종료), 메시지/청크 경계 독립 (FR-003)
- [x] T003 왕복 단위 테스트 — 헤더 폭 경계(15/16/255/256/70k), 6.5만 바이트 초과 메시지, UTF-8

## Phase 1 — 연결
- [x] T004 핸드셰이크와 버전 협상, 미지원 제안은 `0x00000000` 으로 명확히 거절 (FR-001, FR-002)
- [x] T005 `HELLO` → PostgreSQL 접속. 자체 사용자 저장소 없음 (FR-015)
- [x] T006 `RUN`/`PULL`/`DISCARD` 와 `SUCCESS`/`RECORD`/`FAILURE` (FR-005, FR-006)
- [x] T007 `RESET` 과 실패 상태에서의 `IGNORED` (FR-007)
- [x] T008 연결당 스레드, 연결당 PostgreSQL 세션 (FR-013, FR-017)

## Phase 2 — 값
- [x] T009 `og_cypher_columns()` 를 엔진에 추가 — 필드 순서는 파서가 알려준다 (FR-010)
- [x] T010 노드 → `Node` 구조체, 관계 → `Relationship` 구조체 (FR-011)
- [x] T011 파라미터는 jsonb 로 전달, 질의 문자열에 보간하지 않음 (FR-009)
- [x] T012 `RETURN *` 은 파서가 순서를 모르므로 행의 키로 폴백

## Phase 3 — 세션
- [x] T013 `BEGIN`/`COMMIT`/`ROLLBACK` → PostgreSQL 트랜잭션 (FR-014)
- [x] T014 `db` → 그래프 선택 (FR-016)
- [x] T015 트랜잭션 중 `db` 부재는 "기본값"이 아니라 "BEGIN 때 정한 그래프"
      — 이걸 틀려서 커밋이 다른 그래프로 갔었다
- [x] T016 오류를 `Neo.ClientError.*` 로 매핑하되 컴파일러 메시지는 원문 보존 (FR-019)
- [x] T017 `ROUTE` — 단일 서버 라우팅 테이블로 `neo4j://` 접속 허용

## Phase 4 — 증명
- [x] T018 가이드 질의 24개를 **세 경로**(PostgreSQL / Bolt / Neo4j)로 실행해 대조 (SC-002)
- [x] T019 공식 샘플 앱을 **URI만 바꿔** 실행 (SC-003) — `tests/neo4j-movies/sample_app.py`
- [x] T020 드라이버 수준 검사: Node/Relationship 역직렬화, 필드 순서, 파라미터,
      실패→RESET 복구, 트랜잭션 커밋/롤백, 동시 세션 8개 (SC-004, SC-005, SC-007)
- [x] T021 게이트웨이를 꺼도 PostgreSQL 경로 회귀 스위트가 그대로 통과 (SC-006)
- [x] T022 지원 매트릭스 문서화 — `bolt/README.md` (FR-020)

## Phase 5 — not yet
- [ ] T023 Bolt 5.x (`element_id`, 신규 시공간 타입)
- [ ] T024 `Path` 구조체 인코딩 (현재는 경로 변수가 홉의 리스트로 전달된다)
- [ ] T025 서버 사이드 커서 — 현재 `RUN` 이 결과를 전부 버퍼링한다. 큰 결과의
      메모리 상한은 게이트웨이 프로세스에 걸린다 (003 FR-018의 스트리밍은 SQL 경로에만 해당)
- [ ] T026 진짜 라우팅 테이블 (스펙 007 클러스터 의존)
- [ ] T027 TLS 종단
- [ ] T028 프로토콜 변환 오버헤드 측정 — 같은 질의의 PostgreSQL 경로 대비
