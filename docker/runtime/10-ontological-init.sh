#!/usr/bin/env bash
# Ontological 확장 설치 + graph 생성 — 공식 postgres 엔트리포인트의 initdb 훅.
#
# ## 언제 도는가
#
# 공식 이미지는 이 디렉터리의 스크립트를 **PGDATA 가 비어 있을 때만** 돌린다.
# 볼륨이 이미 있으면 안 돈다. 그래서 여기서 하는 일은 전부 **멱등**이어야 하고,
# "이미 있음"을 실패로 보지 않아야 한다.
#
# ## 조용히 넘어가지 않는다
#
# `set -e` 와 `ON_ERROR_STOP=1` 을 같이 쓴다. 확장이 안 깔리거나 graph 가 안 만들어졌는데
# 컨테이너가 healthy 로 뜨면, 앱은 연결에 성공한 뒤 **조회에서야** 터진다. 이 저장소가
# 실제로 밟은 그 모양이다(`DatabaseNotFound` 를 로그에서 찾아낸 건).
#
# ## 어떤 graph 를 만드나
#
#   OG_GRAPH_DESIGN    설계 데이터 (기본 robo)
#   OG_GRAPH_ANALYSIS  레거시 분석 (기본 analyzer_run)
#
# **둘은 반드시 달라야 한다.** 분석은 대상 graph 를 통째로 비우고 다시 쓴다 — 같으면
# 설계가 사라진다. 같은 값이 들어오면 여기서 멈춘다. 나중에 화면에서 막는 것보다
# 싸다(spec 058 FR-008).
#
#   OG_GRAPHS_EXTRA    쉼표로 구분한 추가 graph (선택). 프로젝트별 graph 를 미리 만들 때.

set -euo pipefail

DESIGN_GRAPH="${OG_GRAPH_DESIGN:-robo}"
ANALYSIS_GRAPH="${OG_GRAPH_ANALYSIS:-analyzer_run}"

say() { printf '[ontological-init] %s\n' "$1"; }

if [ "$DESIGN_GRAPH" = "$ANALYSIS_GRAPH" ]; then
  say "설계 graph 와 분석 graph 가 같다: '$DESIGN_GRAPH'"
  say "  분석은 대상 graph 를 비우고 다시 쓴다 — 같으면 설계가 사라진다."
  say "  OG_GRAPH_DESIGN 과 OG_GRAPH_ANALYSIS 를 다르게 줘라."
  exit 1
fi

# graph 이름이 그대로 SQL 문자열이 된다. 검증한다.
for name in "$DESIGN_GRAPH" "$ANALYSIS_GRAPH"; do
  if ! printf '%s' "$name" | grep -Eq '^[A-Za-z_][A-Za-z0-9_]{0,62}$'; then
    say "graph 이름이 규칙에 안 맞는다: '$name' (영문/숫자/밑줄, 첫 글자는 영문·밑줄)"
    exit 1
  fi
done

psql_db() {
  psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$1" "${@:2}"
}

say "확장 설치 — ontological (CASCADE 로 pgvector 를 끌어온다)"
psql_db "$POSTGRES_DB" -c 'CREATE EXTENSION IF NOT EXISTS ontological CASCADE'

# 실제로 깔렸는지 되읽는다. `IF NOT EXISTS` 는 실패해도 조용할 수 있다.
installed=$(psql_db "$POSTGRES_DB" -tAc \
  "SELECT count(*) FROM pg_extension WHERE extname IN ('ontological','vector')")
if [ "$installed" != "2" ]; then
  say "확장이 둘 다 안 깔렸다 (pg_extension 에 $installed 건)"
  psql_db "$POSTGRES_DB" -c "SELECT extname, extversion FROM pg_extension" || true
  exit 1
fi

create_graph() {
  local graph="$1"
  # `og_create_graph` 는 **이미 멱등하다** — 같은 이름이 있으면 기존 graph_id 를
  # 돌려준다(`engine/src/catalog/types.rs`). 감싸지 않는다. 재구현이 원본보다
  # 옳아 보이면 원본을 먼저 읽는다.
  psql_db "$POSTGRES_DB" -tAc "SELECT og_create_graph('$graph')" >/dev/null
  # **만들었다고 믿지 않고 되읽는다.**
  local found
  found=$(psql_db "$POSTGRES_DB" -tAc \
    "SELECT count(*) FROM og_catalog.graph WHERE name = '$graph'")
  if [ "$found" != "1" ]; then
    say "graph 를 못 만들었다: '$graph'"
    exit 1
  fi
  say "graph 준비됨 — $graph"
}

create_graph "$DESIGN_GRAPH"
create_graph "$ANALYSIS_GRAPH"

if [ -n "${OG_GRAPHS_EXTRA:-}" ]; then
  IFS=',' read -ra extra <<< "$OG_GRAPHS_EXTRA"
  for graph in "${extra[@]}"; do
    graph="$(printf '%s' "$graph" | tr -d '[:space:]')"
    [ -z "$graph" ] && continue
    if ! printf '%s' "$graph" | grep -Eq '^[A-Za-z_][A-Za-z0-9_]{0,62}$'; then
      say "추가 graph 이름이 규칙에 안 맞는다: '$graph'"
      exit 1
    fi
    create_graph "$graph"
  done
fi

say "완료 — 설계=$DESIGN_GRAPH · 분석=$ANALYSIS_GRAPH"
