MATCH (p:Proposal {id: $id}) RETURN p.originalPrompt AS prompt, p.strategicDiff AS sd
