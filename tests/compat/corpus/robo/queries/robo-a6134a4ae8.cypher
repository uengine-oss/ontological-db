UNWIND $node_ids as node_id
MATCH (n {id: node_id})
OPTIONAL MATCH (bc:BoundedContext)-[:HAS_AGGREGATE|HAS_POLICY*1..2]->(n)
OPTIONAL MATCH (bc2:BoundedContext)-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(n)
OPTIONAL MATCH (bc3:BoundedContext)-[:HAS_AGGREGATE]->(agg2:Aggregate)-[:HAS_COMMAND]->(cmd:Command)-[:EMITS]->(n)
OPTIONAL MATCH (bc4:BoundedContext)-[:HAS_UI]->(n)
OPTIONAL MATCH (bc5:BoundedContext)-[:HAS_READMODEL]->(n)
WITH n, coalesce(bc, bc2, bc3, bc4, bc5) as context
RETURN collect({
    nodeId: n.id,
    bcId: context.id,
    bcName: context.name
}) as results
