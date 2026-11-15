MATCH (n:`ErpSupplier`) WHERE toString(n.`name`) = $v OR toLower(trim(toString(n.`name`))) = $lv RETURN n LIMIT $limit
