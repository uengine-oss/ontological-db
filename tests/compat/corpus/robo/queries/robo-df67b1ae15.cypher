MATCH (p:Proposal {id: $pid})-[e:EFFECT]->() DELETE e
