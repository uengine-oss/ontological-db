#!/usr/bin/env bash
# 동시 라벨 생성 회귀 — SQL 스위트는 세션 하나라 이 결함을 못 잡는다.
#
# 결함: 라벨 생성이 graph 전체의 type_label 을 지우고 다시 넣는데, 두 트랜잭션이
# 겹치면 서로가 다시 넣는 걸 못 보고 지운 뒤 type_label_pkey 로 충돌한다.
# 레거시 애널라이저가 AST 를 병렬로 적재하다 여기서 멈췄다.
#
#   ./tests/concurrency.sh            # 컨테이너의 og DB 에 대고 돈다
set -uo pipefail
CONTAINER="${OG_CONTAINER:-ontological-dev}"
PGPORT="${OG_PGPORT:-28816}"
DB="${OG_DB:-og}"
N="${OG_RACE_N:-16}"
fail=0

run() { docker exec "$CONTAINER" psql -h 127.0.0.1 -U dev -d "$DB" -p "$PGPORT" -qtA "$@"; }

check() {
  local label="$1" graph="$2" cypher="$3" want="$4"
  run -c "SELECT og_drop_graph('$graph');" >/dev/null 2>&1
  run -c "SELECT og_create_graph('$graph');" >/dev/null

  local errs
  errs=$(docker exec "$CONTAINER" bash -lc "
    for i in \$(seq 1 $N); do
      psql -h 127.0.0.1 -U dev -d $DB -p $PGPORT -qtAc \"SELECT og_cypher('$graph', \\\$\\\$ $cypher \\\$\\\$);\" \
        >/tmp/race.\$i 2>&1 &
    done
    wait
    cat /tmp/race.* | grep -ci '^ERROR' || true
    rm -f /tmp/race.*
  ")
  local types
  types=$(run -c "SELECT count(*) FROM og_catalog.type t JOIN og_catalog.graph g USING(graph_id) WHERE g.name='$graph';")
  run -c "SELECT og_drop_graph('$graph');" >/dev/null 2>&1

  if [ "${errs:-1}" -eq 0 ] && [ "${types:-0}" -eq "$want" ]; then
    echo "✓ $label  타입 $types/$want · 오류 0"
  else
    echo "✗ $label  타입 ${types:-0}/$want · 오류 ${errs:-?}"
    fail=$((fail+1))
  fi
}

# 서로 다른 라벨을 동시에 — 각자 graph 전체를 relabel 하며 부딪힌다.
check "서로 다른 라벨 $N 개 동시 생성" og_race_distinct 'CREATE (:L$i {n: $i})' "$N"

# 같은 라벨을 동시에 — 둘 다 "없다"를 보고 둘 다 만들려 든다.
check "같은 라벨을 $N 개 세션이 동시 생성" og_race_same 'CREATE (:Shared {n: $i})' 1

echo
if [ $fail -eq 0 ]; then echo "전부 통과"; else echo "실패 ${fail}건"; fi
exit $fail
