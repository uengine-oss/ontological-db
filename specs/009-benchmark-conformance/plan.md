# Implementation Plan: 벤치마크 및 적합성 하네스

**Branch**: `009-benchmark-conformance` | **Date**: 2026-08-06 | **Spec**: [spec.md](spec.md)

## Summary

성능 주장을 재현 가능하게 만드는 인프라. **다른 스펙보다 먼저 골격이 서야 한다** —
측정 없이 개발하면 001의 설계 의도가 조용히 침식된다.

**핵심 설계 결정**

1. **비교는 같은 컨테이너에서.** Ontological / Apache AGE / 순수 PostgreSQL 재귀 CTE를
   같은 이미지·같은 하드웨어에서 돌린다. Neo4j는 별도 컨테이너(JVM)로 옵션 처리.
2. **정확성 먼저.** 시스템 간 결과가 다르면 성능 수치를 **무효 처리**한다. 이것이 없는
   벤치마크는 마케팅이다.
3. **회귀 판정은 상대 비교.** CI 환경의 절대 성능은 흔들리므로, 같은 실행 안에서
   기준선 커밋과 현재 커밋을 번갈아 측정해 비율로 판정한다.

## Architecture

```
bench/
├── harness.py          # 실행·측정·리포트
├── workloads/
│   ├── micro.sql       # 1/2/3-hop 확장, 상속 질의, 프로퍼티 스캔
│   └── ldbc/           # LDBC SNB 생성기 연동 (P2)
├── systems/
│   ├── ontological.py
│   ├── age.py          # Apache AGE 동일 질의
│   └── pgsql_cte.py    # 순수 재귀 CTE 기준선
├── conformance/
│   └── tck.py          # openCypher TCK 러너
└── results/            # 시계열 결과 (커밋)
```

**측정 지표**: p50/p95 지연, 논리 페이지 읽기(`EXPLAIN BUFFERS`), 적재 처리량, 저장 크기.
버퍼 읽기를 재는 이유: 지연은 캐시 상태에 흔들리지만 **페이지 접근 수는 저장 구조의
직접적 함수**이므로 001의 설계 주장을 가장 정직하게 검증한다.

## Constitution Check

| 원칙 | 상태 |
|------|------|
| **X** | ✅ 본 스펙이 원칙 X의 구현체 |
| I | ✅ 하네스는 확장 밖. 제품 코드에 영향 없음 |

## Phasing

| Phase | 내용 |
|-------|------|
| P0 | 마이크로 벤치 + AGE/CTE 비교 + 정확성 대조 + 정합성 검사기 |
| P1 | openCypher TCK 러너, 통과율 기준선 |
| P2 | LDBC SNB |
| P3 | CI 회귀 게이트 |
| P4 | 장애 주입, 에이전트 평가 |
