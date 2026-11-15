MATCH (p:Proposal {id:$id}) SET p.stageArtifacts=$arts, p.stageDraftArtifacts=$draftArts, p.currentStage=$cur
