MATCH (p:Proposal {id: $id}) SET p.status = 'SUBMITTED', p.statusHistory = $history
