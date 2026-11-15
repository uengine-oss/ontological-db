MATCH (n {id: $id}) RETURN n.id AS id, n.name AS name, n.displayName AS displayName, labels(n) AS labels
