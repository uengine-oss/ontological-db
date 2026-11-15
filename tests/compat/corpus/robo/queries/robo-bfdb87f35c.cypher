MATCH (p:Proposal {id: $id}) RETURN p.status AS status, p.sandboxWorktreePath AS worktreePath
