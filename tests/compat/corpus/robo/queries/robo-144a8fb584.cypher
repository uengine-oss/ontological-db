MATCH (bc:BoundedContext {id: $bc_id})
WHERE ($session_id IS NULL OR coalesce(bc.session_id, '') = $session_id)

// Aggregate-level rollup
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(agg:Aggregate)
OPTIONAL MATCH (us:UserStory)-[:IMPLEMENTS]->(agg)
OPTIONAL MATCH (us)-[:SOURCED_FROM]->(sr:Rule)
WITH bc, agg,
     collect(DISTINCT CASE WHEN sr IS NOT NULL THEN
                          { rule_id: sr.id, title: sr.title,
                            source_function: sr.source_function,
                            source_rule_id: sr.source_rule_id,
                            given: coalesce(sr.given, ''),
                            when_: coalesce(sr.`when`, ''),
                            then: coalesce(sr.`then`, ''),
                            write_effects_json: coalesce(sr.write_effects_json, '[]'),
                            via_us: us.id }
                          ELSE NULL END) AS aggRules
WITH bc, collect({ id: agg.id, rules: [r IN aggRules WHERE r IS NOT NULL] }) AS aggRollup

// Command-level rollup
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(cmd:Command)
OPTIONAL MATCH (us2:UserStory)-[:IMPLEMENTS]->(cmd)
OPTIONAL MATCH (us2)-[:SOURCED_FROM]->(sr2:Rule)
WITH bc, aggRollup, cmd,
     collect(DISTINCT CASE WHEN sr2 IS NOT NULL THEN
                          { rule_id: sr2.id, title: sr2.title,
                            source_function: sr2.source_function,
                            source_rule_id: sr2.source_rule_id,
                            given: coalesce(sr2.given, ''),
                            when_: coalesce(sr2.`when`, ''),
                            then: coalesce(sr2.`then`, ''),
                            write_effects_json: coalesce(sr2.write_effects_json, '[]'),
                            via_us: us2.id }
                          ELSE NULL END) AS cmdRules
WITH bc, aggRollup,
     collect({ id: cmd.id, rules: [r IN cmdRules WHERE r IS NOT NULL] }) AS cmdRollup

// Event-level rollup — via Command's IMPLEMENTING US's
OPTIONAL MATCH (bc)-[:HAS_AGGREGATE]->(:Aggregate)-[:HAS_COMMAND]->(cmd3:Command)-[:EMITS]->(evt:Event)
OPTIONAL MATCH (us3:UserStory)-[:IMPLEMENTS]->(cmd3)
OPTIONAL MATCH (us3)-[:SOURCED_FROM]->(sr3:Rule)
WITH bc, aggRollup, cmdRollup, evt,
     collect(DISTINCT CASE WHEN sr3 IS NOT NULL THEN
                          { rule_id: sr3.id, title: sr3.title,
                            source_function: sr3.source_function,
                            source_rule_id: sr3.source_rule_id,
                            given: coalesce(sr3.given, ''),
                            when_: coalesce(sr3.`when`, ''),
                            then: coalesce(sr3.`then`, ''),
                            write_effects_json: coalesce(sr3.write_effects_json, '[]'),
                            via_us: us3.id }
                          ELSE NULL END) AS evtRules
WITH aggRollup, cmdRollup,
     collect({ id: evt.id,
               rules: [r IN evtRules WHERE r IS NOT NULL] }) AS evtRollup

RETURN aggRollup, cmdRollup, evtRollup
