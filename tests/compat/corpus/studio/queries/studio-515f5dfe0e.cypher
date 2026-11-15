MATCH (:_Entity)-[r]->(:_Entity)
RETURN type(r) AS type, count(*) AS count
ORDER BY count DESC
