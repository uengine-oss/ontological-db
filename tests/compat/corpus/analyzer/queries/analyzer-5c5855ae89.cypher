UNWIND $items AS item
MATCH (s:TABLE) WHERE elementId(s) = item.sid
MATCH (t:TABLE) WHERE elementId(t) = item.tid
CREATE (s)-[r:FK_CANDIDATE]->(t)
SET r = item.props
RETURN count(r) AS created
