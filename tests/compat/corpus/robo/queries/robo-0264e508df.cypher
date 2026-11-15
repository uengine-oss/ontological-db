MATCH (b:FigmaBinding {id: $id})-[:MAPS_STORYBOARD]->(m:StoryboardPageMapping)
RETURN m.status AS status, count(m) AS n
