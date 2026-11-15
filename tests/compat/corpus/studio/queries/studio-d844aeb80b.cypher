CALL db.index.vector.queryNodes('entity_embedding_vector', $top_k, $embedding)
YIELD node, score
RETURN elementId(node) AS node_id,
       [l IN labels(node) WHERE l <> '_Entity'] AS classes,
       node.title AS title,
       node.name AS name,
       node.number AS number,
       substring(toString(node.content), 0, 500) AS content_preview,
       score
