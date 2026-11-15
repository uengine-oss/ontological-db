MATCH (bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)-[:HAS_COMMAND]->(cmd:Command)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (us:UserStory)-[:IMPLEMENTS]->(bc)

WITH bc, 
     collect(DISTINCT {
         id: evt.id,
         name: evt.name,
         displayName: evt.displayName,
         commandId: cmd.id,
         commandName: cmd.name,
         aggregateId: agg.id,
         aggregateName: agg.name,
         actor: cmd.actor,
         version: evt.version
     }) as eventsRaw,
     collect(DISTINCT cmd.actor) as actorsRaw,
     collect(DISTINCT us.role) as userRoles

// Filter out null events
WITH bc, 
     [e IN eventsRaw WHERE e.id IS NOT NULL] as events,
     [a IN actorsRaw WHERE a IS NOT NULL] as actors,
     [r IN userRoles WHERE r IS NOT NULL] as roles

// Combine actors from commands and user story roles
WITH bc, events, 
     [x IN (actors + roles) WHERE x IS NOT NULL | x] as allActors

RETURN {
    bcId: bc.id,
    bcName: bc.name,
    bcDisplayName: bc.displayName,
    bcDescription: bc.description,
    actors: allActors,
    events: events
} as swimlane
ORDER BY bc.name
