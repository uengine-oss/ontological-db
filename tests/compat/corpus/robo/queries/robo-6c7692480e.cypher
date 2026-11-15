MATCH (p:Proposal {id: $id}) RETURN p.status AS status, p.projectRoot AS projectRoot
