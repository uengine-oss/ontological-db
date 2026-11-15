MATCH (n:Command {id: $id}) DETACH DELETE n RETURN count(n) AS cnt
