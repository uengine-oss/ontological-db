-- ============================================================================
-- Releases, schema diff and timelines.
--
-- A schema built and tested in one database (development) reaches another
-- (operations) as a *release*: a package carrying the source schema snapshot,
-- the source change log and the instances. Three things the catalog did not
-- have make that work, and they live here so that every client — the authoring
-- tool, a CI job, the portal — gets the same answer from the database instead
-- of each computing its own:
--
--   og_schema_diff   what changes would make this graph's schema equal to a
--                    snapshot taken elsewhere — as change specs og_schema_plan
--                    accepts, so applying a release is the ordinary,
--                    phase-by-phase schema change and is recorded as one.
--   og_release_*     one row per release: which package (by hash), which schema
--                    versions it moved between, which changes it ran, how many
--                    instances it wrote, who did it. Writes made inside a
--                    release carry og.source = 'release:<id>'.
--   og_timeline      one element's slices over a period — every validity
--                    interval that overlaps [from, to), as it was believed at a
--                    system instant. The time-travel settings answer "what was
--                    true at t"; this answers "how did it change between a and b".
-- ============================================================================

CREATE TABLE og_catalog.release (
    release_id   serial PRIMARY KEY,
    graph_id     int4 NOT NULL REFERENCES og_catalog.graph(graph_id) ON DELETE CASCADE,
    name         text NOT NULL,
    origin       text,                         -- where the package came from
    package_hash text,
    status       text NOT NULL DEFAULT 'applying'
                 CHECK (status IN ('applying','applied','failed')),
    from_version int8,
    to_version   int8,
    change_ids   int4[] NOT NULL DEFAULT '{}',
    instances    int8 NOT NULL DEFAULT 0,
    summary      jsonb NOT NULL DEFAULT '{}'::jsonb,
    actor        text,
    created_by   text NOT NULL DEFAULT current_user,
    started_at   timestamptz NOT NULL DEFAULT now(),
    finished_at  timestamptz
);
CREATE INDEX release_graph_idx ON og_catalog.release (graph_id, release_id DESC);
SELECT pg_catalog.pg_extension_config_dump('og_catalog.release', '');
SELECT pg_catalog.pg_extension_config_dump('og_catalog.release_release_id_seq', '');

-- ---------------------------------------------------------------------------
-- Schema diff
--
-- `target` is an og_schema_snapshot document. `hints` is the source's change
-- log (og_schema_changes rows as JSON): a rename cannot be told apart from a
-- drop plus an add by comparing two snapshots, so a rename is taken from the
-- log when the old name exists here and the new one does not.
--
-- Properties are compared where they are declared. A subtype's copy of an
-- inherited property is not a separate change — adding it to the parent adds it
-- to the subtype.
--
-- Output, in the order it must run: create_type (parents before children),
-- rename_type, rename_property, add_property, alter_property_type,
-- enable_temporal, and — marked destructive, never applied by a release on its
-- own — drop_property.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_schema_diff(graph text, target jsonb, hints jsonb DEFAULT '[]'::jsonb)
RETURNS jsonb LANGUAGE plpgsql STABLE AS $$
DECLARE
    cur      jsonb := COALESCE(og_schema_snapshot(graph), '{}'::jsonb);
    out      jsonb := '[]'::jsonb;
    ct       jsonb := '{}'::jsonb;   -- current: type name -> type doc
    tt       jsonb := '{}'::jsonb;   -- target:  type name -> type doc
    t        jsonb;
    h        jsonb;
    p        record;
    n        text;
    pending  text[];
    progress bool;
    inherited bool;
