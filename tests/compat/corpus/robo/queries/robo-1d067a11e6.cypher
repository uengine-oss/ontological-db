UNWIND $node_ids as node_id
MATCH (n {id: node_id})
WITH n, labels(n)[0] as nodeType, node_id

// Find parent BC based on known containment patterns
OPTIONAL MATCH (bc1:BoundedContext {id: node_id})
OPTIONAL MATCH (bc2:BoundedContext)-[:HAS_AGGREGATE]->(n)
OPTIONAL MATCH (bc3:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(n)
OPTIONAL MATCH (bc4:BoundedContext)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(n)
OPTIONAL MATCH (bc5:BoundedContext)-[:HAS_POLICY]->(n)

WITH n, nodeType, coalesce(bc1, bc2, bc3, bc4, bc5) as bc
RETURN collect({
    nodeId: n.id,
    nodeType: nodeType,
    bcId: bc.id,
    bcName: bc.name
}) as results
