MATCH (b:FigmaBinding {id: $id})-[:MAPS_STORYBOARD]->(m:StoryboardPageMapping)
RETURN m ORDER BY m.figmaPageName
