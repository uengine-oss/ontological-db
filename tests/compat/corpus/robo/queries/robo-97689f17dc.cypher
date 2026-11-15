MATCH (us:UserStory {id: $user_story_id})
SET us.role = CASE WHEN $role IS NOT NULL THEN $role ELSE us.role END,
    us.action = CASE WHEN $action IS NOT NULL THEN $action ELSE us.action END,
    us.benefit = CASE WHEN $benefit IS NOT NULL THEN $benefit ELSE us.benefit END,
    us.priority = CASE WHEN $priority IS NOT NULL THEN $priority ELSE us.priority END,
    us.status = CASE WHEN $status IS NOT NULL THEN $status ELSE us.status END,
    us.acceptanceCriteria = CASE WHEN $criteria_present THEN $criteria ELSE us.acceptanceCriteria END,
    us.criteriaUserEdited = CASE WHEN $criteria_present THEN true ELSE coalesce(us.criteriaUserEdited, false) END,
    us.criteriaEditedAt = CASE WHEN $criteria_present THEN datetime() ELSE us.criteriaEditedAt END,
    us.updatedAt = datetime()
RETURN us {.id, .name, .role, .action, .benefit, .priority, .status, .sequence,
           uiDescription: us.uiDescription, displayName: us.displayName,
           sourceScreenName: us.sourceScreenName, sourceUnitId: us.sourceUnitId,
           acceptanceCriteria: coalesce(us.acceptanceCriteria, []),
           criteriaUserEdited: coalesce(us.criteriaUserEdited, false),
           criteriaEditedAt: us.criteriaEditedAt} as user_story
