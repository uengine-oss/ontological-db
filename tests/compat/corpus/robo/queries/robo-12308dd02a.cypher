CREATE (cs:ChangeSet {
    id: $id,
    title: $title,
    author: $author,
    createdAt: datetime($createdAt),
    status: 'DRAFT'
})
WITH cs
UNWIND $changeIds AS cid
MATCH (chg:RequirementChange {id: cid})
CREATE (cs)-[:CONTAINS]->(chg)
SET chg.changeSetId = cs.id
RETURN cs {.*} AS cs
