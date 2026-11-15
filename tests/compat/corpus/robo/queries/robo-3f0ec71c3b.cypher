MATCH (p:Proposal {id: $id}) SET p.status = 'MERGE_FAILED', p.statusHistory = $history
