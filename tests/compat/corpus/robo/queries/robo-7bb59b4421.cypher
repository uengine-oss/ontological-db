MATCH (center {id: $node_id})
OPTIONAL MATCH p=(center)-[r:HAS_AGGREGATE|HAS_COMMAND|EMITS|TRIGGERS*1..2]-(n)
WITH center, [p in collect(p) WHERE p IS NOT NULL] as ps
WITH center,
     CASE
        WHEN size(ps) = 0 THEN [center]
        ELSE reduce(allNodes = [], p in ps | allNodes + nodes(p))
     END as node_list,
     CASE
        WHEN size(ps) = 0 THEN []
        ELSE reduce(allRels = [], p in ps | allRels + relationships(p))
     END as rel_list

UNWIND node_list as nd
WITH collect(DISTINCT nd) as nodes, rel_list

UNWIND (CASE WHEN size(rel_list) = 0 THEN [null] ELSE rel_list END) as rl
WITH nodes, collect(DISTINCT rl) as rels
WITH nodes, [r IN rels WHERE r IS NOT NULL] as rels

RETURN
  [n in nodes | {
    id: n.id,
    type: labels(n)[0],
    name: coalesce(n.name, ''),
    description: coalesce(n.description, ''),
    properties: properties(n)
  }] as nodes,
  [r in rels | {
    source: startNode(r).id,
    target: endNode(r).id,
    type: type(r),
    properties: properties(r)
  }] as relationships
