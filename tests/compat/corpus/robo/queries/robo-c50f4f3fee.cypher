MERGE (p:Policy {key: $key, session_id: $sid}) ON CREATE SET p.id = randomUUID() SET p.name = $name, p.displayName = $name, p.description = $desc,     p.kind = 'cross_bc', p.bc_id = $bcid
