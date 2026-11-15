MATCH (p:Proposal {id: $id}) SET p.status = 'SUBMITTED', p.sandboxStatus = null, p.sandboxBranch = null, p.sandboxWorktreePath = null, p.testResults = null
