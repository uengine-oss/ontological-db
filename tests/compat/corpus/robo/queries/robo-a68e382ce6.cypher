MATCH (us:UserStory {id: $id}) RETURN coalesce(us.action, us.id) AS name
