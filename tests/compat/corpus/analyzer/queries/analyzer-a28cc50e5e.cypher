UNWIND $items AS item
MERGE (s:ANALYZER_OWNED:SCHEMA {id: item.id, graph_owner: 'analyzer'})
SET s.db = item.db,
    s.name = item.name,
    s.graph_owner = 'analyzer'
RETURN count(s) AS created
