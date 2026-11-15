MATCH (p:Proposal {id: $id}) RETURN p.title AS title, p.tasksJson AS tasksJson
