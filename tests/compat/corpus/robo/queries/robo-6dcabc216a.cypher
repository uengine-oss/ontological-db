MATCH (n:RequirementChange {id: $id}) SET n.status = $s, n.statusHistory = $hist
