# Банк теории для `/mock` (режим «учёба»)

Используется командой `/mock`: после поведенческих вопросов агент задаёт эти вопросы по одному, владелец смотрит ответ бота, обсуждаем и освежаем теорию. Коды (`MC-01` …) нужны для прогресса в `mock-progress.json`.

### Мистер классик (`MC`)

- **MC-01.** Что такое TF-IDF и как оно работает?
- **MC-02.** Лемматизация vs стемминг: алгоритмы и когда применять.
- **MC-03.** Что такое SVM и как оно работает?
- **MC-04.** Наивный Байес: как работает и где применять.
- **MC-05.** Дисбаланс классов: как увидеть и что делать?
- **MC-06.** Обучение классификатора на *k* классов при ~10 примерах на класс: few-shot, synthetic data, augmentation, transfer learning.
- **MC-07.** Как построить сильный baseline для NLP-задачи перед BERT/LLM?
- **MC-08.** Как избежать data leakage в текстовой классификации?

### Векторизэйшн (`VEC`)

- **VEC-01.** Способы получения эмбеддингов и когда какой лучше.
- **VEC-02.** Что такое Word2Vec?
- **VEC-03.** CBOW vs Skip-gram. Какая учится быстрее?
- **VEC-04.** Основные типы векторизации.
- **VEC-05.** Почему важна размерность эмбеддинга?
- **VEC-06.** Как учится Word2Vec? Лосс/что максимизируется.
- **VEC-07.** Разница: GloVe, ELMo, FastText vs Word2Vec.
- **VEC-08.** Что такое sentence embedding? Способы получения.
- **VEC-09.** Чем bi-encoder отличается от cross-encoder?
- **VEC-10.** Что такое reranker и почему он часто лучше простого cosine similarity?
- **VEC-11.** Dense embeddings vs sparse embeddings vs hybrid retrieval.
- **VEC-12.** Как выбрать embedding model для RAG: язык, домен, размерность, latency, cost, benchmark.

### Архитектура нейросеток (`ARC`)

- **ARC-01.** Что такое RNN?
- **ARC-02.** Как обучается RNN (BPTT)?
- **ARC-03.** Сравнить RNN и attention: когда что использовать?
- **ARC-04.** Что такое трансформер: внутреннее устройство.
- **ARC-05.** Что такое Cross-Attention?
- **ARC-06.** Размерности в self-attention матрицах.
- **ARC-07.** Разница между головами и слоями в трансформерах.
- **ARC-08.** Зачем PreNorm и PostNorm в трансформерах?
- **ARC-09.** Как число голов влияет на параметры, память и вычисления в attention?
- **ARC-10.** Проблемы RNN: vanishing gradients, параллелизация.
- **ARC-11.** Как считается attention? История, формулы, виды.
- **ARC-12.** Объяснить multi-head attention.
- **ARC-13.** LayerNorm или BatchNorm и почему?
- **ARC-14.** Что такое RoPE и почему его используют в современных LLM?
- **ARC-15.** Что такое FlashAttention и какую проблему он решает?
- **ARC-16.** MHA vs MQA vs GQA: отличия, плюсы/минусы для инференса.
- **ARC-17.** Почему long-context модели всё равно могут плохо использовать середину контекста?

### Мистер Transformers (`TR`)

- **TR-01.** Архитектура энкодера в BERT.
- **TR-02.** Архитектура декодера в GPT.
- **TR-03.** Разница BERT vs GPT в подсчёте attention.
- **TR-04.** Размерность эмбеддингового слоя в трансформере.
- **TR-05.** Почему эмбеддинги контекстуальные?
- **TR-06.** Как работает декодер GPT на обучении и инференсе.
- **TR-07.** Сравнение BERT, GPT и T5.
- **TR-08.** На каких задачах обучался BERT?
- **TR-09.** Что такое T5 и BART? Отличия.
- **TR-10.** Проблема знаний в BERT/GPT и как решать.
- **TR-11.** Устройство T5, Llama 2/3, Mistral.
- **TR-12.** Instruction-tuned model vs base model: в чём разница?
- **TR-13.** Chat template: зачем нужен и что ломается при неправильном шаблоне?
- **TR-14.** Чем reasoning model отличается от обычной instruction model?
- **TR-15.** Что такое test-time compute / inference-time scaling?
- **TR-16.** Малые языковые модели (SLM): когда выбрать маленькую модель вместо большой?

