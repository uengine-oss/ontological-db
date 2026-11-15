MATCH (p:Proposal {id: $id}) SET p.tasksJson = $tasks
