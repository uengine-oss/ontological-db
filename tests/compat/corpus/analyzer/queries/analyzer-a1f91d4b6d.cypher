MATCH (t:TABLE)
WHERE t._owner = $owner AND t.datasource IN $datasources
RETURN elementId(t) AS id, t.datasource AS datasource, t.schema AS schema, t.name AS name
