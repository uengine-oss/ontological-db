MATCH (p:Proposal {id: $id}) RETURN p.strategicDiff AS sd, p.projectRoot AS projectRoot, p.implementationPlan AS plan, p.planDraft AS planDraft, p.tacticalDiff AS td, p.decompositionMode AS mode
