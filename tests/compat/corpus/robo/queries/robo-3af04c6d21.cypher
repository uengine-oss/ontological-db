MATCH (p:Proposal {id:$id}) SET p.tacticalDiff=$td, p.odaArtifacts=$arts, p.odaConformance=$conf
