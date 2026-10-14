# 튜토리얼

> **이 문서가 답하는 질문**
> - Ontological DB의 기능을 하나씩, 작은 예제로 따라 해 보려면 어디서 시작하는가?
> - 각 기능이 실제로 무엇을 하고, 무엇은 아직 하지 않는가?

일곱 편 모두 같은 방식으로 만들었습니다. 작은 업무 시나리오 하나를 끝까지 끌고 가고, 데이터는 눈으로 검산할 수 있을 만큼
작으며, **모든 출력은 실제 실행 결과**입니다. 튜토리얼의 원본(`examples/tutorials/<이름>/tutorial.src.md`)에 있는 예제를
빌드 도구가 빈 데이터베이스에서 차례로 실행해 출력을 끼워 넣습니다. 같은 예제가 릴리스 전 회귀 테스트로도 돕니다
(`./tests/tutorials.sh`).

각 편에는 2~3분짜리 데모 영상이 있습니다. Studio에서 같은 흐름을 실제로 실행한 화면이고, 설명은 자막으로 들어 있습니다.

| | 튜토리얼 | 한 줄 | 내려받기 |
|---|---|---|---|
| 1 | [세 언어, 하나의 그래프](three-languages-tutorial.md) | OWL로 들어온 온톨로지를 Cypher와 TypeQL이 함께 읽고 쓴다 | [PDF](three-languages-tutorial.pdf) · [Word](three-languages-tutorial.docx) · [영상](media/three-languages-demo.mp4) |
| 2 | [사이퍼 하나로 여러 DB 조인하기 — 가상 패브릭](virtual-fabric-tutorial.md) | PostgreSQL과 MySQL의 데이터를 복사 없이 그래프로 묻는다 | [PDF](virtual-fabric-tutorial.pdf) · [Word](virtual-fabric-tutorial.docx) · [영상](media/virtual-fabric-demo.mp4) |
| 3 | [시간을 아는 그래프 — 이중 시간](temporal-tutorial.md) | 그때 실제로 어땠는지와 그때 무엇을 믿었는지를 따로 묻는다 | [PDF](temporal-tutorial.pdf) · [Word](temporal-tutorial.docx) · [영상](media/temporal-demo.mp4) |
| 4 | [그래프 순회 중에도 적용되는 권한 — 내재된 보안](security-tutorial.md) | 같은 질의가 역할에 따라 다른 경로를 돌려준다 | [PDF](security-tutorial.pdf) · [Word](security-tutorial.docx) · [영상](media/security-demo.mp4) |
| 5 | [에이전트를 위한 인터페이스](agent-native-tutorial.md) | 에이전트가 스스로 탐색하고, 틀리면 고치고, 실행 전에 가늠한다 | [PDF](agent-native-tutorial.pdf) · [Word](agent-native-tutorial.docx) · [영상](media/agent-native-demo.mp4) |
| 6 | [그래프 안에서 의미로 찾기 — 벡터·하이브리드 검색](vector-search-tutorial.md) | 임베딩이 노드뿐 아니라 관계에도 붙는다 | [PDF](vector-search-tutorial.pdf) · [Word](vector-search-tutorial.docx) · [영상](media/vector-search-demo.mp4) |
| 7 | [운영 중에 바꾸는 스키마 — 스키마 진화](schema-evolution-tutorial.md) | 데이터가 들어 있는 그래프의 타입을 서비스 중에 고친다 | [PDF](schema-evolution-tutorial.pdf) · [Word](schema-evolution-tutorial.docx) · [영상](media/schema-evolution-demo.mp4) |

## 어디서부터 볼까

- **처음이라면** 1번(세 언어)으로 그래프와 타입 계층이 무엇인지 보고, 2번(가상 패브릭)으로 넘어가세요.
- **AI 에이전트를 붙이려 한다면** 5번(에이전트 인터페이스) → 4번(보안)의 "행위(Action)" → 6번(벡터 검색) 순서가 맞습니다.
- **운영을 맡았다면** 7번(스키마 진화)과 3번(이중 시간)을 함께 보세요. 스키마 버전과 데이터의 시점은 같은 질문의 두 축입니다.

## 직접 돌려 보기

```bash
python3 examples/tutorials/build_tutorial.py <이름>     # 한 편을 실행해 문서를 다시 만든다
./tests/tutorials.sh                                    # 전부를 테스트로 실행한다 (아무것도 쓰지 않는다)
python3 examples/tutorials/<이름>/demo.py               # 데모 영상을 다시 녹화한다 (Studio 필요)
```

가상 패브릭 편만 방식이 조금 다릅니다. 원천 DB 두 대가 필요하고, 예제를 Studio를 통해 실행합니다. 그 문서의 "준비"에 있습니다.
