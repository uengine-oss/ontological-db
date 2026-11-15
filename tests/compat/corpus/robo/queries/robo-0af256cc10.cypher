MATCH (p:Proposal) WHERE p.projectRoot IS NOT NULL RETURN DISTINCT p.projectRoot AS root
