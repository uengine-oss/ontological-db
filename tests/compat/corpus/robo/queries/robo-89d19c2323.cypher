MATCH (s)
WHERE s.id IN $seed_ids
WITH collect(DISTINCT s) as seeds
UNWIND seeds as seed
OPTIONAL MATCH p=(seed)-[r:HAS_AGGREGATE|HAS_COMMAND|EMITS|TRIGGERS*1..2]-(n)
WITH seeds, collect(p) as ps
WITH seeds,
     CASE
        WHEN size(ps) = 0 THEN seeds
        ELSE reduce(allNodes = [], p in ps | allNodes + nodes(p)) + seeds
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
    attachedToId: n.attachedToId,
    attachedToType: n.attachedToType,
    attachedToName: n.attachedToName,
    parentType: n.parentType,
    parentId: n.parentId,
    isKey: n.isKey,
    isForeignKey: n.isForeignKey,
    isRequired: n.isRequired
  }] as nodes,
  [r in rels | {
    source: startNode(r).id,
    target: endNode(r).id,
    type: type(r)
  }] as relationships
