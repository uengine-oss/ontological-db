// Copied from the client: robo-architect-lite:docs/cypher/schema/03_node_types.cypher (statements unchanged)

// One change: randomUUID() is replaced by a fixed UUID (uuid5 of file, statement, occurrence) so the
// fixture is the same at every run; the clients' queries keep their randomUUID().

CREATE (r:Requirement {
    id: "REQ-001",
    title: "주문 관리 기능",
    description: "고객이 주문을 생성, 조회, 취소할 수 있어야 한다",
    source: "기능요구사항서 v1.0",
    createdAt: datetime(),
    priority: "high"
});

CREATE (us:UserStory {
    id: "US-001",
    role: "customer",
    action: "cancel my order",
    benefit: "I can get a refund if I change my mind",
    priority: "high",
    status: "approved"
});

MERGE (bc:BoundedContext { key: "order" })
ON CREATE SET bc.id = '9295692a-7d23-552d-9f67-0073c7f6a5c6'
SET bc.name = "Order",
    bc.description = "주문 생성, 수정, 취소 및 주문 상태 관리",
    bc.owner = "Order Team",
    bc.classification = "core";

MATCH (bc:BoundedContext { key: "order" })
MERGE (agg:Aggregate { key: "order.order" })
ON CREATE SET agg.id = '2569a96c-e620-5fb5-85fb-976a198c009c'
SET agg.name = "Order",
    agg.rootEntity = "Order",
    agg.invariants = [
        "주문 총액은 0보다 커야 함",
        "배송 시작 후에는 취소 불가"
    ]
MERGE (bc)-[:HAS_AGGREGATE]->(agg);

MATCH (agg:Aggregate { key: "order.order" })
MERGE (cmd:Command { key: "order.order.cancel-order" })
ON CREATE SET cmd.id = 'f6cb76b8-5b8a-5b99-a42d-077bf3eb4554'
SET cmd.name = "CancelOrder",
    cmd.actor = "customer",
    cmd.inputSchema = '{"orderId": "string", "reason": "string"}'
MERGE (agg)-[:HAS_COMMAND]->(cmd);

MATCH (cmd:Command { key: "order.order.cancel-order" })
MERGE (evt:Event { key: "order.order.cancel-order.order-cancelled@1.0.0" })
ON CREATE SET evt.id = '9761503d-d888-557a-86a4-2ada5026e73f'
SET evt.name = "OrderCancelled",
    evt.version = "1.0.0",
    evt.schema = '{"orderId": "string", "cancelledAt": "datetime", "reason": "string"}',
    evt.isBreaking = false
MERGE (cmd)-[:EMITS]->(evt);

MATCH (bcPayment:BoundedContext { key: "payment" })
MERGE (pol:Policy { key: "payment.refund-on-order-cancellation" })
ON CREATE SET pol.id = '044fa7a5-140f-528a-9f15-a7f9947423f7'
SET pol.name = "RefundOnOrderCancellation",
    pol.condition = "OrderCancelled received",
    pol.description = "주문 취소 이벤트 수신 시 환불 처리"
MERGE (bcPayment)-[:HAS_POLICY]->(pol);

MATCH (cmd:Command { key: "order.order.cancel-order" })
MERGE (prop:Property { parentType: "Command", parentId: cmd.id, name: "orderId" })
ON CREATE SET prop.id = '90a8af15-e50c-5d91-bc8b-62adb5ed5435'
SET prop.type = "UUID",
    prop.description = "주문 고유 식별자",
    prop.isKey = true,
    prop.isForeignKey = false,
    prop.isRequired = true
MERGE (cmd)-[:HAS_PROPERTY]->(prop);

MATCH (bc:BoundedContext { key: "order" })
MATCH (cmd:Command { key: "order.order.cancel-order" })
MERGE (ui:UI { key: "ui.command." + cmd.id })
ON CREATE SET ui.id = 'b4ccc576-6b93-55e2-a711-82047f6e1b0f'
SET ui.name = "CancelOrder UI",
    ui.description = "주문 취소 화면: 주문번호 입력 후 취소 사유를 선택하고 '취소' 버튼 클릭",
    ui.template = "",
    ui.attachedToId = cmd.id,
    ui.attachedToType = "Command",
    ui.attachedToName = cmd.name,
    ui.userStoryId = "US-001"
MERGE (bc)-[:HAS_UI]->(ui)
MERGE (ui)-[:ATTACHED_TO]->(cmd);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MERGE (given:Given {parentType: "Command", parentId: cmd.id, key: "given." + cmd.id})
ON CREATE SET given.id = '9b3f3513-4195-5eff-b76f-08bde2976476'
SET given.name = "Command: CancelOrder",
    given.description = "주문 취소 명령이 실행됨",
    given.referencedNodeId = cmd.id,
    given.referencedNodeType = "Command"
MERGE (cmd)-[:HAS_GIVEN]->(given)
MERGE (given)-[:REFERENCES]->(cmd);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (agg:Aggregate {key: "order.order"})
MERGE (when:When {parentType: "Command", parentId: cmd.id, key: "when." + cmd.id})
ON CREATE SET when.id = '8eeb0a2d-47eb-5391-a987-83c0559058e4'
SET when.name = "Aggregate: Order",
    when.description = "Order Aggregate가 Command를 처리함",
    when.referencedNodeId = agg.id,
    when.referencedNodeType = "Aggregate"
