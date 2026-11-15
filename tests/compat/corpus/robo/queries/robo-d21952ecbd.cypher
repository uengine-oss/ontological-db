UNWIND $ids AS eid
MATCH (e {id: eid})
RETURN e.id AS id, coalesce(e.version, 0) AS version
