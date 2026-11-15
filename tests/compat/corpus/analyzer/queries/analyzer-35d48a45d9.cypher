UNWIND $items AS item
MERGE (c:ANALYZER_OWNED:COLUMN {id: item.id, graph_owner: 'analyzer'})
SET c.name = item.name,
    c.dtype = item.dtype,
    c.nullable = item.nullable,
    c.is_primary_key = item.is_primary_key,
    c.is_unique = item.is_unique,
    c.default_value = item.default_value,
    c.description = coalesce(c.description, item.description),
    c.description_source = coalesce(c.description_source, item.description_source),
    c.comment = coalesce(c.comment, item.comment),
    c.analyzed_description = coalesce(c.analyzed_description, item.analyzed_description),
    c.graph_owner = 'analyzer'
RETURN count(c) AS created
