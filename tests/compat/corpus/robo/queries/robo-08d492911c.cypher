MATCH (p:Proposal {id: $id})
RETURN p.originalPrompt AS prompt,
       p.title AS title,
       p.strategicDiff AS strategicDiff,
       p.tacticalDiff AS tacticalDiff,
       p.implementationPlan AS implementationPlan,
       p.projectRoot AS projectRoot
