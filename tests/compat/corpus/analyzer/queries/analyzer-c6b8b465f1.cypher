UNWIND $items AS item
MATCH (s:TABLE)-[r]->(t:TABLE) WHERE elementId(r) = item.rid
SET r += item.props
RETURN count(r) AS updated
