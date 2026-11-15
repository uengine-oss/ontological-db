MATCH (src:Property)
WHERE coalesce(src.isForeignKey, false) = true
  AND src.fkTargetHint IS NOT NULL
  AND trim(toString(src.fkTargetHint)) <> ''
RETURN src.id as id, src.fkTargetHint as fkTargetHint
ORDER BY src.id
