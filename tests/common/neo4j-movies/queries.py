"""The Neo4j Movie Graph sample application, verbatim.

Every query below is copied from the official guide that ships with the sample
(`neo4j-graph-examples/movies`, `documentation/movies.adoc` — the content behind
`:play movies` in Neo4j Browser). They are the queries a user of that sample
actually runs, in the order the guide runs them, and they are not rewritten for
this database: rewriting them would be answering a different question.

Two guide queries are parameterised on the reader ("<Your Name>") and one
depends on the node the previous step created; those keep the guide's own
placeholder resolved to a fixed name so the run is reproducible.
"""

# (id, title, cypher, kind)
#   kind: "read"  — must return rows
#         "write" — mutates; run once, after the read set
QUERIES = [
    ("q01", "Movies released after 2000, limited",
     "MATCH (m:Movie) WHERE m.released > 2000 RETURN m LIMIT 5", "read"),
    ("q02", "Movies released after 2005",
     "MATCH (m:Movie) WHERE m.released > 2005 RETURN m", "read"),
    ("q03", "Count of movies after 2005",
     "MATCH (m:Movie) WHERE m.released > 2005 RETURN count(m)", "read"),
    ("q04", "Directors of movies after 2010",
     "MATCH (p:Person)-[d:DIRECTED]-(m:Movie) WHERE m.released > 2010 RETURN p,d,m", "read"),
    ("q05", "Actors in movies after 2010",
     "MATCH (p:Person)-[r:ACTED_IN]-(m:Movie) WHERE m.released > 2010 RETURN p,r,m", "read"),
    ("q06", "Twenty people",
     "MATCH (p:Person) RETURN p LIMIT 20", "read"),
    ("q07", "Twenty nodes of any label",
     "MATCH (n) RETURN n LIMIT 20", "read"),
    ("q08", "Movie titles and release years",
     "MATCH (m:Movie) RETURN m.title, m.released", "read"),
    ("q09", "Person names and birth years",
     "MATCH (p:Person) RETURN p.name, p.born", "read"),
    ("q10", "Find a person by property in the pattern",
     "MATCH (p:Person {name: 'Tom Hanks'}) RETURN p", "read"),
    ("q11", "Find the same person through WHERE",
     'MATCH (p:Person) WHERE p.name = "Tom Hanks" RETURN p', "read"),
    ("q12", "Find a movie by title",
     'MATCH (m:Movie {title: "Cloud Atlas"}) RETURN m', "read"),
    ("q13", "Movies in a release-year range",
     "MATCH (m:Movie) WHERE m.released > 2010 AND m.released < 2015 RETURN m", "read"),
    ("q14", "Every ACTED_IN relationship, directed",
     "MATCH (p:Person)-[r:ACTED_IN]->(m:Movie) RETURN p,r,m", "read"),
    ("q15", "Every ACTED_IN relationship, undirected",
     "MATCH (p:Person)-[r:ACTED_IN]-(m:Movie) RETURN p,r,m", "read"),
    ("q16", "Reviews",
     "MATCH (p:Person)-[r:REVIEWED]-(m:Movie) RETURN p,r,m", "read"),
    ("q17", "Who directed Cloud Atlas",
     "MATCH (m:Movie {title: 'Cloud Atlas'})<-[d:DIRECTED]-(p:Person) RETURN p.name", "read"),
    ("q18", "Tom Hanks' co-actors",
     'MATCH (tom:Person {name: "Tom Hanks"})-[:ACTED_IN]->(:Movie)<-[:ACTED_IN]-(p:Person) '
     "RETURN p.name", "read"),
    ("q19", "How people relate to Cloud Atlas",
     'MATCH (p:Person)-[relatedTo]-(m:Movie {title: "Cloud Atlas"}) '
     "RETURN p.name, type(relatedTo)", "read"),
    ("q20", "Bacon path — everything within three hops",
     "MATCH (p:Person {name: 'Kevin Bacon'})-[*1..3]-(hollywood) RETURN DISTINCT p, hollywood",
     "read"),
    ("q21", "Create a person",
     "CREATE (p:Person {name: 'John Doe'}) RETURN p", "write"),
    ("q22", "MERGE a person with ON CREATE / ON MATCH",
     "MERGE (p:Person {name: 'John Doe'}) "
     "ON CREATE SET p.createdAt = timestamp() "
     "ON MATCH SET p.lastLoggedInAt = timestamp() "
     "RETURN p", "write"),
    ("q23", "MERGE a movie with ON CREATE / ON MATCH",
     "MERGE (m:Movie {title: 'Greyhound'}) "
     'ON CREATE SET m.released = "2020", m.lastUpdatedAt = timestamp() '
     "ON MATCH SET m.lastUpdatedAt = timestamp() "
     "RETURN m", "write"),
    ("q24", "Create a WATCHED relationship between two matched nodes",
     'MATCH (p:Person), (m:Movie) WHERE p.name = "Tom Hanks" AND m.title = "Cloud Atlas" '
     "CREATE (p)-[w:WATCHED]->(m) RETURN type(w)", "write"),
]
