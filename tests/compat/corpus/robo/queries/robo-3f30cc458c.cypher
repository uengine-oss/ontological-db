MATCH (c {proposalSource: $pid})
WHERE c:Property OR c:Given OR c:When OR c:Then OR c:UI OR c:Invariant
   OR c:Journey OR c:JourneyStep
DETACH DELETE c
