// Copied from the client: robo-architect-lite:docs/cypher/schema/04_relationships.cypher (statements unchanged)

// One change: randomUUID() is replaced by a fixed UUID (uuid5 of file, statement, occurrence) so the
// fixture is the same at every run; the clients' queries keep their randomUUID().

MATCH (us:UserStory {id: "US-001"})
MATCH (bc:BoundedContext {key: "order"})
CREATE (us)-[:IMPLEMENTS {
    createdAt: datetime(),
    confidence: 0.95
}]->(bc);

MATCH (bc:BoundedContext {key: "order"})
MATCH (agg:Aggregate {key: "order.order"})
CREATE (bc)-[:HAS_AGGREGATE {
    isPrimary: true
}]->(agg);

MATCH (bc:BoundedContext {key: "payment"})
MATCH (pol:Policy {key: "payment.refund-on-order-cancellation"})
CREATE (bc)-[:HAS_POLICY]->(pol);

MATCH (agg:Aggregate {key: "order.order"})
MATCH (cmd:Command {key: "order.order.cancel-order"})
CREATE (agg)-[:HAS_COMMAND {
    isIdempotent: true
}]->(cmd);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (evt:Event {key: "order.order.cancel-order.order-cancelled@1.0.0"})
CREATE (cmd)-[:EMITS {
    isGuaranteed: true
}]->(evt);

MATCH (evt:Event {key: "order.order.cancel-order.order-cancelled@1.0.0"})
MATCH (pol:Policy {key: "payment.refund-on-order-cancellation"})
CREATE (evt)-[:TRIGGERS {
    priority: 1,
    isEnabled: true
}]->(pol);

MATCH (pol:Policy {key: "payment.refund-on-order-cancellation"})
MATCH (cmd:Command {key: "payment.refund.process-refund"})
CREATE (pol)-[:INVOKES {
    isAsync: true
}]->(cmd);

MATCH (bc1:BoundedContext {key: "order"})
MATCH (bc2:BoundedContext {key: "payment"})
CREATE (bc1)-[:DEPENDS_ON {
    integrationPattern: "event"
}]->(bc2);

MATCH (bc:BoundedContext {key: "order"})
MATCH (ui:UI {key: "ui.command.<commandId>"})
CREATE (bc)-[:HAS_UI]->(ui);

MATCH (ui:UI {key: "ui.command.<commandId>"})
MATCH (cmd:Command {key: "order.order.cancel-order"})
CREATE (ui)-[:ATTACHED_TO]->(cmd);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (prop:Property {parentType: "Command", parentId: cmd.id, name: "orderId"})
CREATE (cmd)-[:HAS_PROPERTY]->(prop);

MATCH (src:Property {parentType: "ReadModel", parentId: "<readModelId>", name: "orderId"})
MATCH (tgt:Property {parentType: "Aggregate", parentId: "<aggregateId>", name: "id"})
CREATE (src)-[:REFERENCES]->(tgt);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (given:Given {parentType: "Command", parentId: cmd.id})
CREATE (cmd)-[:HAS_GIVEN]->(given);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (when:When {parentType: "Command", parentId: cmd.id})
CREATE (cmd)-[:HAS_WHEN]->(when);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (then:Then {parentType: "Command", parentId: cmd.id})
CREATE (cmd)-[:HAS_THEN]->(then);

MATCH (bc:BoundedContext {id: "<bc-id>"})
MATCH (j:Journey {id: "<journey-id>"})
MERGE (bc)-[:HAS_JOURNEY]->(j);

MATCH (j:Journey {id: "<journey-id>"})
MATCH (s:JourneyStep {id: "<step-id>"})
MERGE (j)-[:HAS_STEP]->(s);

MATCH (s:JourneyStep {id: "<screen-step-id>"})
MATCH (u:UI {id: "<ui-id>"})
MERGE (s)-[:SHOWS]->(u);

MATCH (a:JourneyStep {id: "<step-a-id>"})
MATCH (b:JourneyStep {id: "<step-b-id>"})
MERGE (a)-[r:NEXT {id: "<deterministic-uuid5>"}]->(b)
ON CREATE SET r.createdAt = datetime()
SET r.condition = "",
    r.documentExcerpt = "원본 문서 인용",
    r.source = "llm",
    r.updatedAt = datetime();

MATCH (bc:BoundedContext { key: "order" })
MATCH (f:Feature { key: "order.feature.order-cancellation" })
MERGE (bc)-[hf:HAS_FEATURE]->(f)
ON CREATE SET hf.createdAt = datetime();

MATCH (f:Feature { key: "order.feature.order-cancellation" })
MATCH (us:UserStory { id: "US-001" })
MERGE (f)-[hus:HAS_USER_STORY]->(us)
ON CREATE SET hus.createdAt = datetime()
SET hus.source = "llm", hus.confidence = 0.9;

MATCH (agg:Aggregate { key: "order.order" })
MATCH (inv:Invariant { key: "order.order.invariant.order-total-positive-abc123def456" })
MERGE (agg)-[:HAS_INVARIANT]->(inv);

MATCH (inv:Invariant { key: "order.order.invariant.order-total-positive-abc123def456" })
MATCH (cmd:Command { key: "order.order.cancel-order" })
MERGE (inv)-[vb:VERIFIED_BY]->(cmd)
ON CREATE SET vb.createdAt = datetime();

MATCH (agg:Aggregate { key: "order.order" })
MATCH (impl:ImplementationFile {
    projectId: "00000000-0000-0000-0000-000000000000",
    path: "src/order/domain/Order.ts"
})
MERGE (agg)-[:IMPLEMENTED_IN]->(impl);

MATCH (chg:RequirementChange {id: "CHG-001"})
MATCH (us:UserStory {id: "US-001"})
CREATE (chg)-[:EFFECT {
    reason: "반품 기간 변경으로 인수조건 수정 필요",
    impactLevel: "HIGH",
    diff: null,
    appliedAt: null
}]->(us);

MATCH (cs:ChangeSet {id: "CS-001"})
MATCH (chg:RequirementChange {id: "CHG-001"})
MERGE (cs)-[:CONTAINS]->(chg);

