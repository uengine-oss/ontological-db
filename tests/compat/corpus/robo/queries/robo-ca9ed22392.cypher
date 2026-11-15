MATCH (us:UserStory)-[:IMPLEMENTS]->(d)
WHERE us.id IN $ids AND d.id IS NOT NULL
WITH DISTINCT d
WHERE NOT EXISTS {
    MATCH (o:UserStory)-[:IMPLEMENTS]->(d)
    WHERE NOT o.id IN $ids
}
RETURN collect(d.id) AS ids
