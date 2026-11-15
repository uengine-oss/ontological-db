MATCH (n:`ErpSupplier`) WHERE toLower(toString(n.`name`)) CONTAINS $t0 RETURN n LIMIT 15
