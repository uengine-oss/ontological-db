MATCH (us:UserStory {id: $id}) RETURN us.role AS role, us.action AS action
