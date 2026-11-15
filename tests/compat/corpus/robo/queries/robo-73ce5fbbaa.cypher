MATCH (n:RequirementChange {id: $id}) SET n.status = $status, n.statusHistory = $hist
