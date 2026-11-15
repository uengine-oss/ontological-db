MATCH (p:Proposal {id: $id}) RETURN p.tacticalDiff AS td, p.planDraft AS draft
