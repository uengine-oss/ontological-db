MATCH (us:UserStory {id: $id})
SET us.role = $role,
    us.action = $action,
    us.benefit = $benefit,
    us.priority = $priority,
    us.status = $status,
    us.acceptanceCriteria = $acceptanceCriteria,
    us.criteriaUserEdited = CASE
        WHEN size($acceptanceCriteria) > 0 OR coalesce(us.criteriaUserEdited, false)
        THEN true ELSE false END,
    us.criteriaEditedAt = CASE
        WHEN size($acceptanceCriteria) > 0 THEN datetime() ELSE us.criteriaEditedAt END,
    us.updatedAt = datetime()
WITH us
CREATE (h:EditHistory {
    id: randomUUID(),
    timestamp: datetime(),
    userName: $user_name,
    userEmail: $user_email,
    changes: $changes_json
})
CREATE (us)-[:HAS_HISTORY]->(h)
RETURN us.updatedAt AS updatedAt
