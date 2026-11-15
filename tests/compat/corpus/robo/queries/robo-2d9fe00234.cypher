MATCH (p:Proposal {id: $id}) SET p.status = $status, p.statusHistory = $history
