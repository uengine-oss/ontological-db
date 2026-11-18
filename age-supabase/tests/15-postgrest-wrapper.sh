#!/usr/bin/env bash
# One short PostgREST check (out of scope beyond this): a jsonb wrapper around
# cypher() called as service_role through POST /rpc/cypher. Everything it
# creates is removed at the end.
#   ADMIN_PW_FILE=... ROBO_PW_FILE=... ./15-postgrest-wrapper.sh
set -uo pipefail
DB="${DB:-agesb-db}"; NET="${NET:-agesb-net}"; PORT="${PORT:-55440}"
PW="$(cat "${ADMIN_PW_FILE:?}")"; RPW="$(cat "${ROBO_PW_FILE:?}")"
admin() { docker exec -i -e PGPASSWORD="$PW" "$DB" psql -X -q -v ON_ERROR_STOP=1 -U supabase_admin -h 127.0.0.1 -d postgres "$@"; }
robo()  { docker exec -i -e PGPASSWORD="$RPW" "$DB" psql -X -q -v ON_ERROR_STOP=1 -U robo -h 127.0.0.1 -d postgres "$@"; }

AUTH_PW="$(python3 -c 'import secrets; print(secrets.token_urlsafe(18))')"
JWT_SECRET="$(python3 -c 'import secrets; print(secrets.token_urlsafe(48))')"

admin <<SQL
CREATE SCHEMA graph;
-- query text goes in dollar-quoted with a tag the text does not contain:
-- AGE refuses an ordinary string literal (%L) for the Cypher argument
CREATE FUNCTION graph.cypher(graph_name text, query text, params jsonb DEFAULT '{}', columns text[] DEFAULT ARRAY['result'])
RETURNS SETOF jsonb LANGUAGE plpgsql SECURITY INVOKER
SET search_path = ag_catalog, extensions, public, pg_catalog
AS \$fn\$
DECLARE col_defs text; col_sel text; tag text := '\$cy_' || md5(random()::text) || '\$';
BEGIN
  IF EXISTS (SELECT 1 FROM unnest(columns) c WHERE c !~ '^[a-z_][a-z0-9_]{0,62}\$') THEN
    RAISE EXCEPTION 'invalid column name';
  END IF;
  IF position(tag IN query) > 0 THEN RAISE EXCEPTION 'quote tag collision'; END IF;
  SELECT string_agg(format('%I ag_catalog.agtype', c), ', '),
         string_agg(format('%L, (ag_catalog.agtype_build_list(%I)::jsonb) -> 0', c, c), ', ')
    INTO col_defs, col_sel FROM unnest(columns) c;
  RETURN QUERY EXECUTE format('SELECT jsonb_build_object(%s) FROM ag_catalog.cypher(%L, %s%s%s, \$1) AS (%s)',
                              col_sel, graph_name, tag, query, tag, col_defs)
    USING (params::text)::ag_catalog.agtype;
END
\$fn\$;
REVOKE ALL ON FUNCTION graph.cypher(text, text, jsonb, text[]) FROM PUBLIC, anon, authenticated;
GRANT USAGE ON SCHEMA graph TO service_role;
GRANT EXECUTE ON FUNCTION graph.cypher(text, text, jsonb, text[]) TO service_role;
ALTER ROLE authenticator WITH PASSWORD '$AUTH_PW';
SQL
robo <<'SQL'
GRANT USAGE ON SCHEMA robo_t1 TO service_role;
GRANT SELECT ON ALL TABLES IN SCHEMA robo_t1 TO service_role;
SQL

docker run -d --name agesb-rest --network "$NET" --memory 300m --cpus 1 -p "127.0.0.1:${PORT}:3000" \
  -e PGRST_DB_URI="postgres://authenticator:${AUTH_PW}@${DB}:5432/postgres" \
  -e PGRST_DB_SCHEMAS=public,graph -e PGRST_DB_ANON_ROLE=anon \
  -e PGRST_JWT_SECRET="$JWT_SECRET" postgrest/postgrest:v12.2.12 >/dev/null
for _ in $(seq 1 30); do curl -sf "http://127.0.0.1:${PORT}/" >/dev/null 2>&1 && break; sleep 1; done

JWT="$(JWT_SECRET="$JWT_SECRET" python3 -I -c '
import base64, hashlib, hmac, json, os, time
b = lambda d: base64.urlsafe_b64encode(d).rstrip(b"=").decode()
h = b(json.dumps({"alg": "HS256", "typ": "JWT"}).encode())
p = b(json.dumps({"role": "service_role", "exp": int(time.time()) + 600}).encode())
s = b(hmac.new(os.environ["JWT_SECRET"].encode(), f"{h}.{p}".encode(), hashlib.sha256).digest())
print(f"{h}.{p}.{s}")')"

echo "### service_role: POST /rpc/cypher with a parameter"
curl -s -X POST "http://127.0.0.1:${PORT}/rpc/cypher" -H "Authorization: Bearer $JWT" \
  -H 'Content-Type: application/json' -H 'Content-Profile: graph' \
  -d '{"graph_name": "robo_t1", "query": "MATCH (a:Aggregate)-[r:HAS]->(b) WHERE a.name = $name RETURN a, r, b.name, a.version", "params": {"name": "Order"}, "columns": ["a", "r", "b_name", "version"]}'
echo
echo "### anon: refused"
curl -s -X POST "http://127.0.0.1:${PORT}/rpc/cypher" -H 'Content-Type: application/json' -H 'Content-Profile: graph' \
  -d '{"graph_name": "robo_t1", "query": "MATCH (n) RETURN count(n)", "columns": ["c"]}'
echo

echo "### cleanup"
docker rm -f agesb-rest >/dev/null
robo <<'SQL'
REVOKE SELECT ON ALL TABLES IN SCHEMA robo_t1 FROM service_role;
REVOKE USAGE ON SCHEMA robo_t1 FROM service_role;
SQL
admin <<'SQL'
DROP SCHEMA graph CASCADE;
ALTER ROLE authenticator WITH PASSWORD NULL;
SQL
echo done
