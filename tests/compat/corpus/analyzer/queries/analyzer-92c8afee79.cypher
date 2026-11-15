UNWIND $items AS item
MERGE (t:ANALYZER_OWNED:TABLE {id: item.id, graph_owner: 'analyzer'})
SET t.db = item.db,
    t.schema = item.schema,
    t.name = item.name,
    t.description = coalesce(t.description, item.description),
    t.description_source = coalesce(t.description_source, item.description_source),
    t.comment = coalesce(t.comment, item.comment),
    t.analyzed_description = coalesce(t.analyzed_description, item.analyzed_description),
    t.table_type = item.table_type,
    t.graph_owner = 'analyzer'
RETURN count(t) AS created
