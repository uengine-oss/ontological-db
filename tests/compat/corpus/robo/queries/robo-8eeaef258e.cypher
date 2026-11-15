MATCH (p:Proposal {id: $id}) SET p.sandboxStatus = 'DONE', p.testResults = null
