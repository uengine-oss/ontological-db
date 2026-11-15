MATCH (s:TABLE)-[r]->(t:TABLE)
WHERE s._owner = $owner AND t._owner = $owner AND s.datasource IN $datasources
  AND type(r) IN ['FK', 'FK_CANDIDATE']
RETURN elementId(r) AS rid, type(r) AS type, r{.*} AS props,
       elementId(s) AS sid, s.datasource AS from_datasource, s.schema AS from_schema, s.name AS from_table,
       elementId(t) AS tid, t.datasource AS to_datasource, t.schema AS to_schema, t.name AS to_table
