MATCH (n) WHERE elementId(n) = $id SET n.name = $name, n.grade = $grade, n.updated_at = datetime() RETURN n
