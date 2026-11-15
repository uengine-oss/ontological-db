// Copied from the client: robo-architect-lite:docs/cypher/schema/01_constraints.cypher (statements unchanged)

// One change: randomUUID() is replaced by a fixed UUID (uuid5 of file, statement, occurrence) so the
// fixture is the same at every run; the clients' queries keep their randomUUID().

CREATE CONSTRAINT constraint_requirement_id IF NOT EXISTS
FOR (r:Requirement)
REQUIRE r.id IS UNIQUE;

CREATE CONSTRAINT constraint_userstory_id IF NOT EXISTS
FOR (us:UserStory)
REQUIRE us.id IS UNIQUE;

CREATE CONSTRAINT constraint_boundedcontext_id IF NOT EXISTS
FOR (bc:BoundedContext)
REQUIRE bc.id IS UNIQUE;

CREATE CONSTRAINT constraint_boundedcontext_key IF NOT EXISTS
FOR (bc:BoundedContext)
REQUIRE bc.key IS UNIQUE;

CREATE CONSTRAINT constraint_boundedcontext_key_exists IF NOT EXISTS
FOR (bc:BoundedContext)
REQUIRE bc.key IS NOT NULL;

CREATE CONSTRAINT constraint_aggregate_id IF NOT EXISTS
FOR (agg:Aggregate)
REQUIRE agg.id IS UNIQUE;

CREATE CONSTRAINT constraint_aggregate_key IF NOT EXISTS
FOR (agg:Aggregate)
REQUIRE agg.key IS UNIQUE;

CREATE CONSTRAINT constraint_aggregate_key_exists IF NOT EXISTS
FOR (agg:Aggregate)
REQUIRE agg.key IS NOT NULL;

CREATE CONSTRAINT constraint_command_id IF NOT EXISTS
FOR (cmd:Command)
REQUIRE cmd.id IS UNIQUE;

CREATE CONSTRAINT constraint_command_key IF NOT EXISTS
FOR (cmd:Command)
REQUIRE cmd.key IS UNIQUE;

CREATE CONSTRAINT constraint_command_key_exists IF NOT EXISTS
FOR (cmd:Command)
REQUIRE cmd.key IS NOT NULL;

CREATE CONSTRAINT constraint_event_id IF NOT EXISTS
FOR (evt:Event)
REQUIRE evt.id IS UNIQUE;

CREATE CONSTRAINT constraint_event_key IF NOT EXISTS
FOR (evt:Event)
REQUIRE evt.key IS UNIQUE;

CREATE CONSTRAINT constraint_event_key_exists IF NOT EXISTS
FOR (evt:Event)
REQUIRE evt.key IS NOT NULL;

CREATE CONSTRAINT constraint_policy_id IF NOT EXISTS
FOR (pol:Policy)
REQUIRE pol.id IS UNIQUE;

CREATE CONSTRAINT constraint_policy_key IF NOT EXISTS
FOR (pol:Policy)
REQUIRE pol.key IS UNIQUE;

CREATE CONSTRAINT constraint_policy_key_exists IF NOT EXISTS
FOR (pol:Policy)
REQUIRE pol.key IS NOT NULL;

CREATE CONSTRAINT constraint_readmodel_id IF NOT EXISTS
FOR (rm:ReadModel)
REQUIRE rm.id IS UNIQUE;

CREATE CONSTRAINT constraint_readmodel_key IF NOT EXISTS
FOR (rm:ReadModel)
REQUIRE rm.key IS UNIQUE;

CREATE CONSTRAINT constraint_readmodel_key_exists IF NOT EXISTS
FOR (rm:ReadModel)
REQUIRE rm.key IS NOT NULL;

CREATE CONSTRAINT constraint_userstory_role IF NOT EXISTS
FOR (us:UserStory)
REQUIRE us.role IS NOT NULL;

CREATE CONSTRAINT constraint_userstory_action IF NOT EXISTS
FOR (us:UserStory)
REQUIRE us.action IS NOT NULL;

CREATE CONSTRAINT constraint_event_name IF NOT EXISTS
FOR (evt:Event)
REQUIRE evt.name IS NOT NULL;

CREATE CONSTRAINT constraint_event_version IF NOT EXISTS
FOR (evt:Event)
REQUIRE evt.version IS NOT NULL;

CREATE CONSTRAINT constraint_bc_name IF NOT EXISTS
FOR (bc:BoundedContext)
REQUIRE bc.name IS NOT NULL;

