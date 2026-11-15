MATCH (n0:`Customer`) WHERE elementId(n0) = $e0 MATCH (n1:`Supplier`) WHERE elementId(n1) = $e1 DELETE n0, n1
