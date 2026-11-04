# requirements.md — aria router（Rust）

> 本文件为 `router` 仓库 **Semantic + Builtin Agent + AFM-D 并列网关 / OpenAI 兼容 HTTP / 八语言 SDK** 的功能边界、API、配置、异常与验收标准。**须经人工逐项审核**，审核通过后方可据其生成 / 执行 `task.md`。
>
> 架构参考：[vLLM Semantic Router](https://github.com/vllm-project/semantic-router) YAML v0.3（运行时对等，不做 Operator/Envoy）；运维 Dashboard 对齐其 dashboard 的 Config / Topology / Playground / Replay，不接 Grafana / ML / Security。Agent 面为 **进程内** 定制 builtin（少量固定工具 + 限 turns），不做 pi / deepseek-harness 子进程。AFM-D 面为 **HTTP System One**（契约对齐 `model/afm-d` / aria-engine），不做进程内 candle。

## 1. 目标与范围

用 **Rust** 实现独立 OpenAI 兼容网关：按 entrypoint 选择 **semantic**、**agent**（builtin）或 **afm-d**（System One）决策器，在硬约束剪枝后选择或组合 provider 路径并转发。

- **产品面**：`aria-router` CLI（`setup` / `validate` / `serve` / `upgrade`）+ 进程内库 + C ABI + 八语言 SDK。
- **三种 router**：`semantic`（signals → Boolean recipe → algorithm）、`agent`（轻量进程内 builtin：固定工具 + `max_turns`）、`afm-d`（eligible → System One Choice → aria-engine）并列；共享 listeners / providers / 硬约束 / 转发 / replay。
- **不做**：Operator、Helm、官网、Python `vllm-sr`、Envoy ExtProc、Grafana / Prometheus、ML wizard、Security Policy、wizmap、fleet-sim、pi / deepseek-harness / 可插拔 `extensions`、把 TS harness 链进 crate、在 router 内嵌 AFM-D 权重 / candle。
- **与 engine**：engine 为本地 typed-decision 推理；**afm-d** 决策器经 HTTP `POST /v1/systemone` 调用；也可另将 engine 注册为 chat provider。本仓 SDK 与 `ariacompute-engine` **两套包**，`.so` 名互不覆盖。

### 1.1 阶段

| 阶段 | 名称 | 交付深度 |
|------|------|----------|
| **A-semantic** | 黄金路径 | YAML 加载、keyword、Boolean、static、chat/SSE、bypass、fail closed |
| **A-agent** | 并列黄金路径 | 进程内 builtin tool-loop、硬剪枝、越权拒绝、与 semantic 入口隔离 |
| **A-afm-d** | AFM-D 决策器 | `router: afm-d`；硬剪枝 → System One Choice → `RouteDecision`；无 endpoint first-eligible；tiny/gateway 示例 |
| **B** | 启发式管线 | 启发式 signals、projections、latency-aware / multi-factor、核心 plugins |
| **C** | 运行时对等 | learned signals（ONNX feature `ml`）、剩余 algorithm/looper/plugin；未实现显式 Unsupported |
| **E** | SDK | C ABI + 八语言；`cases.json`；`run-binding-tests.sh` |
| **F-dashboard** | 运维面 | 管理面 SPA：Overview / Config（可写热重载）/ Topology / Providers / Replay / Playground（多轮聊天气泡 + SSE + 路由 Header 面板；对齐 vLLM SR 核心体验，不做 MCP/Claw/联网/附件） |
| **G-cost** | 成本 + API key | 六因子成本账本；YAML `pricing`；Dashboard「API 密钥」签发；数据面 / provider 注册 Bearer；engine 传 `router_api_key` |
| **H-accounts** | 本地用户 + OAuth | Dashboard 注册/登录（密码）；本地 `sk-aria_` 归属用户；OAuth（Aria Compute）关联 + `sk-bf-`；cost `by_local_user` / `by_serve_user`；CLI 扁平 setup |
| **R-stateful-prod** | 状态化 + 生产 A + 决策器 B | `emits`/`retention`；`global.stores`/`services`；feature `ml` learned；Elo/`ratings`；semantic_cache；软限流；加厚 replay |

未达阶段的 YAML 能力须 `Unsupported`，禁止静默空实现。

### 1.2 硬不变量

- **Location / auth / modality / tools**：硬剪枝在决策之前；agent / afm-d 只看见合格候选；无合格路径 fail closed。
- **Compute** 进 pool 排名/工具结果，不进 eligibility。
- **Preference**：semantic 用 signal；agent 用 prompt/tools 上下文；afm-d 用 System One Choice；不得覆盖 Location。
- 检测 ≠ 执行。Agent / AFM-D 输出必须是 typed `RouteDecision`。
- 一次请求禁止串跑多种决策器。实名模型 **bypass** recipe。
- `entrypoint.router` 必须等于 `recipe.router`。Semantic recipe 禁止 `agent:` / `afm-d:`；agent recipe 禁止 `signals`/`decisions` / `afm-d:`；afm-d recipe 禁止 `routing`/`agent:`。

## 2. 功能边界

| # | 特性 | 实现深度 |
|---|------|----------|
| 1 | **config** | YAML v0.3 结构：`version` / `listeners` / `providers` / `entrypoints` / `recipes` / `global`；`${VAR}` 替换；引用校验；**无**顶层 `extensions` |
| 2 | **semantic** | signals、projections、Boolean AST、priority/confidence、algorithms、plugins |
| 3 | **agent** | 进程内 `BuiltinAgent`；固定工具 + `max_turns` / `timeout_ms`；schema 校验 |
| 3b | **afm-d** | HTTP System One Choice 决策器；`afm-d.endpoint` → aria-engine；可选 `track: encoder\|decoder`；无 endpoint → first-eligible；`fallback` / `min_confidence` |
| 4 | **provider** | OpenAI 兼容转发、加权 `backend_refs`、health、latency 采样 |
| 5 | **http** | 数据面 `:8899` chat/SSE/`/v1/models`；管理面默认 `127.0.0.1` health/validate/replay/providers + Dashboard API |
| 6 | **ffi** | `libaria-router_ffi`：init/connect/complete/stream/models/last_route |
| 7 | **bindings** | rust/python/go/typescript/react-native/flutter/swift/kotlin |
| 8 | **dashboard** | 管理面同端口 SPA（`dashboard/`）；`--no-dashboard` 仅 JSON API；Cost / API 密钥页 |
| 9 | **cost** | 内存六因子账本；`GET /v1/router/cost`；按 model/layer/entrypoint/key/`local_user`/`serve_user` 分桶 |
| 10 | **api-keys** | Dashboard 签发 `sk-aria_`（`owner_user_id`）；`keys_path` 只存 sha256；数据面与 `PUT /providers` Bearer；`require_api_key` |
| 11 | **local-users** | `users_path` argon2；Register/Login session；`allow_register`；admin 管用户 |
| 12 | **oauth-account** | OAuth 为 `router-keys.json` 中 `kind: oauth` 条目；关联 ariacompute.com/cn；`sk-bf-` 存储/展示（Dashboard Account） |
| 13 | **stateful** | decision `emits`/`retention`；session sticky model（`x-aria-session` / body `session`） |
| 14 | **stores/services** | 进程内 `memory` / `semantic_cache`；软 `ratelimit`（429）；加厚 `router_replay`（含 reroute） |

### 2.1 非目标

- Envoy ExtProc、Operator、Helm、fleet-sim、Grafana / Prometheus、ML Setup、Security Policy、wizmap、独立 dashboard 端口 / 本地 OIDC/SSO（本地仅用户名+密码）
- Playground 不做 MCP / Claw / Web Search / 附件 / 语音 / Feedback（对齐 vLLM SR **核心**聊天体验即可）
- Hybrid 本增量用 `bfvk` 转发 gateway（仅存储与鉴权分桶）；邮箱验证码；自助注册升 admin
- **硬** quota（软限流 429 允许）、Slack 告警、把 Dashboard `sk-aria_` 当 HF/ModelScope token
- Vendoring / 子进程接入 Pi / DeepSeek Harness；YAML 自定义 tools
- 在 Rust 内重写 Cordis；`vector_store`、HaluGate、完整 SAAR / tool-loop hard lock

## 3. API 边界

### 3.1 配置（YAML v0.3）

```yaml
version: v0.3
listeners:
  - name: http-8899
    address: 0.0.0.0
    port: 8899
    timeout: 300s
providers:
  defaults:
    default_model: local/general
  models:
    - name: local/general
      provider_model_id: my-served-model
      locality: local          # local | private | cloud
      modality: text
      capabilities: [chat]
      backend_refs:
        - name: primary
          endpoint: 127.0.0.1:8000
          protocol: http
          weight: 100
      pricing:                    # 可选；USD / 百万 token
        input_per_mtok: 0.15
        output_per_mtok: 0.60
entrypoints:
  - model_names: [ariacompute/semantic-auto]
    router: semantic
    recipe: mom
  - model_names: [ariacompute/agent-auto]
    router: agent
    recipe: agent-default
  - model_names: [ariacompute/afm-d-auto]
    router: afm-d
    recipe: afm-d-default
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      signals: { keywords: [...] }
      decisions: [...]
  - name: agent-default
    router: agent
    agent:
      endpoint: ${ROUTER_LLM_URL:-}
      model: router-llm
      timeout_ms: 5000
      max_turns: 3
      fallback: local/general
  - name: afm-d-default
    router: afm-d
    afm-d:
      endpoint: "${DECISION_MODEL_URL:-http://127.0.0.1:8010}"
      track: encoder          # 可选；默认 encoder→Choice≤255（:8010）；decoder→≤16（:8011）
      timeout_ms: 5000
      fallback: local/general
      instructions: Pick the best model for this user request.
global:
  require_api_key: true           # default true；false → 数据面可不带 Bearer
  allow_register: true            # Dashboard 普通用户自助注册
  keys_path: ~/.ariacompute/router-keys.json   # keys[]：kind=local (sk-aria_) | kind=oauth (sk-bf-)
  users_path: ~/.ariacompute/router-users.json
  model_catalog:                  # 可选；learned / semantic_cache 权重或 hash 后端注册
    embed_hash: { kind: hash, dim: 64 }
  stores:
    memory: { enabled: true, max_sessions: 4096, default_ttl_turns: 2 }
    semantic_cache: { enabled: false, similarity_threshold: 0.92, max_entries: 1024, ttl_turns: 4 }
  services:
    ratelimit: { enabled: false, requests_per_minute: 120 }
    router_replay: { enabled: true, max_items: 256, persist_path: null }
```

密钥：`${VAR}` / `${VAR:-default}`。未知顶层块 → validate 失败。未知 `global` / `pricing` / `stores` / `services` 子键 → validate 失败。`backend_refs.api_key` 仅转发上游，与 Dashboard 签发的客户端 key **不是同一把**。本地 `sk-aria_` 与 OAuth `sk-bf-` 同文件 `keys[]`，以 `kind` 区分（无独立 `router-serve.json`）。

Decision 可选 `emits`（对齐 v0.3 Themis）：

```yaml
emits:
  - kind: retention
    retention:
      drop: false              # true → 跳过 semantic_cache 写入；与正 ttl_turns 互斥
      ttl_turns: 2
      keep_current_model: true # 同 session 后续 turn 粘住本决策模型（仍须 eligible）
      prefer_prefix_retention: false  # 仅透传响应头，本增量不实现 SAAR 计分
```

### 3.2 Semantic signals

**启发式（阶段 B，A 至少 `keyword`）**：`keyword`、`language`、`context`、`authz`、`conversation`、`metadata`、`event`、`structure`。

**Learned（阶段 C/R，Cargo feature `ml`）**：`classifier`、`complexity`、`domain`、`embedding`、`fact-check`、`jailbreak`、`kb`、`modality`、`pii`、`preference`、`reask`、`user-feedback`。

- 无 `ml` feature 且被 decision 引用 → `Unsupported`，禁止跳过。
- 有 `ml`：默认 **hash** 嵌入/打分后端（确定性，CI 友好）；YAML `weight_path` 指向 ONNX 时走 `ort`（可选）；也可 `model: <catalog_key>` 引用 `global.model_catalog`。
- Embedding：`threshold`、`candidates[]`、`aggregation_method`（mean/max/any）。
- 其余 learned：薄封装（description / threshold / patterns → hash 或分类器置信度）。

规则：`(type, name) → {match: bool, confidence: 0..1}`。仅计算被引用类型。

### 3.3 Projections / decisions / algorithms / plugins

- Projections：`partition` / `score` / `mapping`。
- Decision：Boolean `AND`/`OR`/`NOT` 树 + `priority` + `modelRefs` + 可选 algorithm/plugins + **`emits`**；未知 decision 键 → validate 失败（`deny_unknown_fields`）。
- Strategy：`priority`（默认）或 `confidence`。
- Selection：阶段 A `static`；B 增 `latency-aware`、`multi-factor`；**R 增 `elo`**（内存 Elo ratings 表，按成功/延迟反馈更新）；其余（`automix`、`hybrid`、`kmeans`、`knn`、`mlp`、`prompt`、`router-dc`、`svm`）未实现则 Unsupported。
- Looper：**R 实现 `ratings`**（读/写同一 Elo 表）；其余（`confidence`、`fusion`、`remom`、`workflows`）仍 Unsupported。
- Plugins：B 至少 `header-mutation`、`request-params`、`system-prompt`、`fast-response`、`response-cache`（exact）；C 其余引用即须实现或 Unsupported。

### 3.3.1 状态化路由（阶段 R）— ContextHold

- Session 键：`x-aria-session` 或 body `session`（与 cost 共用）。**仅显式 session** 参与 ContextHold；未提供时 cost 使用一次性 `req_*` id，不冻结决策、不写 sticky。
- **`prefix_reuse`（缺省 `true`；显式 `false` 关闭）**：同一显式 `(session, user 消息条数)` 内冻结 `RouteDecision`，工具往返不换模型；新 user 消息再决策。消息条数变少或 system / `tools` 指纹变化视为压缩，清除该 session 的决策缓存与 sticky。
- Semantic：命中 decision 且 `retention.keep_current_model` → `stores.memory` sticky；后续 turn 在 algorithm 后若 sticky 仍 eligible 则覆盖选型。同模型再 pin **不重置** TTL；TTL 仅在 **user 消息条数上升** 时 `-= 1`，到 0 清除。
- Agent / AFM-D：当 `stores.memory.enabled` 且 `prefix_reuse` 开启时，路由成功后 pin sticky；新 user turn 上若 sticky 仍 eligible 则短路（不调用 System One / builtin agent）。`get_request_view` 仍可读 session 摘要。
- `retention.drop`：禁止本响应写入 `semantic_cache`。
- 响应头：`x-aria-router-retention-drop` / `ttl-turns` / `keep-current-model` / `prefer-prefix`（显式设置才发）。`prefer_prefix_retention` 仍仅透传，不实现 SAAR 计分。
- Bypass 不写 retention。

### 3.3.2 生产 stores / services（阶段 R）

- `stores.memory`：进程内 session → sticky；`max_sessions` LRU。
- `stores.semantic_cache`：决策/模型感知；相似度 = embedding 余弦（无向量则 hash 文本）；lookup 在转发前；命中响应头 `x-aria-router-semantic-cache: hit`；与 plugin `response-cache`（exact）并存。
- `services.ratelimit`：按 API key id 或 `anonymous` 令牌桶；超限 **429**（软限流 ≠ 硬 quota）。
- `services.router_replay`：`ReplayRecord`（signals 摘要、projection、decision、emits、model、latency）；`GET /v1/router/replay`；`POST /v1/router/replay/{id}/reroute?forward=false` 默认仅重跑决策；可选 `persist_path` JSONL。

### 3.4 Agent（轻量 builtin）

`RouteDecision { model, algorithm?, reason, confidence }`。`model` ∈ 硬剪枝后 eligible pool。

进程内 OpenAI 兼容 chat + **固定** tool_calls 环（不可 YAML 扩展）：

| Tool | 作用 |
|------|------|
| `list_eligible_models` | 候选 name / locality / modality / capabilities |
| `get_backend_health` | 候选健康（failures 计数） |
| `get_recent_latency` | 近期延迟采样（若有） |
| `get_request_view` | 脱敏请求摘要 |
| `submit_route` | **终态**：`{model, reason, confidence?, algorithm?}` |

约束：`max_turns` 默认 3、clamp ≤8；`timeout_ms` 默认 5000（整段 loop）；无 `endpoint` → first-eligible / 测试 canned。超时 / 超 turns / 非法 JSON / 越权 → fail closed，或 recipe `fallback`（须为已声明 provider）。禁止 shell / 任意 HTTP / 文件 IO / 动态注册 tool。

### 3.4.1 AFM-D（System One 决策器）

`RouteDecision { model, algorithm?, reason, confidence }`。`model` ∈ 硬剪枝后 eligible pool。`layer` / `decision` = `"afm-d"`。

流程：硬剪枝 → **确定性档位短路**（优先于 System One；池中须有对应 `tier`）→（同 utterance×eligible 的进程内缓存命中 → `afm-d:cache:{model}`）→（`eligible.len()<=1` 或无 `endpoint` → first-eligible）→ 将候选编成单题 Choice → `POST {endpoint}/v1/systemone`：

短路优先级（对齐 `bench/corpus/routing_gateway_hard.json` 金标）：

| 顺序 | 条件 | tier | `reason` |
|------|------|------|----------|
| 1 | 裸问候/短寒暄 | small | `afm-d:chitchat-small` |
| 2 | explain / walk through / how does\|do | large | `afm-d:explain-large` |
| 3 | stand for / acronym / 常见缩写问句 | small | `afm-d:acronym-small` |
| 4 | reverse proxy / trade-off / eviction / proxy 选型 | mid | `afm-d:systems-mid` |
| 5 | 多问句（题干 `?`/`？` ≥ 2） | mid | `afm-d:multi-q-mid` |
| 6 | 封闭题（MCQ `A.`/`B.` 或 “Answer with the letter…” / “Answer concisely.”） | small | `afm-d:mcq-small` |
| 7 | 短事实（沸点、单位换算、短 what-is/which/is/name；按**题干首行**计词，忽略选项块） | small | `afm-d:factoid-small` |

```json
{
  "state": "User message: <last user text>",
  "questions": {
    "route": {
      "type": "choice",
      "instructions": "<afm-d.instructions>",
      "criteria": { "<model_name>": "<description>" }
    }
  }
}
```

- `state` 为纯文本 `User message: …`（不用 JSON blob；Encoder 对问候更稳）。
- `criteria` 描述：YAML `afm-d.descriptions` 覆盖优先，否则由 `tier` / `locality` / `capabilities` 合成；默认 instructions **宁小勿大**（large 仅明确教学）。
- 响应 `answers.route.choice` 须 ∈ eligible；`confidence` 写入 `RouteDecision`；`reason` = `afm-d:{choice}`；成功答案写入容量 256 的进程内 FIFO 缓存。
- `timeout_ms` 默认 5000；超时 / 非法响应 / 越权 choice → `fallback`（若配置且为已声明 provider）否则 FailClosed。
- `min_confidence`：若配置且响应 confidence 低于阈值 → 同 fallback / FailClosed。
- 可选 `afm-d.track: encoder | decoder`（与 aria-engine `--track` 对齐）。默认 / 未设 / `encoder` → eligible ≤255（`:8010`）；`decoder` → ≤16（validate：text providers 数；runtime FailClosed；`:8011`）。gateway 示例默认 `track: encoder` + `:8010`。
- **不做**：进程内 candle / 加载 `afm-de` 权重；Score/Noul 路由原语（首期仅 Choice）。

### 3.5 HTTP

**数据面**（listener）：

- `POST /v1/chat/completions` JSON + SSE
- `GET /v1/models`：entrypoint 虚拟名 + 实名 provider 名
- 响应头 `x-aria-router-layer`、`x-aria-router-decision`、`x-aria-router-model`；另可选 `x-aria-router-algorithm` / `x-aria-router-reason` / `x-aria-router-confidence` / `x-aria-router-bypass` / retention·semantic-cache 头
- SSE（`stream: true`）：上游 `bytes_stream` **透传**至客户端（`text/event-stream`）；流结束后再记 cost
- `global.require_api_key: true` 时须 `Authorization: Bearer` 或 `x-api-key` 命中未吊销 **本地** `sk-aria_` **或** 已配置的 OAuth `sk-bf-`，否则 **401**
- Cost 事件：`identity` = `local_user` | `local` | `serve` | `anonymous` | `playground`；报告含 `by_local_user` / `by_serve_user`

**管理面**（默认 `127.0.0.1:8080`）：

- `GET /health`
- Auth（公开）：`POST /v1/router/auth/register`、`POST /v1/router/auth/login`、`GET /v1/router/auth/register-status`；无用户且未 setup → register **503**
- Auth（会话）：`POST /v1/router/auth/logout`、`GET /v1/router/auth/me`、`POST /v1/router/auth/password`
- Users（admin）：`GET/POST /v1/router/users`、禁用/重置密码、`PUT /v1/router/settings/allow_register`
- 既有 validate/replay/config/overview/providers/topology/chat/cost/keys；`GET /v1/router/models`（与数据面 `/v1/models` 同源 JSON，供 Dashboard Playground）；keys 带 `owner_user_id`；用户非空时除公开 auth/health/OAuth callback 外须 session
- OAuth（公开登录）：`POST /v1/router/auth/oauth/start`（返回 serve authorize_url）→ `GET /v1/router/auth/oauth/callback`（换 code、upsert 绑定 serve 用户并签发本地会话；回跳仅 `127.0.0.1|localhost` loopback）。遗留 `/v1/router/serve/link/*` mgmt 端点与回调已删除；serve 账户（会话）：`GET /v1/router/serve/account`；`POST /v1/router/serve/account/sync` 自动从 serve 同步 `sk-bf-` 明文（serve `GET /api/api-keys` 现返回 secret，router 轮询写入 `kind:oauth` 记录的 `api_key`；新增 key 自动采用、401/403 或空列表 → 标记 `api_key_deleted`）。**手动粘贴入口 `POST /v1/router/serve/account/key` 已删除**——dashboard Account 页不再提供粘贴框，纯自动同步。
- `GET /` 与 SPA fallback → `dashboard/dist`

管理面默认只绑 `127.0.0.1`。本地密钥明文只在 POST 响应出现一次；`keys_path` 只存 sha256；密码 argon2id。

CLI 子命令：`setup` / `validate` / `serve` / `upgrade [version]` / `version`。CLI help 由 **clap** derive 生成（对齐 memo：`about` / `Usage` / `Commands` / `Options`；支持 `aria-router <cmd> --help`）。无参调用打印 help 并 exit **2**；`-v` / `--version` / 子命令 `version` 打印版本。

- `setup`：写入 `~/.ariacompute/router.yml`，starter = **semantic-gateway** / **agent-gateway** / **afm-d-gateway**（非 tiny）。交互：template → models（`base_url` / `api_key_env` / 三档 `provider_model_id`；agent 另 `endpoint`/`model`/`fallback`；afm-d 另 System One `endpoint`/`fallback`）→ admin → `upgrade_url`。逻辑名 `ariacompute/ariamodel-{small,mid,large}` 为配方槽位默认保留；`providers.models[].tier`（`small|mid|large`）供 agent / afm-d 认档，优先于名字启发式。默认 `allow_register=true`、`require_api_key=true`。flags：`--status` / `--clear` / `--template` / `--admin-user` / `--admin-password` / `--base-url` / `--api-key-env` / `--model-small|mid|large` / `--agent-endpoint|model|fallback` / `--afm-d-endpoint|fallback` / `--upgrade-url`。`--template`+admin 齐全且未传 model flags → 静默用 gateway 默认。**不**签发 `sk-aria_`、不跑 OAuth 浏览器。`--status` 扁平 `key: value`（含 `upgrade_url` 与 `lib`）。`--clear` 可删 `router-keys.json` / `router-users.json`，并清除 `router-cli.yml`。离线 CI 仍可用显式 `--config *-tiny.yaml`。换上游后 bench Track B 须自备 `--model-id` / `--pick-map` / prices。
- `upgrade [version]`：按 `~/.ariacompute/router-cli.yml` 的 `upgrade_url`（组织根；与配方 `router.yml` **分离**）拼 `{upgrade_url}/router`，调 GitHub/Gitee Releases API；默认最新**正式** Release（忽略 prerelease/draft），可选 `0.1.0` / `v0.1.0`；下载本机平台 `aria-router_*` + `libaria-router_ffi_*`，原地原子替换当前 CLI，并将 FFI 装入 `~/.ariacompute/lib/`（提示 `ARIA_ROUTER_FFI_LIB`）。默认 org：`.com`→`https://github.com/ariacompute`，`.cn`→`https://gitee.com/ariacompute`（`setup` 写入）。未配置 `upgrade_url` 时报错并提示先 `setup`；下载/解压失败不得损坏现有 CLI。

**与 engine**：单一 `router_api_key` 字段可传 `sk-aria_` 或 `sk-bf-`；router 按前缀解析 `keys[]`（`kind: local|oauth`）。

### 3.6 错误

`RouterError::{Io, Config, Unsupported, InvalidParam, FailClosed, Upstream, Timeout, Extension, Unauthorized, RateLimited}`（`Extension` 用于 builtin LLM/tool 路径故障；`RateLimited` → HTTP 429）。禁止 panic 当控制流。

### 3.7 FFI / SDK

C API（`include/aria_router.h`）：

| C API | 语义 |
|------|------|
| `aria_router_init(config_path)` | 进程内加载 YAML → opaque handle；`config_path` 可选（`NULL`/空串 → 默认 `~/.ariacompute/router.yml`） |
| `aria_router_connect(base_url)` | HTTP 连已运行网关 |
| `aria_router_complete` / `_stream` | chat；JSON 出参 / chunk 回调 |
| `aria_router_models` / `aria_router_last_route` | 模型列表；最近决策 JSON |
| `aria_router_destroy` / `aria_router_last_error` | 生命周期 / 错误 |

语言包：Python、Go、Rust（`ariacompute-router`）、Swift、Kotlin、Flutter、React Native（`@ariacompute/router-rn`）、TypeScript（`@ariacompute/router-ts`）。布局：`ffi/` + `bindings/<lang>/` + `bindings/testdata/`。

动态库：`ARIA_ROUTER_FFI_LIB` → 包内捆绑 → `~/.ariacompute/lib/`。Rust 原生不 dlopen。

实例 `setup`：仅内存 `base_url` / `token`；禁止写 `router.yml` / `engine.yml`。无 `auth` 别名。

测试：共享 `cases.json`（lifecycle / chat / stream / models / last_route / connect / fail-closed）；`cargo test -p ariacompute-router-ffi`；`./scripts/run-binding-tests.sh`。

发布：fail-pass 多 registry，不阻断 CLI。Release 资产：`aria-router_{ver}_{os}.tar.gz` / `.zip`（CLI）与 `libaria-router_ffi_{ver}_{os}.tar.gz`（FFI）。

## 4. 验收

- A-semantic：keyword 命中转发；实名 bypass；无路径 fail closed；SSE 至少 1 chunk。
- A-agent：`submit_route` / 合法终态采纳；工具结果正确；非法/越权/超时/超 `max_turns` fail closed；与 semantic 入口不串扰。
- A-afm-d：`afm-d-tiny` validate；无 endpoint → first-eligible；mock System One → choice 入 eligible；越权 / 低 confidence → fallback 或 FailClosed；`track: decoder` 时 text providers / eligible >16 → validate/FailClosed；与 semantic/agent 入口不串扰。
- B：启发式 + 三算法 + 五插件单测。
- C：无 `ml` 时 learned 被引用 → Unsupported；未知 algorithm 同。
- R：retention sticky 同 session 第二轮粘住模型；`semantic_cache` hit 头；ratelimit 超限 429；`elo` 选高分 modelRef；`cargo test` 默认绿；`cargo test -p aria-router-signal --features ml`（及 http `-F ml`）绿。
- E：八语言跑通 `cases.json` 黄金项。
- F：`PUT /v1/router/config` 非法 YAML 不改文档；合法 tiny YAML 热重载；topology 对 semantic-tiny / agent-tiny 有预期节点；`POST /v1/router/chat` 走 keyword / canned-agent 黄金路径；`GET /v1/router/models` 含 entrypoint；`stream:true` 返回 `text/event-stream` 且透传至少 1 chunk；响应含扩展 `x-aria-router-*`；Dashboard Playground：多轮气泡 + 模型下拉 + Header 面板 + markdown + 本地会话侧栏；`--no-dashboard` 时 `/` 不提供 SPA。
- G：带 `usage` 的 mock chat 计入账本；无 usage → estimate；无 pricing → `cost=0` 且 `priced=false`；`require_api_key: true` 无 Bearer 聊天与 PUT providers → 401；合法 key → 200 且 `by_key` 有 id；吊销后 401；Cost JSON 含六因子键。
- H：register→login；`allow_register=false` 拒注册；无用户 register→503；session 门控 keys；OAuth 为 keys `kind=oauth`；`sk-bf-` Bearer → `by_serve_user`；engine 单一 `router_api_key`（双前缀）。
- T（测试与覆盖率）：`cargo test --workspace --all-features` 全绿，每项功能含正常 + 异常路径用例；
  `scripts/coverage.sh` 门禁为 **100% 行覆盖**（`--workspace --all-features`，仅豁免 `build.rs`），
  口径为合并后的 lcov `DA:` 记录（见 `docs/coverage-exclusions.md` 的统计口径说明与豁免清单）；
  豁免项必须文件级、可审计并写明原因，禁止用 `#[cfg]` 散落屏蔽「凑覆盖率」；
  `.github/workflows/ci.yml` 在 `pull_request` 与 `push main` 上跑 fmt / clippy（`-D warnings`）/
  test（`--all-features`）/ coverage（复用同一脚本）/ bindings 五个 job 且全绿。

## 5. 目录

```
router/
  config/ signal/ decision/ algorithm/ plugin/ provider/ agent/ afm_d/ http/ bin/ ffi/
  dashboard/   # Vite React SPA；产物 dashboard/dist 由管理面托管
  bindings/{rust,python,go,typescript,react-native,flutter,swift,kotlin,testdata}/
  bench/       # Python report-only 路由 / DRACO 评测（§6）
  config/examples/
  AGENTS.md requirements.md task.md README.md Cargo.toml
```

## 6. 路由评测（`bench/`）

对齐 `engine/bench`：Python ≥3.10、标准库为主、**report-only**（报告恒含 `ci_fail: false`）、不启动 router / provider 进程；不可达 → `skipped`，进程 exit 0；坏 CLI → exit 2。

### 6.1 目标与非目标

**目标**

- **`routing`**（ADR-040）：对 corpus × pool 建 `(quality, tokens[, signal])` 矩阵；离线策略 `always_X` / `oracle_quality` / `oracle_cost_optimal(ε=0.03)` / 可选 `domain` / `knn`；**一个或多个** live router（如 `aria_router`、`vllm_sr`）取 pick；报告 quality、cost、$q/$、**% of oracle**。
- **`research`**（Perplexity DRACO 形）：任务 completion + rubric 四轴加权 MET/UNMET；对比 `always_X` 与各 live router。
- **`compare`**（对齐 [vLLM Semantic Router](https://github.com/vllm-project/semantic-router) 公开 bench）：MCQ **accuracy + E2E latency + tokens**（可选 USD via `prices.py`）；同语料并排 always_X 与各 router。
- **质量**（routing/research）`--quality label|overlap|judge`（Mode A 可离线 CI；Mode B 需 judge URL）。

**非目标**

- 不 vendoring 完整 HF DRACO / MMLU-Pro；`download-draco` / `download-mmlu` 失败则 skip。
- 不依赖 `vllm-semantic-router-bench` 包；不在 bench 内启 Envoy / `vllm-sr` / `cargo run` / mock upstream。
- 不因分数阈值让 CI 失败；不做 VSR Evaluation Plane / Dashboard 集成。
- 不做 metaharness 全量 deep-research fusion / embedding 训练管线。

### 6.2 多 router CLI

- `--router` 可重复：`NAME=URL`；裸 URL 兼容映射为 `aria_router`。
- `--entrypoint` 可重复：`NAME=MODEL`；裸字符串为默认 entrypoint（缺省 `ariacompute/semantic-auto`）。
- `--pick-header` 可重复：`NAME=HEADER`；`aria_router` 默认 `x-aria-router-model`；其它 router 默认仅 body `model`。
- `--pick-map FOREIGN=POOL_MODEL`：把外部分配的 model id 映射到 pool。
- `--api-key`：`ALIAS=KEY`；router 可用 `router`（单）或 `router_<NAME>` / `<NAME>`。

端口约定（文档）：共享 backend `:8000`/`:9001+`；aria-router `:8899`；vLLM Semantic Router `:8890`（避免抢默认 8899）。

### 6.3 CLI 示例

```bash
python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pool small=http://127.0.0.1:9001 --pool large=http://127.0.0.1:9002 \
  --model-id small=local/small --model-id large=local/large \
  --quality label \
  --corpus bench/corpus/routing_tiny.json \
  --report ./out/vs_vsr_routing.json

python -m bench research \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --pool large=http://127.0.0.1:9002 \
  --model-id large=local/large \
  --quality label \
  --corpus bench/corpus/research_tiny.jsonl \
  --report ./out/vs_vsr_research.json

python -m bench compare \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pool base=http://127.0.0.1:8000 \
  --model-id base=Qwen/Qwen3-0.6B \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --report ./out/vs_vsr_compare.json

python -m bench list-corpus
python -m bench download-draco --out ./out/draco_test.jsonl
python -m bench download-mmlu --out ./out/mmlu_pro.jsonl
```

公共：`--prices JSON`、`--api-key`、`--timeout`、`--max-tokens`、`--ref-model`（overlap）、`--judge-url` / `--judge-model` / `--judge-api-key`（judge）、`--skip-probe`。

### 6.4 字段契约

| 模式 | 语料 | 报告 `mode` | 核心产出 |
|------|------|-------------|----------|
| routing | JSON list：`{id, prompt, expected_model?, domain?}` | `router_routing` | cells + ladder（含各 live router）+ picks |
| research | JSONL：`{id, domain, problem, answer}`（tiny 可含 `expected_hits`） | `router_research` | 系统×域均值、四轴、相对 always 差 |
| compare | JSONL：`{id, question, choices?, answer, category?}` | `router_compare` | 每 system accuracy / latency p50·p95 / tokens；相对 best-always |

- **label** / **overlap** / **judge** / **rubric**：同阶段 K。
- Live router：按 NAME 请求对应 entrypoint；pick = 配置头优先，否则 body `model`；经 `--pick-map` 后仍 ∉ pool → `error` 行继续。
- **compare**：从 completion 抽选项字母 / Yes-No / 短答；`avg_cost_usd` 可选。
- Cost：`prices.py` USD/MTok；`cost = tokens/1e6 * rate`。

### 6.5 验收

1. `python -m unittest discover -s bench/tests -t .` 全绿（无外网；双 router mock；裸 `--router URL` 兼容）。
2. `routing` 双 router → ladder 同时含 `aria_router` 与 `vllm_sr`。
3. `compare` + `mmlu_tiny` → JSON+MD 含 accuracy / latency / tokens；缺 router 时该 system skip。
4. 缺 `--judge-url` 时 `judge` → 清晰 skip/error。
5. `cargo test` 不受影响（纯 Python 包）。

### 6.6 Gateway bench win vs vLLM SR（非对称）

**目标**：在 Track B 上让 **aria-router advanced Gateway recipe** 相对 **vLLM SR keyword baseline** 在成本、更难 corpus、compare MCQ、E2E 延迟与能力面上可证明领先。

**约定**
- aria：`config/examples/semantic-gateway.yaml` 可含投影入决策、`factoid_small`、pricing、修好的 `response-cache`、路由 `x-aria-router-latency-ms`。
- vllm-sr：`bench/vllm-sr/config-gateway.yaml` **冻结为 keyword 基线**（explain→large / systems_mid→mid / else→small），故意不对齐 aria 增量。
- Hard corpus：`bench/corpus/routing_gateway_hard.json`；价格表：`bench/prices/ariamodel.json`（`--prices`）。

**成功标准（报告-only，不令 CI 红）**
1. `routing` + hard corpus：`aria_router.mean_quality ≥ vllm_sr.mean_quality`，且 `aria_router.mean_cost_usd < vllm_sr.mean_cost_usd`（或同等 quality 下更高 `q_per_dollar`）。
2. `compare` Gateway 三档 + 双 router：`aria` accuracy ≥ `vllm_sr`，且 aria latency p50 ≤ vllm_sr。
3. 决策条件 `type: projection`（`name` + 可选 `equals`）参与 `select_decision`；单测覆盖 band/factoid。
4. `response-cache` 仅在决策插件启用时写入；cache key 使用选中模型 + 最近 user 文本。

### 6.7 Compare 热路径 vs vLLM SR（连接复用）

**目标**：在冷启动（无 response-cache）compare 上拉开 E2E p50/mean，超越 Envoy 数据面的连接优势差距。

**实现**
1. `PoolState` 持有共享 `reqwest::Client`（连接池 / keep-alive）；`forward` / `forward_sse_stream` 不得每次 `Client::new()`。
2. `algorithm: static` 决策跳过全量 `cost_map` / `latency_map` 构建（MCQ/factoid 默认路径）。
3. 公平测法：compare 前重启 aria 清 cache（见 `bench/MODEL_CHECKLIST.md`）。

**成功标准（报告-only）**
1. 冷启动 `compare`：`aria` accuracy ≥ `vllm_sr`，且 `aria` p50 **与 mean** 均 ≤ `vllm_sr`（目标 mean 差距显著大于仅 p50 的 ~70ms 噪声）。
2. `cargo test` 全绿；既有 gateway / provider 单测不回归。

### 6.8 Agent strategy Track B（非对称）

**目标**：评测 **aria builtin agent**（`ariacompute/agent-auto`）相对 **vLLM SR keyword baseline** 在 routing + compare 上的选档质量；与 §6.6 semantic 梯子正交（同端口 XOR serve）。

**约定**
- aria：`config/examples/agent-gateway.yaml`（TokenHub 三档 + agent LLM=mid）；entrypoint `ariacompute/agent-auto`。
- vllm-sr：`bench/vllm-sr/config-gateway.yaml` **同一冻结 keyword 基线**（`auto`）；不实现 vLLM agent 对等面。
- 语料复用：`routing_gateway_hard.json` / `mmlu_tiny.jsonl`；价格：`bench/prices/ariamodel.json` + `--pick-map`。
- 报告：`out/agent_vs_vsr_routing.{json,md}`、`out/agent_vs_vsr_compare.{json,md}`。

**成功标准（报告-only，不令 CI 红）**
1. `routing` + hard：`aria_router.mean_quality ≥ vllm_sr.mean_quality`；cost/q$ 必出报告，**不**强制 cost 胜（agent 多一轮 mid LLM）。
2. `compare` + `mmlu_tiny`：`aria` accuracy ≥ `vllm_sr`；p50/mean **报告**，**不**要求 ≤ vllm（tool-loop 结构性更慢）；aria `results_error` 不劣于 vllm。
3. Bench CLI 以 `--entrypoint aria_router=ariacompute/agent-auto` 指向 agent；单测覆盖 entrypoint 解析。

**CLI 摘要**
```bash
aria-router serve --config config/examples/agent-gateway.yaml \
  --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090
# vllm-sr keyword baseline :8890（同 §6.6）

python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/agent-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map ariacompute/ariamodel-small=glm-5.3-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=glm-5.3-flash --model-id mid=glm-5.3 --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json --quality label \
  --corpus bench/corpus/routing_gateway_hard.json \
  --timeout 300 --report ./out/agent_vs_vsr_routing.json
```

---

## 7. HiRoute 对标候选需求（候选 · 待人工审核 · 未排期 · 不进入实施）

> **状态声明（勿略过）**：本章是 [`docs/hiroute-vs-aria-router.md`](docs/hiroute-vs-aria-router.md) 差距分析的产物，**仅为候选输入**。
> 依本文件开头「须经人工逐项审核，审核通过后方可据其生成 / 执行 `task.md`」与 `AGENTS.md` 的 Harness 约定：
>
> - 本章**未经人审**，不构成实施依据；`task.md` 不得据此生成待办任务或排期；
> - 编号用 `C1–C9`（Candidate），**不占用、不重排** T0–T89；
> - 本章**不修改** §1 阶段表、§1.2 硬不变量、§2 功能边界、§2.1 非目标以及 §3–§6 的任何既有条款；
> - 「现状依据」为 2026-11-04 的源码核实结果，可能与 README 既有三方对比表的结论式判定不同，见 §7.4；
> - HiRoute 的准确率与成本数字均为其特定研究口径（事后复核子集、衡量策略符合度而非任务完成率、API 等价估算），见对比文档 §1.2，**一律不作为验收目标值**。

### 7.1 资料与编号

| 项 | 内容 |
|---|---|
| 主线 | 微信文章《HIROUTE 新版本发布：结合自己的真实工作选择最佳模型》 |
| 补齐 | HiRoute GitHub README、决策扩展 README、System One 论文与复现材料、智能模型路由指南、Code map（完整链接见对比文档 §8） |
| 对照 | 对比文档 §3 矩阵 #1–#39、§4 差异解读、§5 独有优势、§6 可借鉴方向 |
| 编号 | `C<n>` 在「对比文档 §6」「本章」「[`task.md`](task.md) 候选池」三处一一对应 |

### 7.2 候选条目

#### C1 — 显式化逐步重判边界

- **来源**：矩阵 #1；[W] 每步重新判断、[D3] 切换只发生在新消息 / 上下文压缩 / 重建。
- **现状依据**：`http/src/lib.rs:1328-1418` 的 `prefix_reuse` + session sticky（`x-aria-session` / body `session`）+ `retention.ttl_turns` 提供「粘住模型」，但无「该重新判断」与「沿用决策」的显式边界。
- **影响面**：`http` / `config`；需新增边界定义与可观测字段。
- **前置 / 风险**：§1.2「一次请求禁止串跑多种决策器」——重判只允许发生在**跨请求**边界，不得变成单请求内多决策器。
- **验收草案（待人审）**：给出新用户消息 / 上下文压缩 / 工具续跑三类边界的判定规则与响应头；正常用例覆盖冻结与重判，异常用例覆盖无法识别同 turn 时 fail closed。

#### C2 — afm-d 三问题模板

- **来源**：矩阵 #4、对比文档 §4.3；[W][D1] 三问（上一步胜任度 / 这一步难度 / 问题是否延续）。
- **现状依据**：engine 契约 `questions: Map<String, Value>` 已允许多键（工作区同级 `engine` 仓 `core/src/systemone.rs:11-15`，非本仓路径），但 `afm_d/src/lib.rs:560-597` 只发 1 个 `route` choice；`decision_from_systemone()`（`599-646`）只解析 `/answers/route.choice`；`track` 不进请求体，tier 快捷键与 256 条 FIFO 缓存会跳过 System One。
- **影响面**：`afm_d`（与 engine 契约对齐）；不改 C ABI。
- **前置 / 风险**：§1.2「检测 ≠ 执行」「Agent / AFM-D 输出必须是 typed `RouteDecision`」——三问答案须归一为单个 `RouteDecision`，并明确 `confidence` / `min_confidence` 语义。
- **验收草案（待人审）**：模板可选且默认保持单 `route` 问题，不破坏现有配置；正常用例覆盖三问归一，异常用例覆盖某问缺失 / 越权 choice / 超时时 fail closed 或走 `fallback`。

#### C3 — 低分保护状态机

- **来源**：矩阵 #7；[D1][D3] 评分须新鲜、完整、同类目、同计划版本、同评估标准；问题延续 < 0.5 解除保护，仅解除历史约束，难度仍决定组；未评分 ≠ 0 分，旧低分不重复生效。
- **现状依据**：`decision/src/lib.rs:121+` 的 `apply_competence()` 为一次性离散降档；`CompetenceTracker`（`core/src/lib.rs:377+`）；replay 的 `stage_competence` 生产恒 `None`。
- **影响面**：`decision` / `http` / `config` / `core`。
- **前置 / 风险**：依赖 C6 先接通评分来源；不得让保护覆盖 Location 类硬约束（§1.2）。
- **验收草案（待人审）**：保护触发与解除成对用例；旧低分不重复生效、部分评分不触发；无评分时维持现有 fail closed 行为。

#### C4 — 信任度连续调档 + 证据门槛 + 回放后发布

- **来源**：矩阵 #6 #13 #14；[W] 上调 / 下调门槛，[D3] 近 7 天已发布版本的历史分配估算（非预测）与 `publish` 冻结语义。
- **现状依据**：已有 `min_confidence`、`priority` / `confidence` 决策策略、`router_replay` 的 reroute（`http/src/lib.rs:266-320`，单轮 / `prompt_preview` 前 200 字 / entrypoint 取第一个）与 `PUT /v1/router/config` 热重载；无档位、无建议、无发布流程。
- **影响面**：`config` / `http` / `dashboard`；新增配置键须走 `deny_unknown_fields` 与 `validate`。
- **前置 / 风险**：发布流程会改变「配置即刻生效」的现有语义；建议只是建议，发布前不得改变路由。
- **验收草案（待人审）**：档位阈值可配、证据不足时不产生建议、回放只估算不改写线上路由、发布需显式动作；异常用例覆盖空 replay 与跨 entrypoint 记录。

#### C5 — Dashboard「模型表现」页

- **来源**：矩阵 #12；[W][D3] 阶段评分、新模型承担比例、费用估算与按分支 / 组 / 模型的分列。
- **现状依据**：Dashboard 已有 Cost / CostByCompetence 页；`http/src/cost.rs:168-245` 六因子账本与 6 个分桶；`/v1/router/cost/by-competence` 存在但生产证据字段恒 `None`。
- **影响面**：`dashboard` + 管理面 JSON API。
- **前置 / 风险**：依赖 C6；不得引入 Grafana / Prometheus（§2.1）。
- **验收草案（待人审）**：无证据时显示「无数据」而非 0；按 model / tier 分列；不改动既有 Cost 页口径。

#### C6 — 执行证据字段接通生产路径

- **来源**：矩阵 #11 #18；[D3] 会话路由时间线。
- **现状依据**：`http/src/replay.rs:32-48` 的 `task_category` / `tool_outcomes` / `accepted_output` / `stage_competence` 在生产路径恒 `None`（`http/src/lib.rs:1276-1280, 1698-1702`），仅单测赋值；`selected_group` 仅由模型名推 tier（`core/src/lib.rs:402`）。
- **影响面**：`http` / `core` / replay 存储（可选 `persist_path` JSONL 结构）。
- **前置 / 风险**：字段语义须先定义；不写入凭据与完整原始请求体，与现有 `prompt_preview` 截断惯例一致。
- **验收草案（待人审）**：生产路径可产生可查证据；关闭 replay 时无额外开销；异常用例覆盖工具调用失败与 SSE 中断时的字段状态；JSONL 向后兼容。

#### C7 — API Key 花费额度

- **来源**：矩阵 #26；[W] 一次性 / 按日历周期补充 / 多 Key 共享，转发前预留、完成后结算，按转发流量估算（非供应商余额），用完跳过该 Key。
- **现状依据**：`http/src/cost.rs:7` 内存账本 4096 事件上限、`by_key` 只记账；`http/src/keys.rs` 无 quota / spend / budget / credit / balance 字段；软 429 是速率限制（`http/src/ratelimit.rs`）。
- **影响面**：`keys` / `cost` / 数据面鉴权路径。
- **前置 / 风险**：**§2.1 明确「硬 quota（软限流 429 允许）」为非目标**，须先裁决是否调整边界；金额口径为转发流量估算。
- **验收草案（待人审）**：预留—结算闭环；额度耗尽时跳过该 Key 且返回可区分错误；SSE 流中断的结算补偿；额度数值不写入日志。

#### C8 — backend 候选链 / 同名模型 failover

- **来源**：矩阵 #9 #27；[D3] regular 候选耗尽 → 同类 primary relay，[W] 订阅 + API 同名模型自动接续。
- **现状依据**：`provider/src/lib.rs:260-265` 取权重最大的单个 backend，无 retry、无 backend 级 failover；全仓 `*.rs` 无 `failover`；`fallback` 仅决策器错误路径回退到单个已声明模型。
- **影响面**：`provider` / `config`（`backend_refs` 语义可能变化）。
- **前置 / 风险**：§1.2「无合格路径 fail closed」；须明确重试上限、幂等，以及响应开始后不切换模型。
- **验收草案（待人审）**：按序尝试与耗尽失败；异常用例覆盖首个 backend 超时 / 非 2xx / 流中途断开；不改变现有单 backend 配置的默认行为。

#### C9 — 策略一致性指标与离线复现

- **来源**：矩阵 #35–#38；[D2] 论文手稿、双语数据集、`reproduce.py`、冻结问题定义与局限声明。
- **现状依据**：`bench/cli.py:42-198` 共 8 个子命令；`bench/accuracy/runner.py:36-48` 有 `overall_accuracy` + per-tier recall + 混淆矩阵但需连 live router；`bench/corpus/README.md:3` 明示 bundled tiny fixtures、非官方 DRACO 或完整 MMLU-Pro；bench Python 中无等价 `consistency` 指标。
- **影响面**：`bench`（Python，report-only）；不改运行时。
- **前置 / 风险**：指标须声明为「符合声明策略」而非任务完成率；外部数据集下载失败沿用现有 skip 约定，不伪造分数。
- **验收草案（待人审）**：新增指标有单测（`python -m unittest discover -s bench/tests -t .`）；离线模式可对已保存决策记录复算；报告标注语料来源与排除清单。

### 7.3 需先行裁决的边界冲突

| 候选 | 冲突点 | 需裁决的问题 |
|---|---|---|
| C1 | §1.2「一次请求禁止串跑多种决策器」 | 重判仅指跨请求边界，是否需写入硬不变量说明 |
| C2 | §1.2「检测 ≠ 执行」「输出必须是 typed `RouteDecision`」 | 三问答案归一规则与 `min_confidence` 语义 |
| C3 / C4 | §1.2「Preference 不得覆盖 Location」 | 评分 / 信任度属 Preference，须保证不越过硬剪枝 |
| C7 | §2.1「硬 quota（软限流 429 允许）」为非目标 | 是否调整非目标边界，或仅做「记账不拦截」的软额度 |
| C8 | §1.2「无合格路径 fail closed」 | 按序重试与 fail closed 的边界 |

### 7.4 文档口径一致性待办（非候选需求）

[`docs/hiroute-vs-aria-router.md`](docs/hiroute-vs-aria-router.md) §7 登记了 `README.md` / `README_cn.md` 既有三方对比表中若干 ✅ 与源码状态不符的判定（硬约束剪枝范围、Bounded failover、执行证据留存链、长周期委托、Takeaway 的单向表述）。属**文档修订**，不入候选池、不在本章排期；本章按最小改动原则不修改 README 本身。

### 7.5 人审检查项

1. 逐条确认 C1–C9 是否在产品边界内，尤其 C7 与 §2.1 的冲突。
2. 确认 §1.2 硬不变量在任何候选下都不被破坏。
3. 确认 C6 为 C3 / C5 的前置，排序合理。
4. 确认 HiRoute 的准确率与成本数字未被写成验收目标值。
5. 审核通过后，另行按 Harness 流程生成 / 执行 `task.md`；在此之前本章保持「未排期」。

---

## 8. Router Web UI：impeccable 渲染审计与视觉优化（U1 · 计划稿 · 待人审激活）

> **状态声明**：本章是与 §7 **并列、互不隶属**的独立需求，编号 **U1**，不占用 `C1–C9`、不改动 §1–§7 任何条款。
> 与 [`docs/implementation-plan.md`](docs/implementation-plan.md) 的任务卡 T99 / 里程碑 M6、[`task.md`](task.md) 阶段 T 的 T99 三处对应，统一标注「**计划稿 · 待人审激活**」。
> 依本文件第 3 行与 `AGENTS.md` 的 Harness 约定，**本章仍须人工逐项审核**，审核通过前不进入编码、不成为已激活待办；§7 候选池的「未排期」状态不因本章变化。
> 参考实现来自工作区同级 `playground` / `serve` 仓的已落地 impeccable 工具链（只读参考，不修改）；本仓当前 **无任何 impeccable 文件**。

### 8.1 目标与定位

- 在 `dashboard/`（管理面同端口 React SPA）落地 **impeccable 4.8.0** 渲染审计工具链，按审计结论**完成视觉重做**，并建立回归护栏。
- 作用对象是 §2.1 / §1.1 阶段 F-dashboard 已交付的 SPA 页面与样式；**不改变 dashboard 功能行为**：API 契约、SSE 流式、路由响应头面板、localStorage 会话侧栏、`--no-dashboard` 语义均保持。
- 本节只定义需求边界与验收；可执行级的目标文件、单测用例、验证命令与合并顺序见 [`docs/implementation-plan.md`](docs/implementation-plan.md) §5 的 T99 任务卡。

### 8.2 工具链形态（实施阶段新增）

| 产物 | 内容 |
|------|------|
| `PRODUCT.md` | `impeccable:product-schema 1` 设计契约，**按 router 运维面板定位重写**，不照抄 playground 的四个 playground / 13 语言描述 |
| `dashboard/.impeccable/config.json` | 检测器引擎映射；playground 仅配 `.vue → html`，**本仓为 `.tsx`，支持情况须先核实**（§8.8） |
| `scripts/audit-ui.sh` | 五种模式 `--static` / `--dom` / `--url` / `--browser` / `--list-rules`；runner 按 **npm / node_modules** 适配（playground 用 bun，不可照抄） |
| `docs/impeccable-audit.md` | 基线记录：Tooling 表、Detection modes、findings → 修复 → 残余项、复跑命令 |
| `dashboard/tests/style.spec.*` / `render-snapshot.spec.*` | 静态样式护栏 + jsdom 渲染快照 |

- CLI 版本与环境变量口径对齐参考实现：`impeccable` 4.8.0、`IMPECCABLE_VERSION` / `IMPECCABLE_BIN` / `IMPECCABLE_BROWSER` / `PREVIEW_PORT` / `AUDIT_BASE_URL` / `AUDIT_ROUTES` / `AUDIT_THEMES` / `AUDIT_OUT_DIR`（`detect-out`）/ `AUDIT_PAGE_TIMEOUT`。
- `--static` 为源码级扫描（退出码 0/2，无需浏览器）；`--dom` 用 jsdom 渲染后 DOM 跑元素级规则（无需浏览器，看不到 layout）；`--url` 用真实 Chrome/Chromium/Edge 渲染态扫描（权威，需浏览器）。
- `detect-out/` 须加入 `.gitignore`。

### 8.3 页面矩阵（按本仓实际，不照抄 playground）

- **13 条路由**（`dashboard/src/App.tsx:166-181`）：`/login`、`/register`、`/`、`/account`、`/cost`、`/cost-competence`、`/keys`、`/users`（admin）、`/config`、`/topology`、`/providers`、`/replay`、`/playground`。
- **主题**：`light` / `dark`（`dashboard/src/theme.ts`、`components/ThemeSwitch.tsx`）。
- **无语言维度**：`dashboard/src` 无 i18n（仅 `Account.tsx` 的 `toLocaleString()` 用于日期格式化），文案硬编码英文；`dashboard/index.html:2` 的 `<html lang="en">` 属审计关注项。
- 审计矩阵 = 13 路由 × 双主题；**不得**引入 playground 的 5 路由 × 双主题 × 13 语言口径。

### 8.4 视觉重做范围

- 作用文件：4 个 CSS（`dashboard/src/index.css`、`App.module.css`、`pages/page.module.css`、`components/playground/playground.module.css`）、`App.tsx` 与相关 `pages/*` / `components/*`、`dashboard/index.html`、`dashboard/vite.config.ts`。
- 设计契约方向（以 `PRODUCT.md` 定稿为准）：单色、直角、功能文案字号下限、WCAG AA 对比度、禁渐变与不一致圆角、动效只用 transform/opacity、本地字体栈替代外部 CDN。
- 已可预见的审计 findings：`dashboard/index.html:7-12` 引用外部 Google Fonts CDN、`:2` 硬编码 `lang="en"`，处理结论须写入 `docs/impeccable-audit.md`。
- **功能等价是硬约束**：视觉重做不得改动 `dashboard/src/api.ts` 的请求契约与 `x-aria-router-*` 响应头展示逻辑。

### 8.5 测试护栏与 CI

- `dashboard/package.json` 现仅 `dev` / `build` / `preview`，**无 test / lint / a11y 脚本、无 vitest / jsdom / testing-library**；须新增 `test`、`audit`、`audit:dom`、`audit:url`、`audit:browser`、`audit:rules` 与相应 devDependency（**需 G11 裁决**）。
- 护栏覆盖：字号下限、对比度、`radius`、禁渐变 / uppercase / letter-spacing、动效属性白名单；快照覆盖 13 路由 × 双主题。
- **CI 现状**：`.github/workflows/` 无任何 dashboard 构建 / 检查 job；dashboard 目前仅由 `http/build.rs:5,74-77` 隐式触发 `npm --prefix dashboard run build`，且失败仅告警（CI 可能静默跳过）。本需求须新增显式 job，把 `npm --prefix dashboard ci && npm --prefix dashboard run build` 与护栏测试设为必过。

### 8.6 验收

**正常路径**

1. `npm --prefix dashboard ci && npm --prefix dashboard run build` 与新增 `test` 全绿。
2. `./scripts/audit-ui.sh --static` 0 primary findings；`--dom` 全路由 × 双主题 0 primary findings；`--url` 在有 Chrome 的机器执行并把结果写入 `docs/impeccable-audit.md`。
3. 视觉重做后 13 个页面在双主题下功能等价，API 契约与 SSE 行为不变。

**异常路径**

1. `audit-ui.sh` 未知参数报错退出；`--browser` 找不到浏览器时退出 1 并给安装提示；`--static` 退出码 0/2；`--dom` 无浏览器可运行；`PREVIEW_PORT` 占用时拒绝复用；扫描超时有明确日志。
2. CI 无浏览器时 `--url` 记录 `skipped`，**不伪造成功**。
3. 样式护栏任一 token / 字号 / 效果回归即失败；快照缺失或为空即失败。

### 8.7 与 §7 候选池及 C5 的关系

- U1 **不占用 `C` 编号**，不进入 §7 候选池；§7 的 `C1–C9`、状态声明与「未排期」标注一字不改。
- U1 与 C4 / C5 共改 `dashboard/src/App.tsx` / `api.ts`，与 C5 在 4 个 CSS 上重叠最大。**首选把 C4/C5 的 dashboard 部分与 U1 合并为同一批次**；若分批，则 U1 的设计 token / 工具链 / CI 骨架先行，再由 C4 → C5 在新设计系统上加页面，避免 C5 新页 CSS 被 U1 重做推翻。详见 `docs/implementation-plan.md` §3、§4。
- U1 不依赖 C1–C9 的 Rust 运行时改动；可与 M2 的低耦合任务并行。

### 8.8 风险与待核实项（人审时逐项确认）

| # | 待核实 / 风险 | 处理 |
|---|---------------|------|
| 1 | **impeccable 静态引擎对 `.tsx`/JSX 的支持未核实**（参考实现只映射 `.vue → html`） | 列为 U1 首个调查项；降级方案：静态扫描限定 `.css` + `index.html`，元素级规则交给 `--dom` / `--url` |
| 2 | 新增前端 devDependency 影响 `npm ci` 与 CI 时间 | G11 裁决；不允许则 U1 降级为「一次性检查 + 最小修复」并重新入审 |
| 3 | `--url` 依赖真实浏览器 | CI 无浏览器按 `skipped` 记录，不伪造成功 |
| 4 | 设计契约须按 router 运维面板定位重写 | 不照抄 playground 的产品描述 |
| 5 | CI 当前无 dashboard job、构建失败仅告警 | 新增显式必过 job |
| 6 | `detect-out` 未 ignore、外部字体 CDN 与 `lang` 硬编码 | 与审计 findings 一并处理 |

### 8.9 非范围（本次交付）

本需求章节、任务卡 T99 与 `task.md` T99 均为**文档计划**。以下属人审通过后的实施阶段，**本次交付不创建、不安装、不修改**：`PRODUCT.md`、`scripts/audit-ui.sh`、`dashboard/.impeccable/config.json`、`docs/impeccable-audit.md`、`dashboard/tests/*`、`dashboard/package.json` 的依赖与 scripts、4 个 CSS 与组件、`.github/workflows/ci.yml`、`.gitignore`。