CREATE CONSTRAINT constraint_agg_name IF NOT EXISTS
FOR (agg:Aggregate)
REQUIRE agg.name IS NOT NULL;

CREATE CONSTRAINT constraint_cmd_name IF NOT EXISTS
FOR (cmd:Command)
REQUIRE cmd.name IS NOT NULL;

CREATE CONSTRAINT constraint_policy_name IF NOT EXISTS
FOR (pol:Policy)
REQUIRE pol.name IS NOT NULL;

CREATE CONSTRAINT constraint_readmodel_name IF NOT EXISTS
FOR (rm:ReadModel)
REQUIRE rm.name IS NOT NULL;

CREATE CONSTRAINT constraint_property_id IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.id IS UNIQUE;

CREATE CONSTRAINT constraint_property_parent_name_unique IF NOT EXISTS
FOR (prop:Property)
REQUIRE (prop.parentType, prop.parentId, prop.name) IS UNIQUE;

CREATE CONSTRAINT constraint_property_name IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.name IS NOT NULL;

CREATE CONSTRAINT constraint_property_type IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.type IS NOT NULL;

CREATE CONSTRAINT constraint_property_description IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.description IS NOT NULL;

CREATE CONSTRAINT constraint_property_isKey IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.isKey IS NOT NULL;

CREATE CONSTRAINT constraint_property_isForeignKey IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.isForeignKey IS NOT NULL;

CREATE CONSTRAINT constraint_property_isRequired IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.isRequired IS NOT NULL;

CREATE CONSTRAINT constraint_property_parentType IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.parentType IS NOT NULL;

CREATE CONSTRAINT constraint_property_parentId IF NOT EXISTS
FOR (prop:Property)
REQUIRE prop.parentId IS NOT NULL;

CREATE CONSTRAINT constraint_ui_id IF NOT EXISTS
FOR (ui:UI)
REQUIRE ui.id IS UNIQUE;

CREATE CONSTRAINT constraint_ui_key IF NOT EXISTS
FOR (ui:UI)
REQUIRE ui.key IS UNIQUE;

CREATE CONSTRAINT constraint_ui_key_exists IF NOT EXISTS
FOR (ui:UI)
REQUIRE ui.key IS NOT NULL;

CREATE CONSTRAINT constraint_ui_name IF NOT EXISTS
FOR (ui:UI)
REQUIRE ui.name IS NOT NULL;

CREATE CONSTRAINT constraint_given_id IF NOT EXISTS
FOR (g:Given)
REQUIRE g.id IS UNIQUE;

CREATE CONSTRAINT constraint_given_key IF NOT EXISTS
FOR (g:Given)
REQUIRE g.key IS UNIQUE;

CREATE CONSTRAINT constraint_given_key_exists IF NOT EXISTS
FOR (g:Given)
REQUIRE g.key IS NOT NULL;

CREATE CONSTRAINT constraint_given_name IF NOT EXISTS
FOR (g:Given)
REQUIRE g.name IS NOT NULL;

CREATE CONSTRAINT constraint_given_parentType IF NOT EXISTS
FOR (g:Given)
REQUIRE g.parentType IS NOT NULL;

CREATE CONSTRAINT constraint_given_parentId IF NOT EXISTS
FOR (g:Given)
REQUIRE g.parentId IS NOT NULL;

CREATE CONSTRAINT constraint_given_parent_unique IF NOT EXISTS
FOR (g:Given)
REQUIRE (g.parentType, g.parentId) IS UNIQUE;

CREATE CONSTRAINT constraint_when_id IF NOT EXISTS
FOR (w:When)
REQUIRE w.id IS UNIQUE;

CREATE CONSTRAINT constraint_when_key IF NOT EXISTS
FOR (w:When)
REQUIRE w.key IS UNIQUE;

CREATE CONSTRAINT constraint_when_key_exists IF NOT EXISTS
FOR (w:When)
REQUIRE w.key IS NOT NULL;

CREATE CONSTRAINT constraint_when_name IF NOT EXISTS
FOR (w:When)
REQUIRE w.name IS NOT NULL;

CREATE CONSTRAINT constraint_when_parentType IF NOT EXISTS
FOR (w:When)
REQUIRE w.parentType IS NOT NULL;

CREATE CONSTRAINT constraint_when_parentId IF NOT EXISTS
FOR (w:When)
REQUIRE w.parentId IS NOT NULL;

