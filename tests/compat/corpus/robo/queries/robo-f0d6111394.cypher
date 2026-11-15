MATCH (p:Proposal {id:$id}) RETURN p.decompositionMode AS mode, p.odaConformance AS conf
