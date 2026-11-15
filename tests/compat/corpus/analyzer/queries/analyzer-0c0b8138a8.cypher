MATCH (t:TABLE) WHERE t._owner = $owner AND t.datasource = $ds RETURN DISTINCT coalesce(t.schema + '.' + t.name, t.name) AS fqn
