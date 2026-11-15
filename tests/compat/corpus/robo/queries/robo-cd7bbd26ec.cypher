MATCH (p:Proposal {id: $id})
SET p.projectRoot = $projectRoot,
    p.sandboxBranch = $branch,
    p.sandboxWorktreePath = $path,
    p.sandboxStatus = 'IMPLEMENTING'
