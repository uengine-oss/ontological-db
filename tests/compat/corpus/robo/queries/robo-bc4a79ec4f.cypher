MATCH (p:Proposal {id:$id}) SET p.decompositionMode=$mode, p.currentStage='SCOPE'