CREATE CONSTRAINT constraint_when_parent_unique IF NOT EXISTS
FOR (w:When)
REQUIRE (w.parentType, w.parentId) IS UNIQUE;

CREATE CONSTRAINT constraint_then_id IF NOT EXISTS
FOR (t:Then)
REQUIRE t.id IS UNIQUE;

CREATE CONSTRAINT constraint_then_key IF NOT EXISTS
FOR (t:Then)
REQUIRE t.key IS UNIQUE;

CREATE CONSTRAINT constraint_then_key_exists IF NOT EXISTS
FOR (t:Then)
REQUIRE t.key IS NOT NULL;

CREATE CONSTRAINT constraint_then_name IF NOT EXISTS
FOR (t:Then)
REQUIRE t.name IS NOT NULL;

CREATE CONSTRAINT constraint_then_parentType IF NOT EXISTS
FOR (t:Then)
REQUIRE t.parentType IS NOT NULL;

CREATE CONSTRAINT constraint_then_parentId IF NOT EXISTS
FOR (t:Then)
REQUIRE t.parentId IS NOT NULL;

CREATE CONSTRAINT figma_binding_singleton IF NOT EXISTS
FOR (b:FigmaBinding) REQUIRE b.id IS UNIQUE;

CREATE CONSTRAINT storyboard_page_mapping_id_unique IF NOT EXISTS
FOR (m:StoryboardPageMapping) REQUIRE m.id IS UNIQUE;

CREATE CONSTRAINT storyboard_page_mapping_command_unique IF NOT EXISTS
FOR (m:StoryboardPageMapping) REQUIRE m.commandId IS UNIQUE;

CREATE CONSTRAINT binding_history_event_id_unique IF NOT EXISTS
FOR (e:BindingHistoryEvent) REQUIRE e.id IS UNIQUE;

CREATE CONSTRAINT sync_run_id_unique IF NOT EXISTS
FOR (r:SyncRun) REQUIRE r.id IS UNIQUE;

CREATE CONSTRAINT journey_id_unique IF NOT EXISTS
FOR (j:Journey) REQUIRE j.id IS UNIQUE;

CREATE CONSTRAINT journey_key_unique IF NOT EXISTS
FOR (j:Journey) REQUIRE j.key IS UNIQUE;

CREATE CONSTRAINT journey_step_id_unique IF NOT EXISTS
FOR (s:JourneyStep) REQUIRE s.id IS UNIQUE;

CREATE CONSTRAINT journey_step_key_unique IF NOT EXISTS
FOR (s:JourneyStep) REQUIRE s.key IS UNIQUE;

CREATE INDEX sync_run_status_idx IF NOT EXISTS
FOR (r:SyncRun) ON (r.status);

CREATE INDEX sync_run_binding_file_key_idx IF NOT EXISTS
FOR (r:SyncRun) ON (r.bindingFileKey);

CREATE CONSTRAINT constraint_then_parent_unique IF NOT EXISTS
FOR (t:Then)
REQUIRE (t.parentType, t.parentId) IS UNIQUE;

CREATE CONSTRAINT feature_id_unique IF NOT EXISTS
FOR (f:Feature) REQUIRE f.id IS UNIQUE;

CREATE CONSTRAINT feature_key_unique IF NOT EXISTS
FOR (f:Feature) REQUIRE f.key IS UNIQUE;

CREATE CONSTRAINT invariant_id_unique IF NOT EXISTS
FOR (inv:Invariant) REQUIRE inv.id IS UNIQUE;

CREATE CONSTRAINT invariant_key_unique IF NOT EXISTS
FOR (inv:Invariant) REQUIRE inv.key IS UNIQUE;

CREATE CONSTRAINT invariant_key_exists IF NOT EXISTS
FOR (inv:Invariant) REQUIRE inv.key IS NOT NULL;

CREATE CONSTRAINT invariant_declaration_exists IF NOT EXISTS
FOR (inv:Invariant) REQUIRE inv.declaration IS NOT NULL;

CREATE CONSTRAINT implementationfile_id_unique IF NOT EXISTS
FOR (impl:ImplementationFile) REQUIRE impl.id IS UNIQUE;

CREATE CONSTRAINT implementationfile_project_path_unique IF NOT EXISTS
FOR (impl:ImplementationFile) REQUIRE (impl.projectId, impl.path) IS UNIQUE;

