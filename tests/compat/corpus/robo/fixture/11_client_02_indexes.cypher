// Copied from the client: robo-architect-lite:docs/cypher/schema/02_indexes.cypher (statements unchanged)

// One change: randomUUID() is replaced by a fixed UUID (uuid5 of file, statement, occurrence) so the
// fixture is the same at every run; the clients' queries keep their randomUUID().

CREATE TEXT INDEX index_bc_name IF NOT EXISTS
FOR (bc:BoundedContext)
ON (bc.name);

CREATE TEXT INDEX index_agg_name IF NOT EXISTS
FOR (agg:Aggregate)
ON (agg.name);

CREATE TEXT INDEX index_cmd_name IF NOT EXISTS
FOR (cmd:Command)
ON (cmd.name);

CREATE TEXT INDEX index_evt_name IF NOT EXISTS
FOR (evt:Event)
ON (evt.name);

CREATE TEXT INDEX index_policy_name IF NOT EXISTS
FOR (pol:Policy)
ON (pol.name);

CREATE INDEX index_userstory_role IF NOT EXISTS
FOR (us:UserStory)
ON (us.role);

CREATE INDEX index_userstory_priority IF NOT EXISTS
FOR (us:UserStory)
ON (us.priority);

CREATE INDEX index_userstory_status IF NOT EXISTS
FOR (us:UserStory)
ON (us.status);

CREATE INDEX index_evt_name_version IF NOT EXISTS
FOR (evt:Event)
ON (evt.name, evt.version);

CREATE INDEX index_evt_version IF NOT EXISTS
FOR (evt:Event)
ON (evt.version);

CREATE INDEX index_evt_breaking IF NOT EXISTS
FOR (evt:Event)
ON (evt.isBreaking);

CREATE FULLTEXT INDEX fulltext_userstory IF NOT EXISTS
FOR (us:UserStory)
ON EACH [us.role, us.action, us.benefit];

CREATE FULLTEXT INDEX fulltext_requirement IF NOT EXISTS
FOR (r:Requirement)
ON EACH [r.title, r.description];

CREATE TEXT INDEX index_ui_name IF NOT EXISTS
FOR (ui:UI)
ON (ui.name);

CREATE INDEX index_ui_attached IF NOT EXISTS
FOR (ui:UI)
ON (ui.attachedToId);

CREATE INDEX index_property_parent IF NOT EXISTS
FOR (p:Property)
ON (p.parentType, p.parentId);

CREATE TEXT INDEX index_property_name IF NOT EXISTS
FOR (p:Property)
ON (p.name);

CREATE INDEX journey_bc_idx IF NOT EXISTS
FOR (j:Journey)
ON (j.boundedContextId);

CREATE INDEX journey_slug_idx IF NOT EXISTS
FOR (j:Journey)
ON (j.journeyId);

CREATE INDEX feature_bc_idx IF NOT EXISTS
FOR (f:Feature)
ON (f.boundedContextId);

CREATE TEXT INDEX feature_name_idx IF NOT EXISTS
FOR (f:Feature)
ON (f.name);

CREATE INDEX invariant_aggregate_idx IF NOT EXISTS
FOR (inv:Invariant)
ON (inv.aggregateId);

CREATE TEXT INDEX invariant_declaration_idx IF NOT EXISTS
FOR (inv:Invariant)
ON (inv.declaration);

