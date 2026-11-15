MATCH (p:Proposal {id:$id}) SET p.stagePlan=$plan, p.currentStage=$cur
