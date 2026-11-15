MATCH (n)
WHERE elementId(n) IN $ids
RETURN elementId(n) AS node_id,
       [l IN labels(n) WHERE l <> '_Entity'] AS labels,
       coalesce(n.name, n.title, toString(n.number)) AS name,
       coalesce(toString(n.content), toString(n.title), toString(n.name), '') AS content