MERGE (cmd)-[:HAS_WHEN]->(when)
MERGE (when)-[:REFERENCES]->(agg);

MATCH (cmd:Command {key: "order.order.cancel-order"})
MATCH (evt:Event {key: "order.order.cancel-order.order-cancelled@1.0.0"})
MERGE (then:Then {parentType: "Command", parentId: cmd.id, key: "then." + cmd.id})
ON CREATE SET then.id = '77c6a9cb-93ba-5a04-ad6e-49121a67a826'
SET then.name = "Event: OrderCancelled",
    then.description = "주문 취소 이벤트가 발생함",
    then.referencedNodeId = evt.id,
    then.referencedNodeType = "Event"
MERGE (cmd)-[:HAS_THEN]->(then)
MERGE (then)-[:REFERENCES]->(evt);

CREATE (j:Journey {
    id: "00000000-0000-5000-8000-000000000001",
    key: "membership.journey.normal-signup-abc123",
    journeyId: "normal-signup-abc123",
    name: "정상 회원가입",
    description: "신규 사용자가 약관 동의부터 가입 완료까지 진행하는 여정",
    boundedContextId: "<bc-uuid>",
    source: "llm",
    createdAt: datetime(),
    updatedAt: datetime()
});

CREATE (s:JourneyStep {
    id: "00000000-0000-5000-8000-000000000002",
    key: "membership.journey.normal-signup-abc123.step.gateway.approve-xyz",
    kind: "gateway",
    label: "가입 가능 여부?",
    sequence: 3,
    source: "llm",
    createdAt: datetime(),
    updatedAt: datetime()
});

MATCH (bc:BoundedContext { key: "order" })
MERGE (f:Feature { key: "order.feature.order-cancellation" })
ON CREATE SET f.id = '31c0ac99-bc9d-5cbc-a040-4409e5f8554b', f.createdAt = datetime(), f.source = "llm"
SET f.name = "주문 취소",
    f.description = "고객이 주문을 취소하고 환불을 받는 기능 묶음",
    f.boundedContextId = bc.id,
    f.sequence = 1,
    f.updatedAt = datetime()
MERGE (bc)-[:HAS_FEATURE]->(f);

MATCH (agg:Aggregate { key: "order.order" })
MERGE (inv:Invariant { key: "order.order.invariant.order-total-positive-abc123def456" })
ON CREATE SET inv.id = 'f4ef7b59-d04a-5063-ada8-a42e71f05cce', inv.createdAt = datetime(), inv.source = "manual"
SET inv.declaration = "주문 총액은 항상 0보다 커야 한다",
    inv.name = "주문 총액 양수",
    inv.aggregateId = agg.id,
    inv.seq = 1,
    inv.updatedAt = datetime()
MERGE (agg)-[:HAS_INVARIANT]->(inv);

MATCH (agg:Aggregate { key: "order.order" })
MERGE (impl:ImplementationFile {
    projectId: "00000000-0000-0000-0000-000000000000",
    path: "src/order/domain/Order.ts"
})
ON CREATE SET impl.id = '5099745e-60ea-5889-ad32-a7dccd840ba3',
              impl.role = "primary",
              impl.createdAt = datetime(),
              impl.lastSeenAt = datetime()
ON MATCH SET impl.lastSeenAt = datetime()
MERGE (agg)-[:IMPLEMENTED_IN]->(impl);

CREATE (chg:RequirementChange {
    id: "CHG-001",
    title: "반품 기간 연장",
    originalPrompt: "반품 기간을 7일에서 14일로 변경",
    author: "user@example.com",
    createdAt: datetime(),
    status: "DRAFT",
    statusHistory: "[]",
    sourceType: "PROMPT",
    changeSetId: null
});

CREATE CONSTRAINT req_change_id IF NOT EXISTS
FOR (n:RequirementChange) REQUIRE n.id IS UNIQUE;

CREATE (cs:ChangeSet {
    id: "CS-001",
    title: "Q3 정책 변경 묶음",
    author: "user@example.com",
    createdAt: datetime(),
    status: "DRAFT"
});

CREATE CONSTRAINT changeset_id IF NOT EXISTS
FOR (n:ChangeSet) REQUIRE n.id IS UNIQUE;

CREATE CONSTRAINT proposal_id_unique IF NOT EXISTS
FOR (p:Proposal) REQUIRE p.id IS UNIQUE;

CREATE INDEX proposal_status IF NOT EXISTS
FOR (p:Proposal) ON (p.status);

CREATE INDEX proposal_author IF NOT EXISTS
FOR (p:Proposal) ON (p.author);

CREATE CONSTRAINT constitution_id_unique IF NOT EXISTS
FOR (c:Constitution) REQUIRE c.id IS UNIQUE;

CREATE INDEX constitution_scope IF NOT EXISTS
FOR (c:Constitution) ON (c.scope);