### Обучение и оптимизация (`OPT`)

- **OPT-01.** Этапы обучения LLM. От чего можно отказаться и когда?
- **OPT-02.** Как обучается causal LM?
- **OPT-03.** Что такое адаптеры и где их применять?
- **OPT-04.** PEFT: обзор методов — LoRA, Prefix, Prompt-tuning, P-tuning.
- **OPT-05.** PEFT/LoRA: какие матрицы меняются/замораживаются? Альтернативы.
- **OPT-06.** LoRA: выбор гиперпараметров и что делать, если не лезет в память.
- **OPT-07.** Prefix vs P-tuning vs Prompt-tuning.
- **OPT-08.** Почему LoRA экономит параметры: механика.
- **OPT-09.** Alignment: что это?
- **OPT-10.** RLHF/DPO/PPO — различия.
- **OPT-11.** Как оптимизировать ресурсы при обучении.
- **OPT-12.** Как предотвращать катастрофическое забывание.
- **OPT-13.** Оптимизаторы: обзор, Adam, влияние gradient accumulation.
- **OPT-14.** Способы распределённого обучения: DP, ZeRO, PP, TP, FSDP, DeepSpeed.
- **OPT-15.** QLoRA: как работает и чем отличается от обычной LoRA?
- **OPT-16.** SFT vs DPO vs ORPO/KTO/GRPO: когда какой подход выбирать?
- **OPT-17.** Как подготовить датасет для instruction tuning?
- **OPT-18.** Как бороться с переобучением при fine-tuning LLM на маленьком датасете?
- **OPT-19.** Synthetic data для LLM: где помогает, где опасна?

### Inference (`INF`)

- **INF-01.** Docker в ML/LLM production: зачем контейнеры и что важно знать?
- **INF-02.** Kubernetes в ML/LLM production: deployment, autoscaling, GPU scheduling.
- **INF-03.** Фреймворки инференса: vLLM / TensorRT-LLM / SGLang — сравнение.
- **INF-04.** Что такое NVIDIA Triton и зачем.
- **INF-05.** Температура в softmax: что делает и что выставлять.
- **INF-06.** Виды sampling: top-k, top-p, nucleus.
- **INF-07.** Что такое constrained decoding?
- **INF-08.** KV-cache, GQA, MQA — принципы.
- **INF-09.** Почему первый токен медленнее следующих? TTFT vs tokens/sec.
- **INF-10.** Continuous batching в LLM serving: что это и почему важно.
- **INF-11.** PagedAttention: какую проблему решает в vLLM?
- **INF-12.** Speculative decoding: как ускоряет генерацию и когда не помогает.
- **INF-13.** Prefix caching / prompt caching: где экономит latency и cost.
- **INF-14.** Как маршрутизировать запросы между маленькой/большой моделью?
- **INF-15.** Как оценивать latency/cost/quality trade-off при выборе модели?

### Токенизация (`TOK`)

- **TOK-01.** Почему в трансформерах не лемматизация? Зачем токены?
- **TOK-02.** Виды токенайзеров и сравнение.
- **TOK-03.** Какой токенизатор в BERT, а какой в GPT?
- **TOK-04.** Обычные vs специальные токены.
- **TOK-05.** Расширение токенайзера: когда переобучать и что делать при добавлении токенов.
- **TOK-06.** Как обрабатываются OOV слова?
- **TOK-07.** На что влияет размер словаря токенизатора?
- **TOK-08.** WordPiece vs BPE.
- **TOK-09.** Как обучается токенизатор WordPiece/BPE?
- **TOK-10.** SentencePiece в T5.
- **TOK-11.** Почему число токенов важно для стоимости, latency и качества?
- **TOK-12.** Что ломается при добавлении новых special tokens без resize embeddings?

### RAG и LLM-экосистема (`RAG`)

