MATCH (am:ActivityMapping {session_id: $sid})
OPTIONAL MATCH (t:BpmTask {id: am.task_id, session_id: $sid})-[r:REALIZED_BY]->(:Rule {id: am.rule_id, session_id: $sid})
// `r` 을 WITH 로 함께 넘긴다 — 투영에서 떨어뜨린 변수는
// 뒤따르는 WHERE 에서 볼 수 없다. 게이트웨이가 문법 오류로 막는다.
WITH am, r
WHERE r IS NULL
RETURN am
