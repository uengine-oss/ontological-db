MATCH (p:Proposal {id: $id}) RETURN p.testResults AS testResults, p.status AS status
