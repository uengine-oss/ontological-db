MATCH (us:UserStory {id: $id})
RETURN us.role AS role,
       us.action AS action,
       us.benefit AS benefit,
       us.priority AS priority,
       us.status AS status,
       coalesce(us.acceptanceCriteria, []) AS acceptanceCriteria,
       us.updatedAt AS updatedAt
