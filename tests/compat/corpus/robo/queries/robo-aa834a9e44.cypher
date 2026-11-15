MATCH (bc:BoundedContext)
WHERE NOT EXISTS {
    MATCH (bc)-[:HAS_EVENT]->(e:Event)-[:TRIGGERS]->(:Policy)
}
AND EXISTS {
    MATCH (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)
}
RETURN bc.name AS bcName, bc.id AS bcId
