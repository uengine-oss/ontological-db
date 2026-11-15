MATCH (p:Proposal {id: $id}) RETURN p.statusHistory AS history
