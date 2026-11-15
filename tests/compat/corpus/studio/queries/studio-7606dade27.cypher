MATCH (m:_OntologyModel {schema_id: $schema_id}) OPTIONAL MATCH (m)-[*1..3]->(child) WHERE any(l IN labels(child) WHERE l STARTS WITH '_Ontology') DETACH DELETE child, m