BEGIN
    SELECT COALESCE(jsonb_object_agg(x->>'name', x), '{}'::jsonb) INTO ct
      FROM jsonb_array_elements(COALESCE(cur->'types', '[]'::jsonb)) x;
    SELECT COALESCE(jsonb_object_agg(x->>'name', x), '{}'::jsonb) INTO tt
      FROM jsonb_array_elements(COALESCE(target->'types', '[]'::jsonb)) x;

    -- renames from the source log, oldest first
    FOR h IN SELECT x FROM jsonb_array_elements(COALESCE(hints, '[]'::jsonb)) x
              WHERE x->>'phase' IN ('contracted') ORDER BY (x->>'change_id')::int
    LOOP
        IF h->>'kind' = 'rename_type'
           AND ct ? (h->'spec'->>'type') AND NOT ct ? (h->'spec'->>'new_name')
           AND tt ? (h->'spec'->>'new_name') THEN
            out := out || jsonb_build_array(jsonb_build_object('kind', 'rename_type',
                       'type', h->'spec'->>'type', 'new_name', h->'spec'->>'new_name'));
            ct := (ct - (h->'spec'->>'type'))
                  || jsonb_build_object(h->'spec'->>'new_name', ct->(h->'spec'->>'type'));
        ELSIF h->>'kind' = 'rename_property' THEN
            n := h->'spec'->>'type';
            IF ct ? n AND (ct->n->'properties') ? (h->'spec'->>'property')
               AND NOT (ct->n->'properties') ? (h->'spec'->>'new_name')
               AND (tt->n->'properties') ? (h->'spec'->>'new_name') THEN
                out := out || jsonb_build_array(jsonb_build_object('kind', 'rename_property',
                           'type', n, 'property', h->'spec'->>'property',
                           'new_name', h->'spec'->>'new_name'));
                -- the rename reaches every subtype; reflect it so they are not re-added
                FOR p IN SELECT key FROM jsonb_each(ct) LOOP
                    IF (ct->p.key->'properties') ? (h->'spec'->>'property') THEN
                        ct := jsonb_set(ct, ARRAY[p.key, 'properties'],
                              ((ct->p.key->'properties') - (h->'spec'->>'property'))
                              || jsonb_build_object(h->'spec'->>'new_name',
                                     ct->p.key->'properties'->(h->'spec'->>'property')));
                    END IF;
                END LOOP;
            END IF;
        END IF;
    END LOOP;

    -- new types, parents first
    SELECT array_agg(k) INTO pending FROM jsonb_object_keys(tt) k WHERE NOT ct ? k;
    pending := COALESCE(pending, '{}');
    LOOP
        progress := false;
        FOREACH n IN ARRAY pending LOOP
            t := tt->n;
            IF NOT EXISTS (SELECT 1 FROM jsonb_array_elements_text(t->'parents') par
                            WHERE NOT ct ? par) THEN
                out := out || jsonb_build_array(jsonb_build_object('kind', 'create_type',
                           'type', n, 'type_kind', t->>'kind', 'parents', t->'parents',
                           'abstract', COALESCE((t->>'abstract')::bool, false)));
                -- a new type starts with what it inherits
                ct := ct || jsonb_build_object(n, jsonb_build_object('name', n,
                        'kind', t->>'kind', 'parents', t->'parents', 'temporal', false,
                        'properties', COALESCE((SELECT jsonb_object_agg(k, v)
                              FROM jsonb_array_elements_text(t->'parents') par,
                                   jsonb_each(ct->par->'properties') e(k, v)), '{}'::jsonb)));
                pending := array_remove(pending, n);
                progress := true;
            END IF;
        END LOOP;
        EXIT WHEN NOT progress OR cardinality(pending) = 0;
    END LOOP;
    IF cardinality(pending) > 0 THEN
        RAISE EXCEPTION 'OG_RELEASE_DIFF: types % have parents that exist in neither schema', pending
            USING ERRCODE = '22023';
    END IF;

    -- properties: added where declared, retyped, and (destructive) dropped.
    -- Parents are visited before children so a property added to a parent is
    -- reflected in its subtypes before they are looked at.
    FOR t IN
        WITH RECURSIVE depth(name, d) AS (
            SELECT k, 0 FROM jsonb_object_keys(tt) k
             WHERE jsonb_array_length(COALESCE(tt->k->'parents', '[]'::jsonb)) = 0
            UNION ALL
            SELECT k, depth.d + 1 FROM depth, jsonb_object_keys(tt) k
             WHERE tt->k->'parents' ? depth.name
        )
        SELECT tt->name FROM depth GROUP BY name ORDER BY max(d), name
    LOOP
        n := t->>'name';
        FOR p IN SELECT key, value FROM jsonb_each(COALESCE(t->'properties', '{}'::jsonb)) LOOP
            SELECT EXISTS (SELECT 1 FROM jsonb_array_elements_text(t->'parents') par
                            WHERE (tt->par->'properties') ? p.key) INTO inherited;
            IF NOT (ct->n->'properties') ? p.key THEN
                IF NOT inherited THEN
                    out := out || jsonb_build_array(jsonb_build_object('kind', 'add_property',
                               'type', n, 'property', p.key, 'data_type', p.value->>'type'));
                END IF;
                -- this type and everything below it now has it
                ct := jsonb_set(ct, ARRAY[n, 'properties', p.key], p.value);
                FOR h IN
                    WITH RECURSIVE sub(name) AS (
                        SELECT k FROM jsonb_object_keys(tt) k WHERE tt->k->'parents' ? n
                        UNION SELECT k FROM sub, jsonb_object_keys(tt) k WHERE tt->k->'parents' ? sub.name)
                    SELECT to_jsonb(name) FROM sub
                LOOP
                    IF ct ? (h #>> '{}') THEN
                        ct := jsonb_set(ct, ARRAY[h #>> '{}', 'properties', p.key], p.value);
                    END IF;
                END LOOP;
            ELSIF (ct->n->'properties'->p.key->>'type') IS DISTINCT FROM (p.value->>'type')
                  AND NOT inherited THEN
                out := out || jsonb_build_array(jsonb_build_object('kind', 'alter_property_type',
                           'type', n, 'property', p.key,
                           'from', ct->n->'properties'->p.key->>'type', 'to', p.value->>'type'));
            END IF;
        END LOOP;
        IF COALESCE((t->>'temporal')::bool, false) AND NOT COALESCE((ct->n->>'temporal')::bool, false) THEN
            -- enabling a supertype covers its subtypes; don't repeat it for them
            IF NOT EXISTS (SELECT 1 FROM jsonb_array_elements_text(t->'parents') par
                            WHERE COALESCE((tt->par->>'temporal')::bool, false)) THEN
                out := out || jsonb_build_array(jsonb_build_object('kind', 'enable_temporal', 'type', n));
            END IF;
        END IF;
        FOR p IN SELECT key FROM jsonb_each(COALESCE(ct->n->'properties', '{}'::jsonb))
                  WHERE NOT (t->'properties') ? key LOOP
            IF NOT EXISTS (SELECT 1 FROM jsonb_array_elements_text(t->'parents') par
                            WHERE NOT (tt->par->'properties') ? p.key AND (ct->par->'properties') ? p.key) THEN
                out := out || jsonb_build_array(jsonb_build_object('kind', 'drop_property',
                           'type', n, 'property', p.key, 'destructive', true));
            END IF;
        END LOOP;
    END LOOP;

    -- stable order: by the kind's place in the sequence above, then as built
    RETURN COALESCE((
        SELECT jsonb_agg(c ORDER BY array_position(ARRAY['rename_type','rename_property','create_type',
                   'add_property','alter_property_type','enable_temporal','drop_property'], c->>'kind'), o)
          FROM jsonb_array_elements(out) WITH ORDINALITY AS e(c, o)), '[]'::jsonb);
END $$;

-- ---------------------------------------------------------------------------
-- Releases
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_release_begin(graph text, name text, origin text DEFAULT NULL,
                                 package_hash text DEFAULT NULL)
RETURNS int4 LANGUAGE plpgsql AS $$
DECLARE
    gid int4;
    rid int4;
BEGIN
    SELECT graph_id INTO gid FROM og_catalog.graph g WHERE g.name = og_release_begin.graph;
    IF gid IS NULL THEN
        RAISE EXCEPTION 'graph ''%'' does not exist', graph USING ERRCODE = '42704';
    END IF;
    IF package_hash IS NOT NULL AND EXISTS (
        SELECT 1 FROM og_catalog.release r
         WHERE r.graph_id = gid AND r.package_hash = og_release_begin.package_hash AND r.status = 'applied') THEN
        RAISE EXCEPTION 'OG_RELEASE_DUPLICATE: package % is already applied to graph %', package_hash, graph
            USING ERRCODE = '23505';
    END IF;
    INSERT INTO og_catalog.release (graph_id, name, origin, package_hash, from_version, actor)
    VALUES (gid, og_release_begin.name, origin, package_hash,
            (SELECT max(version) FROM og_catalog.schema_version sv WHERE sv.graph_id = gid),
            NULLIF(current_setting('og.actor', true), ''))
    RETURNING release_id INTO rid;
    RETURN rid;
END $$;

CREATE FUNCTION og_release_commit(release_id int4, change_ids int4[] DEFAULT '{}',
                                  instances int8 DEFAULT 0, summary jsonb DEFAULT '{}'::jsonb,
                                  failed bool DEFAULT false)
RETURNS jsonb LANGUAGE plpgsql AS $$
DECLARE
    r og_catalog.release;
BEGIN
    UPDATE og_catalog.release x
       SET status      = CASE WHEN failed THEN 'failed' ELSE 'applied' END,
           change_ids  = og_release_commit.change_ids,
           instances   = og_release_commit.instances,
           summary     = og_release_commit.summary,
           to_version  = (SELECT max(version) FROM og_catalog.schema_version sv WHERE sv.graph_id = x.graph_id),
           finished_at = now()
     WHERE x.release_id = og_release_commit.release_id AND x.status = 'applying'
    RETURNING * INTO r;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'release % is not being applied', release_id USING ERRCODE = '55000';
    END IF;
    RETURN to_jsonb(r);
END $$;

CREATE FUNCTION og_releases(graph text)
RETURNS SETOF jsonb LANGUAGE sql STABLE AS $$
    SELECT to_jsonb(r) - 'graph_id'
      FROM og_catalog.release r JOIN og_catalog.graph g USING (graph_id)
     WHERE g.name = og_releases.graph
     ORDER BY r.release_id DESC
$$;

-- ---------------------------------------------------------------------------
-- Timeline
--
-- Every slice of the elements of `type` (and its subtypes) whose `key_prop`
-- equals `key`, whose validity overlaps [from, to), and which was believed at
-- `system_at` (now when NULL). A current row is believed from __sys_from on; a
-- history row over [__sys_from, __sys_to). Properties come back under their
-- current names — a renamed property keeps its column.
-- ---------------------------------------------------------------------------
CREATE FUNCTION og_timeline(graph text, type text, key_prop text, key text,
                            "from" timestamptz DEFAULT '-infinity',
                            "to" timestamptz DEFAULT 'infinity',
                            system_at timestamptz DEFAULT NULL)
RETURNS TABLE (type_name text, id int8, valid_from timestamptz, valid_to timestamptz,
               props jsonb, recorded_at timestamptz, superseded_at timestamptz,
               source text, actor text, op text)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    gid  int4;
    root int4;
    sa   timestamptz := COALESCE(system_at, now());
    tr   record;
    cols jsonb;
    kcol text;
    hist bool;
    q    text;
BEGIN
    SELECT graph_id INTO gid FROM og_catalog.graph g WHERE g.name = og_timeline.graph;
    SELECT type_id INTO root FROM og_catalog.type t
     WHERE t.graph_id = gid AND t.name = og_timeline.type;
    IF root IS NULL THEN
        RAISE EXCEPTION 'type ''%'' does not exist in graph ''%''', og_timeline.type, graph
            USING ERRCODE = '42704';
    END IF;

    FOR tr IN
        WITH RECURSIVE sub(type_id) AS (
            SELECT root
            UNION SELECT tp.type_id FROM og_catalog.type_parent tp JOIN sub ON tp.parent_id = sub.type_id)
        SELECT t.type_id, t.name, t.storage_table FROM og_catalog.type t JOIN sub USING (type_id)
         WHERE t.storage_table IS NOT NULL
    LOOP
        SELECT jsonb_object_agg(column_name, name) INTO cols
          FROM og_catalog.property WHERE property.type_id = tr.type_id;
        cols := COALESCE(cols, '{}'::jsonb);
        SELECT column_name INTO kcol FROM og_catalog.property
         WHERE property.type_id = tr.type_id AND name = key_prop;
        hist := EXISTS (SELECT 1 FROM og_catalog.temporal x WHERE x.type_id = tr.type_id)
             OR EXISTS (SELECT 1 FROM og_catalog.temporal x
                         WHERE x.type_id = ANY(og_supertypes(tr.type_id)));

        q := format($q$
            SELECT %L::text, (r.d->>'id')::int8,
                   lower(r.v), upper(r.v),
                   COALESCE((SELECT jsonb_object_agg(COALESCE($1->>k, k), val)
                               FROM jsonb_each(og__visible(r.d) - 'id' - '__ext') AS e(k, val)
                              WHERE val <> 'null'::jsonb), '{}'::jsonb)
                     || COALESCE(NULLIF(r.d->'__ext', 'null'::jsonb), '{}'::jsonb),
                   r.sf, NULLIF(r.st, 'infinity'::timestamptz), r.d->>'__source', r.d->>'__actor', r.op
              FROM (%s) r
             WHERE %s = $2
               AND r.v && tstzrange($3, $4, '[)')
               AND r.sf <= $5 AND r.st > $5
             ORDER BY lower(r.v), r.sf$q$,
            tr.name,
            CASE WHEN hist THEN format(
                'SELECT to_jsonb(x) AS d, x.__valid AS v, x.__sys_from AS sf, %L::timestamptz AS st, %L::text AS op FROM %s x
                 UNION ALL
                 SELECT to_jsonb(h) - ''__sys_to'' - ''__op'', h.__valid, h.__sys_from, h.__sys_to, h.__op FROM og_data.%I h',
                'infinity', 'current', tr.storage_table, 'h_' || tr.type_id)
            ELSE format(
                'SELECT to_jsonb(x) AS d, tstzrange(%L, %L) AS v, %L::timestamptz AS sf, %L::timestamptz AS st, %L::text AS op FROM %s x',
                '-infinity', 'infinity', '-infinity', 'infinity', 'current', tr.storage_table)
            END,
            CASE WHEN kcol IS NOT NULL THEN format('r.d->>%L', kcol)
                 ELSE format('r.d->''__ext''->>%L', key_prop) END);

        RETURN QUERY EXECUTE q USING cols, key, "from", "to", sa;
    END LOOP;
END $$;
