MATCH (d {id: $did})
CREATE (ui:UI {
    id: $id, name: $name, description: $desc, template: $tpl,
    attachedToId: $did, attachedToType: $dtype, attachedToName: $dname,
    userStoryId: $usid, designSource: 'html',
    proposalSource: $pid, createdAt: datetime(), updatedAt: datetime()
})
MERGE (ui)-[:ATTACHED_TO]->(d)
