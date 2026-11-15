MATCH (n:Aggregate {id: $id})
RETURN n.valueObjects    AS valueObjects,
       n.enumerations    AS enumerations,
       n.invariants      AS invariants,
       n.fieldDescriptors AS fieldDescriptors
