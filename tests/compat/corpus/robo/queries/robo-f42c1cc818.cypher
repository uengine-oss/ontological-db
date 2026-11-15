MATCH (p:Proposal {id:$id}) SET p.stagePlan=$plan, p.stageDraftArtifacts=$draftArts, p.currentStage=$cur
