MATCH (b:FigmaBinding {id: $id})-[:MAPS_STORYBOARD]->(m:StoryboardPageMapping)
WHERE m.status = $status
RETURN m ORDER BY m.figmaPageName
