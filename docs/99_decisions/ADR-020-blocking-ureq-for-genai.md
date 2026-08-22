# ADR-020: 외부 네트워크 호출(`genai.vector.encode`)에 async 대신 blocking `ureq`를 쓴다

| 항목 | 값 |
|---|---|
| 상태 | Accepted |
| 날짜 | 2026-08-17 (커밋 `515e4e2`가 `ureq` 의존성을 도입) |
| 영향 범위 | vector, compat, security, ops |
| 근거 | `engine/Cargo.toml:25-29`, `engine/src/compat/genai.rs:1-35` |

> **이 문서가 답하는 질문**
> - 확장이 왜 HTTP 클라이언트를 갖고 있는가?
> - 왜 `reqwest`/`tokio`가 아니라 `ureq`인가?
> - 이 기능이 왜 기본값으로 꺼져 있는가?

## 배경

벡터 검색은 벡터를 받고, 질문하는 쪽은 문자열을 갖고 있다. 그 사이를 무엇이 잇는가가
**클라이언트의 요구사항을 결정한다** (`engine/src/compat/genai.rs:4-8`):
> if the bridge is client-side, every client needs an embedding model and they must all
> agree on it; if it is here, a question can be asked in one Cypher statement and the
> stored vectors and the query vector are produced by the same configuration by
> construction.

Neo4j도 같은 결론에 도달해 `genai.vector.encode`를 추가했고, 이 확장은 **그 이름 그대로**
같은 함수를 제공한다 (`:10-11`).

## 고려한 선택지

1. **클라이언트 측 임베딩만 지원** — 확장이 네트워크를 하지 않는다. 그러나 모든 클라이언트가
   같은 모델·같은 설정에 합의해야 하고, 저장된 벡터와 질의 벡터가 어긋날 위험이 상존한다.
2. **async HTTP 스택(`reqwest` + `tokio`)** — 생태계 표준. 그러나 런타임을 확장 안에
   들여와야 한다.
3. **blocking HTTP 클라이언트(`ureq`)** — 런타임 없음.

## 결정

**3안.** `engine/Cargo.toml:25-29`가 이유를 의존성 옆에 직접 적는다.

> The extension's only outbound network, for `genai.vector.encode`. A blocking client
> with no runtime is the right shape here: a PostgreSQL backend is already the thread
> doing the waiting, so an async stack would buy nothing and cost a scheduler.

```toml
ureq = { version = "2", default-features = false, features = ["json", "tls"] }
```

## 근거

- **PostgreSQL 백엔드는 이미 프로세스당 스레드 하나로 기다리는 모델이다.** 한 백엔드가
  하나의 질의를 처리하므로, 비동기로 얻을 동시성이 애초에 존재하지 않는다. async 런타임은
  스케줄러 비용과 의존성만 추가한다.
- 이 함수가 **확장의 유일한 외부 네트워크 호출**이므로, 의존성 표면을 최소로 유지하는 것이
  합리적이다 (`Cargo.toml:25`).
- 네트워크가 백엔드를 막는다는 사실을 숨기지 않고, **세 겹의 안전장치**를 건다
  (`engine/src/compat/genai.rs:13-30`):

  | 장치 | 내용 |
  |---|---|
  | 기본 비활성 | `genai.enabled`가 `on`이어야 동작 |
  | 엔드포인트는 설정, 인자 아님 | *"Neo4j lets the call name its own endpoint. Here it cannot … Query rights are not fetch rights."* — SSRF 차단 |
  | 시간 상한 | `genai.timeout_ms`, 기본 5,000ms (`:41`) |

- 설정은 `og_catalog.setting`에 저장되고 `og_set_setting()`으로 쓴다 (`:53-62`).

## 결과

**긍정적**
- 확장 의존성이 가볍다. async 런타임과 그 전이 의존성이 없다.
- 저장 벡터와 질의 벡터가 **같은 설정**에서 나온다 — 구조적으로 어긋날 수 없다.
- 엔드포인트를 인자로 받지 않으므로, Cypher를 쓸 수 있는 사용자가 서버로 임의 URL을 fetch
  시킬 수 없다 (Neo4j 대비 의도적으로 좁힌 지점).

**부정적 / 감수한 대가**
- **HTTP를 기다리는 백엔드는 질의를 처리하지 않는 백엔드다.** 주석이 이를 *"a real cost"*
  로 명시한다. 타임아웃이 유일한 방어선이다.
- 트랜잭션 안에서 외부 호출이 일어나므로, 느린 임베딩 서버가 락 보유 시간을 늘릴 수 있다.
- 동시 임베딩 요청 수 = 동시 백엔드 수. 배치·큐잉이 없다.
- 재시도·회로 차단기(circuit breaker)가 없다.

## 재검토 조건

- 대량 임베딩(예: `og_stale_embeddings` 재계산)이 주 사용처가 되면, 백엔드를 막는 동기
  호출 대신 **배경 워커 + 작업 큐** 모델을 재평가한다. 그 시점에는 동시성이 실재하므로
  async 스택의 손익이 뒤집힐 수 있다.
- 외부 호출이 `genai` 외로 늘어나면(예: 원격 추론, 웹훅) 의존성 선택을 다시 계산한다.
  현재 근거는 *"the extension's only outbound network"* 라는 전제 위에 서 있다.
- 타임아웃만으로 부족한 장애(느린 응답의 연속)가 관측되면 회로 차단기를 추가한다.

<!-- affects: vector, compat, security, ops -->