- **RAG-01.** Как работает RAG? Отличия от few-shot KNN.
- **RAG-02.** Как оценивать качество RAG.
- **RAG-03.** Что такое LangChain и LangGraph, зачем они.
- **RAG-04.** Как работают векторные БД? Примеры.
- **RAG-05.** Алгоритмы поиска: BM25, HNSW.
- **RAG-06.** Как дообучить ретривер?
- **RAG-07.** Узкий домен, общий вопрос пользователя — как улучшить ретривер.
- **RAG-08.** Naive RAG vs Advanced RAG vs Agentic RAG.
- **RAG-09.** Chunking strategies: fixed, recursive, semantic, parent-child. Как выбрать?
- **RAG-10.** Hybrid search: BM25 + dense retrieval. Зачем и как объединять scores?
- **RAG-11.** Query transformation: HyDE, query rewriting, query decomposition, step-back prompting.
- **RAG-12.** Reranking: cross-encoder, LLM reranker, late interaction. Когда нужен?
- **RAG-13.** GraphRAG: когда граф знаний лучше обычного vector search?
- **RAG-14.** Как работать с таблицами, PDF, сканами и layout-aware документами в RAG?
- **RAG-15.** Как сделать цитирование источников и не дать модели ссылаться на неиспользованный документ?
- **RAG-16.** Access control в enterprise RAG: как не отдать пользователю чужие документы?
- **RAG-17.** Freshness/versioning: как обновлять индекс при изменении документов?
- **RAG-18.** Как дебажить RAG, если правильный документ есть, но ответ неверный?
- **RAG-19.** RAG vs fine-tuning: что выбрать для знаний, стиля, формата, доменной терминологии?

### Агенты и tool use (`AG`)

- **AG-01.** Что такое AI agent и чем он отличается от обычного LLM-call?
- **AG-02.** ReAct: как работает связка reasoning + acting?
- **AG-03.** Plan-and-execute агент: когда лучше ReAct?
- **AG-04.** Tool calling / function calling: как модель выбирает инструмент и аргументы?
- **AG-05.** Как проектировать schema для tools, чтобы модель реже ошибалась?
- **AG-06.** Что такое agent memory: short-term, long-term, episodic, semantic?
- **AG-07.** Когда агенту нужна память, а когда достаточно RAG?
- **AG-08.** LangGraph: почему graph/state-machine подход удобнее простых chains?
- **AG-09.** Human-in-the-loop: где ставить approval/checkpoint?
- **AG-10.** Как оценивать агента: final answer metrics vs trajectory/tool-call metrics.
- **AG-11.** Типичные failure modes агентов: loops, wrong tool, stale memory, over-planning, hallucinated actions.
- **AG-12.** Как безопасно давать агенту доступ к внешним действиям: email, DB, shell, CRM?
- **AG-13.** Чем workflow отличается от agent? Когда лучше жёсткий граф, а не автономный агент?

### MCP и интеграции (`MCP`)

- **MCP-01.** Что такое Model Context Protocol и какую проблему он решает?
- **MCP-02.** MCP client vs MCP server: кто за что отвечает?
- **MCP-03.** Tools, resources, prompts в MCP: чем отличаются?
- **MCP-04.** Как MCP server описывает tool: name, description, input schema, output.
- **MCP-05.** Почему JSON schema важна для tool calling?
- **MCP-06.** MCP vs обычный REST API wrapper вокруг tool calling.
- **MCP-07.** Как реализовать permissions и user consent для MCP tools?
- **MCP-08.** Prompt injection через tool/resource output: как защищаться?
- **MCP-09.** Какие риски у MCP при доступе к файловой системе, браузеру, БД или shell?
- **MCP-10.** Когда MCP действительно нужен, а когда это лишняя сложность?

### Мультимодальные модели (`MM`)

