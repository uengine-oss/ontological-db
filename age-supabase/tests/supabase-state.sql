-- Snapshot of what Supabase's initdb set up, as sorted text lines, so a stock
-- supabase/postgres container and the AGE image can be diffed line by line.
-- Run as supabase_admin: psql -X -At -f supabase-state.sql
\pset footer off
SELECT 'setting ' || name || ' = ' || setting
  FROM pg_settings
 WHERE name IN ('server_version', 'shared_preload_libraries', 'session_preload_libraries',
                'wal_level', 'search_path', 'password_encryption')
 ORDER BY 1;
SELECT 'role ' || rolname || ' super=' || rolsuper || ' login=' || rolcanlogin
       || ' createrole=' || rolcreaterole || ' createdb=' || rolcreatedb
       || ' repl=' || rolreplication || ' bypassrls=' || rolbypassrls || ' inherit=' || rolinherit
  FROM pg_roles WHERE rolname !~ '^pg_' ORDER BY 1;
SELECT 'member ' || r.rolname || ' in ' || g.rolname
  FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member JOIN pg_roles g ON g.oid = m.roleid
 WHERE g.rolname !~ '^pg_' OR r.rolname !~ '^pg_' ORDER BY 1;
SELECT 'role_setting ' || coalesce(r.rolname, '*') || '@' || coalesce(d.datname, '*') || ' ' || array_to_string(s.setconfig, ',')
  FROM pg_db_role_setting s LEFT JOIN pg_roles r ON r.oid = s.setrole LEFT JOIN pg_database d ON d.oid = s.setdatabase
 ORDER BY 1;
SELECT 'database ' || datname FROM pg_database ORDER BY 1;
SELECT 'extension ' || extname || ' ' || extversion || ' schema=' || extnamespace::regnamespace
  FROM pg_extension ORDER BY 1;
SELECT 'schema ' || nspname || ' owner=' || nspowner::regrole
  FROM pg_namespace WHERE nspname !~ '^pg_(toast|temp)' ORDER BY 1;
SELECT 'event_trigger ' || evtname || ' ' || evtevent || ' ' || evtfoid::regproc || ' enabled=' || evtenabled::text
  FROM pg_event_trigger ORDER BY 1;
SELECT 'publication ' || pubname FROM pg_publication ORDER BY 1;
SELECT 'objects_in ' || n.nspname || ' rel=' || count(DISTINCT c.oid)
  FROM pg_namespace n LEFT JOIN pg_class c ON c.relnamespace = n.oid
 WHERE n.nspname IN ('auth', 'storage', 'realtime', 'extensions', 'graphql', 'graphql_public', 'vault', 'pgsodium', 'net', 'cron', 'public')
 GROUP BY n.nspname ORDER BY 1;
SELECT 'functions_in ' || n.nspname || ' n=' || count(p.oid)
  FROM pg_namespace n LEFT JOIN pg_proc p ON p.pronamespace = n.oid
 WHERE n.nspname IN ('auth', 'storage', 'realtime', 'extensions', 'graphql', 'graphql_public', 'vault', 'pgsodium', 'net', 'cron', 'public')
 GROUP BY n.nspname ORDER BY 1;
SELECT 'bgworker ' || backend_type || coalesce(' ' || application_name, '')
  FROM pg_stat_activity WHERE backend_type NOT IN ('client backend') ORDER BY 1;
