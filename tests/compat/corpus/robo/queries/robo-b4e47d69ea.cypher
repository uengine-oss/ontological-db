MATCH (us:UserStory {id: $user_story_id})

// Path 1: Direct IMPLEMENTS relationships
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(directTarget)
WHERE directTarget:Aggregate OR directTarget:Command OR directTarget:Event OR directTarget:BoundedContext

// Path 2: Through BoundedContext - find the BC this user story belongs to
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(bc:BoundedContext)
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(bcAgg:Aggregate)
OPTIONAL MATCH (bcAgg)-[:HAS_COMMAND]->(bcCmd:Command)
OPTIONAL MATCH (bcCmd)-[:EMITS]->(bcEvt:Event)

// Path 3: If user story implements an aggregate, get its commands and events
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(usAgg:Aggregate)
OPTIONAL MATCH (usAgg)-[:HAS_COMMAND]->(usAggCmd:Command)
OPTIONAL MATCH (usAggCmd)-[:EMITS]->(usAggEvt:Event)

// Path 4: If user story implements a command, get its events
OPTIONAL MATCH (us)-[:IMPLEMENTS]->(usCmd:Command)
OPTIONAL MATCH (usCmd)-[:EMITS]->(usCmdEvt:Event)
OPTIONAL MATCH (usAggParent:Aggregate)-[:HAS_COMMAND]->(usCmd)

WITH us,
     collect(DISTINCT bc) as bcs,
     collect(DISTINCT bcAgg) + collect(DISTINCT usAgg) + collect(DISTINCT usAggParent) as allAggs,
     collect(DISTINCT bcCmd) + collect(DISTINCT usAggCmd) + collect(DISTINCT usCmd) as allCmds,
     collect(DISTINCT bcEvt) + collect(DISTINCT usAggEvt) + collect(DISTINCT usCmdEvt) as allEvts

// Get the first BC (user story typically belongs to one BC)
WITH us,
     CASE WHEN size(bcs) > 0 THEN bcs[0] ELSE null END as bc,
     allAggs, allCmds, allEvts

RETURN {
    id: us.id,
    role: us.role,
    action: us.action,
    benefit: us.benefit,
    priority: us.priority,
    status: us.status
} as userStory,
bc {.id, .name, .description} as boundedContext,
[a IN allAggs WHERE a IS NOT NULL | a {.id, .name, .rootEntity, type: 'Aggregate'}] as aggregates,
[c IN allCmds WHERE c IS NOT NULL | c {.id, .name, .actor, type: 'Command'}] as commands,
[e IN allEvts WHERE e IS NOT NULL | e {.id, .name, .version, type: 'Event'}] as events