- **MM-01.** Что такое мультимодальная модель / VLM?
- **MM-02.** CLIP: как обучается и почему умеет связывать текст и изображения?
- **MM-03.** Как LLM подключают к vision encoder: projector, adapter, cross-attention.
- **MM-04.** OCR pipeline vs VLM: когда что выбрать для документов?
- **MM-05.** Multimodal RAG: как искать по тексту, картинкам, таблицам и сканам?
- **MM-06.** Как обрабатывать PDF с таблицами, графиками и layout?
- **MM-07.** Hallucinations в VLM: почему модель видит то, чего нет, и как это ловить?
- **MM-08.** Grounding в изображении: bounding boxes, citations, visual evidence.
- **MM-09.** Как оценить качество извлечения информации из документов: precision/recall по полям, exact match, human review.
- **MM-10.** Какие риски у multimodal input: hidden text, QR, screenshots with secrets, prompt injection in image.
- **MM-11.** Когда лучше использовать специализированный OCR/table parser, а не general VLM?

### Prompt engineering и structured output (`PRM`)

- **PRM-01.** Zero-shot, few-shot, chain-of-thought: когда что использовать?
- **PRM-02.** Что такое system prompt и какие ограничения у него есть?
- **PRM-03.** Как стабилизировать формат ответа: JSON mode, schema, constrained decoding, retry parser.
- **PRM-04.** Self-check / critique: как использовать без бесконечных self-correction loops?
- **PRM-05.** Lost in the middle: почему модель игнорирует часть длинного контекста?
- **PRM-06.** Prompt injection: чем отличается от jailbreak?
- **PRM-07.** Как проектировать промпт для классификации, extraction и summarization?
- **PRM-08.** Как тестировать промпты: eval set, regression, A/B, human review.
- **PRM-09.** Почему нельзя просить модель раскрывать chain-of-thought в production?

### LLMOps, evals и безопасность (`OPS`)

- **OPS-01.** Что такое LLMOps и чем отличается от MLOps?
- **OPS-02.** Что логировать в LLM-приложении: prompt, model, params, retrieved chunks, tool calls, latency, cost.
- **OPS-03.** Как делать regression tests для LLM/RAG/agent системы?
- **OPS-04.** Offline eval vs online eval vs human SBS.
- **OPS-05.** LLM-as-a-judge: плюсы, минусы, bias, калибровка.
- **OPS-06.** Как мониторить hallucination/groundedness в production?
- **OPS-07.** Guardrails: input guards, output guards, policy checks, PII redaction.
- **OPS-08.** Prompt injection и data exfiltration: как защищать RAG/agents?
- **OPS-09.** Как считать cost per request и оптимизировать token spend?
- **OPS-10.** Как выбирать между open-source моделью и API-моделью?
- **OPS-11.** Как организовать fallback, retry, circuit breaker для LLM API?
- **OPS-12.** Какие privacy/compliance риски есть при отправке данных в external LLM API?

### Метрики (`MET`)

- **MET-01.** С какими NLP-метриками работал(а)?
- **MET-02.** Метрики близости текстов.
- **MET-03.** Accuracy/Recall/Precision/F1/ROC-AUC.
- **MET-04.** BLEU.
- **MET-05.** ROUGE и различия её вариантов.
- **MET-06.** Отличие BLEU от ROUGE.
- **MET-07.** G-Eval: плюсы/минусы.
- **MET-08.** Precision vs Recall простыми словами; что смотреть без F1.
- **MET-09.** Когда macro/micro и зачем weighted.
- **MET-10.** Метрики ретривала: MRR, NDCG, MAP, Recall — плюсы/минусы.
- **MET-11.** 4 ключевые метрики из RAGAS.
- **MET-12.** Perplexity: что это и где считать.
- **MET-13.** Faithfulness, answer relevance, context precision, context recall — что означают?
- **MET-14.** Как оценивать tool calling accuracy?
- **MET-15.** Как оценивать structured extraction: exact match, field-level F1, semantic equivalence.
- **MET-16.** Как сделать golden dataset для LLM eval?

### Квантизация и ускорение (`QTZ`)

- **QTZ-01.** Методы квантизации: GPTQ, AWQ и др.
- **QTZ-02.** Можно ли тюнить квантизованные модели?
- **QTZ-03.** QAT vs Post-Training Quantization.
- **QTZ-04.** Квантизация для resource-constrained: когда и как выбирать параметры.
- **QTZ-05.** INT8 vs INT4 vs FP16/BF16: что теряем и что выигрываем?
- **QTZ-06.** Weight-only quantization vs activation quantization.
- **QTZ-07.** Как проверить, что квантизация не сломала качество?

