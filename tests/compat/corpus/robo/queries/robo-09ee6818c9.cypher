MATCH (fk:Property {proposalSource: $pid}) WHERE fk.isForeignKey = true AND fk.fkTargetHint IS NOT NULL RETURN fk.id AS id, fk.fkTargetHint AS hint
