MATCH (n:Aggregate) WHERE n.id = $id SET n.valueObjects = $vo, n.enumerations = $enums, n.invariants = $inv