### Production (`PRD`)

- **PRD-01.** Подход к отладке сложной прод-проблемы.
- **PRD-02.** Как загрузить кастомную модель like-BERT.
- **PRD-03.** Как определить порядок препроцессинга документа.
- **PRD-04.** Важные тренды в NLP.
- **PRD-05.** Описать свой end-to-end.
- **PRD-06.** Decoder-only модель выучила плохие паттерны: как предотвратить генерацию.
- **PRD-07.** Модель дала 90%+ confidence и ошиблась: calibration, distribution shift, overconfidence.
- **PRD-08.** Как построить end-to-end GenAI feature: от задачи до мониторинга?
- **PRD-09.** Как выбирать модель под бизнес-задачу: качество, latency, cost, privacy, лицензия.
- **PRD-10.** Как сделать безопасный fallback, если LLM/RAG не уверен?
- **PRD-11.** Как проектировать API для LLM-сервиса: sync/async, streaming, idempotency, retries.
- **PRD-12.** Как работать с PII/secrets в логах, промптах и RAG-документах?
- **PRD-13.** Как сделать multilingual NLP/LLM систему и что проверять отдельно?

### Python: базовые вопросы (`PYB`)

- **PYB-01.** Изменяемые и неизменяемые типы данных. Приведите конкретные примеры.
- **PYB-02.** Словарь, коллизии, хэш, как устроен низкоуровнево.
- **PYB-03.** Сложность добавления/чтения в: list — конец, произвольное место; dict.
- **PYB-04.** Объясните, как работает итератор + примеры в torch.
- **PYB-05.** Что такое декоратор функции и как он работает?
- **PYB-06.** Контекстный менеджер, примеры.
- **PYB-07.** Что такое GIL, как он работает. CPU-bound и IO-bound задачи.
- **PYB-08.** Разница между многопоточностью, асинхронностью и многопроцессностью. Примеры задач на каждый тип. Чем отличается async/await от обычных функций и вызовов?
- **PYB-09.** Что такое lambda-функции в Python и для чего они используются?
- **PYB-10.** Разница `is` и `==`.
- **PYB-11.** Магические методы.
- **PYB-12.** Что такое list/dict comprehension и как они используются?
- **PYB-13.** Объясните, как работают `map`, `reduce`, `filter`.
- **PYB-14.** Generator vs iterator: в чём разница?
- **PYB-15.** Что такое `yield` и когда использовать генераторы?
- **PYB-16.** `dataclass` vs обычный class vs Pydantic model.
- **PYB-17.** Что такое type hints и как они помогают в ML/production коде?
- **PYB-18.** Что такое virtualenv/poetry/uv/requirements и зачем фиксировать зависимости?

### Python: продвинутые вопросы (`PYA`)

- **PYA-01.** Общие алгоритмы поиска и обхода графов на Python.
- **PYA-02.** Как работает управление памятью и сборщик мусора.
- **PYA-03.** В чём разница между shallow copy и deep copy?
- **PYA-04.** Как работает try/except/finally/else?
- **PYA-05.** Что такое метаклассы?
- **PYA-06.** Защищённые и приватные атрибуты.
- **PYA-07.** Как работает `asyncio` event loop?
- **PYA-08.** Что такое coroutine, task, future?
- **PYA-09.** Когда использовать multiprocessing, а когда ThreadPool/ProcessPool?
- **PYA-10.** Как профилировать Python-код: cProfile, py-spy, line_profiler.
- **PYA-11.** Как ускорять Python для ML pipeline: vectorization, batching, multiprocessing, numba, C extensions.
- **PYA-12.** Что такое serialization/pickling и какие риски есть у pickle?
- **PYA-13.** Как писать production-ready Python package: структура, tests, lint, typing, CI.
- **PYA-14.** Как обрабатывать большие файлы/датасеты, не загружая всё в память?
- **PYA-15.** Как работает PyTorch Dataset/DataLoader, workers, pin_memory, collate_fn?
