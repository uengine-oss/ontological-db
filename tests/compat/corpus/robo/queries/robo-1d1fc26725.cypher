MATCH (startBc:BoundedContext {id: $bc_id})

// Find all BCs connected via outbound flow: Event → TRIGGERS → Policy → INVOKES → Command → EMITS → Event
OPTIONAL MATCH (startBc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(:Command)-[:EMITS]->(sourceEvt:Event)
       -[:TRIGGERS]->(:Policy)<-[:HAS_POLICY]-(targetBc:BoundedContext)

// Collect all connected BC IDs (including start BC)
WITH startBc.id as startBcId, collect(DISTINCT targetBc.id) as targetBcIds

// Combine start BC with target BCs
WITH targetBcIds + [startBcId] as connectedBcIds

// Unwind and return
UNWIND connectedBcIds as bcId
WITH bcId
WHERE bcId IS NOT NULL
RETURN DISTINCT bcId
