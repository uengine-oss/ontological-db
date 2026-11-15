MATCH (us:UserStory)
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(target)
WITH us, collect(DISTINCT {type: labels(target)[0], name: target.name, id: target.id}) as implemented_in
RETURN {
    id: us.id,
    role: us.role,
    action: us.action,
    benefit: us.benefit,
    priority: us.priority,
    status: us.status,
    uiDescription: us.uiDescription,
    displayName: us.displayName,
    sourceScreenName: us.sourceScreenName,
    implemented_in: implemented_in
} as user_story
ORDER BY us.id
