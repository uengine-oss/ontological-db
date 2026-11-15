MATCH (us:UserStory {id: $id})
RETURN us.id AS id, us.role AS role, us.action AS action,
       coalesce(us.id, us.sourceUnitId) AS src
