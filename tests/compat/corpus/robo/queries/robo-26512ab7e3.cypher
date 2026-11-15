MATCH (b:FigmaBinding {id: $bid})
MERGE (m:StoryboardPageMapping {commandId: $cid})
ON CREATE SET m.id = $newid,
              m.lastRenameAt = null
SET m.figmaPageId = $page_id,
    m.figmaPageName = $page_name,
    m.status = 'active'
MERGE (b)-[:MAPS_STORYBOARD]->(m)
WITH m
OPTIONAL MATCH (c:Command {id: $cid})
FOREACH (cc IN CASE WHEN c IS NULL THEN [] ELSE [c] END |
    MERGE (m)-[:MAPS]->(cc)
)
RETURN m
