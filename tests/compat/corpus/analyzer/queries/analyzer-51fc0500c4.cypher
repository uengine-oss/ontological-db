MATCH (x {id:$id, graph_owner:$graph_owner}) OPTIONAL MATCH (x)-[r]->(t:TABLE {graph_owner:$graph_owner}) WHERE type(r) IN $rels RETURN elementId(x) AS eid, collect({rel:type(r), t:t.name}) AS tb
