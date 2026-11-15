MATCH (e:Event)
RETURN e.id AS id, coalesce(e.displayName, e.name) AS name,
       coalesce(e.sequence, 0) AS seq, coalesce(e.pivotal, false) AS pivotal
ORDER BY seq, name
