MATCH (p:Proposal {id: $id})
RETURN p.title AS title, p.originalPrompt AS prompt,
       p.strategicDiff AS strategicDiff, p.tacticalDiff AS tacticalDiff,
       p.implementationPlan AS implementationPlan, p.projectRoot AS projectRoot
