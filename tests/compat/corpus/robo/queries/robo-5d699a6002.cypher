MATCH (p:Proposal {id: $id}) SET p.status = $status, p.updatedAt = $updatedAt
