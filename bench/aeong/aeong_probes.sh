#!/bin/bash
# AeonG 의 시점 질의(TT AS / TT FROM..TO)가 틀린 답을 내는 최소 재현 사례.
# 빈 AeonG (기본 = 논리 타임스탬프 모드) 에 대고 실행한다.
#
#   bash bench/aeong/aeong_probes.sh [컨테이너 이름, 기본 aeong]
#
# 논리 모드에서 TT 값은 커밋 카운터라 사용자가 "어느 시각"을 미리 알 수 없다.
# 그래서 0..N 을 훑으며 결과가 바뀌는 지점을 본다.
C=${1:-aeong}
q() { echo "$1;" | docker exec -i "$C" mgconsole --host 127.0.0.1 --port 7687 --output-format csv 2>&1 | tail -n +2 | tr '\n' ' '; }
scan() {  # scan <최대 ts>  — 노드 속성과 관계를 시점별로
  for t in $(seq 0 "$1"); do
    printf "  TT AS %-3s  p.v=%-10s r.w=%s\n" "$t" "$(q "MATCH (p:P) TT AS $t RETURN p.v")" "$(q "MATCH (:P)-[r:R]->() TT AS $t RETURN r.w")"
  done
}

q "MATCH (n) DETACH DELETE n" >/dev/null
echo "== P1. 노드 속성만 바뀔 때: 정상 =="
q "CREATE (:P {v:1})-[:R {w:'a'}]->(:C)" >/dev/null
q "MATCH (p:P) SET p.v=2" >/dev/null
q "MATCH (p:P) SET p.v=3" >/dev/null
scan 8

echo "== P2. 붙어 있던 관계를 지운 뒤 같은 질문: 모든 과거 시점이 현재값(3), 관계는 사라짐 =="
q "MATCH (:P)-[r:R]->() DELETE r" >/dev/null
scan 8

echo "== P3. 구간 질의: 존재하지 않는 'NULL' 버전이 섞여 나온다 =="
echo "  TT FROM 0 TO 1e14 -> $(q 'MATCH (p:P) TT FROM 0 TO 99999999999999 RETURN p.v')"

echo "== P4. 노드를 지우면 그 노드의 이력 전체가 시점 질의로 안 보인다 =="
q "CREATE (:Q {v:7})" >/dev/null
q "MATCH (q:Q) SET q.v=8" >/dev/null
echo "  삭제 전 구간: $(q 'MATCH (q:Q) TT FROM 0 TO 99999999 RETURN q.v')"
q "MATCH (q:Q) DELETE q" >/dev/null
echo "  삭제 후 구간: [$(q 'MATCH (q:Q) TT FROM 0 TO 99999999 RETURN q.v')]"

echo "== P5. 문법에 있는 VT(유효시간) 절은 파서가 거절한다 =="
echo "  $(q 'MATCH (c:C) VT AS 0 RETURN c')"

echo "== P6. GC(FREE MEMORY) 후에도 P2 의 결과는 그대로 =="
q "FREE MEMORY" >/dev/null
for t in 2 4 6; do echo "  TT AS $t -> $(q "MATCH (p:P) TT AS $t RETURN p.v")"; done
