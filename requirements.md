# requirements.md — aria router（Rust）

> 本文件为 `router` 仓库 **Semantic + 轻量 Builtin Agent 并列网关 / OpenAI 兼容 HTTP / 八语言 SDK** 的功能边界、API、配置、异常与验收标准。**须经人工逐项审核**，审核通过后方可据其生成 / 执行 `task.md`。
>
> 架构参考：[vLLM Semantic Router](https://github.com/vllm-project/semantic-router) YAML v0.3（运行时对等，不做 Operator/Envoy）；运维 Dashboard 对齐其 dashboard 的 Config / Topology / Playground / Replay，不接 Grafana / ML / Security。Agent 面为 **进程内** 定制 builtin（少量固定工具 + 限 turns），不做 pi / deepseek-harness 子进程。

## 1. 目标与范围

用 **Rust** 实现独立 OpenAI 兼容网关：按 entrypoint 选择 **semantic** 或 **agent**（builtin）决策器，在硬约束剪枝后选择或组合 provider 路径并转发。

- **产品面**：`aria-router` CLI（`setup` / `validate` / `serve`）+ 进程内库 + C ABI + 八语言 SDK。
- **两种 router**：`semantic`（signals → Boolean recipe → algorithm）与 `agent`（轻量进程内 builtin：固定工具 + `max_turns`）并列；共享 listeners / providers / 硬约束 / 转发 / replay。
- **不做**：Operator、Helm、官网、Python `vllm-sr`、Envoy ExtProc、Grafana / Prometheus、ML wizard、Security Policy、wizmap、fleet-sim、pi / deepseek-harness / 可插拔 `extensions`、把 TS harness 链进 crate。
- **与 engine**：engine 仅本地推理；可选向本网关注册为 provider。本仓 SDK 与 `ariacompute-engine` **两套包**，`.so` 名互不覆盖。

### 1.1 阶段

| 阶段 | 名称 | 交付深度 |
|------|------|----------|
| **A-semantic** | 黄金路径 | YAML 加载、keyword、Boolean、static、chat/SSE、bypass、fail closed |
| **A-agent** | 并列黄金路径 | 进程内 builtin tool-loop、硬剪枝、越权拒绝、与 semantic 入口隔离 |
| **B** | 启发式管线 | 启发式 signals、projections、latency-aware / multi-factor、核心 plugins |
| **C** | 运行时对等 | learned signals（ONNX feature `ml`）、剩余 algorithm/looper/plugin；未实现显式 Unsupported |
| **E** | SDK | C ABI + 八语言；`cases.json`；`run-binding-tests.sh` |
| **F-dashboard** | 运维面 | 管理面 SPA：Overview / Config（可写热重载）/ Topology / Providers / Replay / Playground（多轮聊天气泡 + SSE + 路由 Header 面板；对齐 vLLM SR 核心体验，不做 MCP/Claw/联网/附件） |
| **G-cost** | 成本 + API key | 六因子成本账本；YAML `pricing`；Dashboard「API 密钥」签发；数据面 / provider 注册 Bearer；engine 传 `router_api_key` |
| **H-accounts** | 本地用户 + OAuth | Dashboard 注册/登录（密码）；本地 `sk-aria_` 归属用户；OAuth（Aria Compute）关联 + `sk-bf-`；cost `by_local_user` / `by_serve_user`；CLI 扁平 setup |
| **R-stateful-prod** | 状态化 + 生产 A + 决策器 B | `emits`/`retention`；`global.stores`/`services`；feature `ml` learned；Elo/`ratings`；semantic_cache；软限流；加厚 replay |

未达阶段的 YAML 能力须 `Unsupported`，禁止静默空实现。

### 1.2 硬不变量

- **Location / auth / modality / tools**：硬剪枝在决策之前；agent 只看见合格候选；无合格路径 fail closed。
- **Compute** 进 pool 排名/工具结果，不进 eligibility。
- **Preference**：semantic 用 signal；agent 用 prompt/tools 上下文；不得覆盖 Location。
- 检测 ≠ 执行。Agent 输出必须是 typed `RouteDecision`。
- 一次请求禁止串跑两种决策器。实名模型 **bypass** recipe。
- `entrypoint.router` 必须等于 `recipe.router`。Semantic recipe 禁止 `agent:`；agent recipe 禁止 `signals`/`decisions`。

## 2. 功能边界

| # | 特性 | 实现深度 |
|---|------|----------|
| 1 | **config** | YAML v0.3 结构：`version` / `listeners` / `providers` / `entrypoints` / `recipes` / `global`；`${VAR}` 替换；引用校验；**无**顶层 `extensions` |
| 2 | **semantic** | signals、projections、Boolean AST、priority/confidence、algorithms、plugins |
| 3 | **agent** | 进程内 `BuiltinAgent`；固定工具 + `max_turns` / `timeout_ms`；schema 校验 |
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

### 3.3.1 状态化路由（阶段 R）

- Session 键：`x-aria-session` 或 body `session`（与 cost 共用）。
- 命中 decision 且 `retention.keep_current_model`：写入 `stores.memory` sticky model + `ttl_turns`；后续 turn 在 algorithm 前若 sticky 仍 eligible 则覆盖选型；每成功转发 `ttl_turns_left -= 1`，到 0 清除。
- `retention.drop`：禁止本响应写入 `semantic_cache`。
- 响应头：`x-aria-router-retention-drop` / `ttl-turns` / `keep-current-model` / `prefer-prefix`（显式设置才发）。
- Bypass 不写 retention；agent 路径不强制 sticky（仅 `get_request_view` 可读 session 摘要）。

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

CLI：`aria-router setup` 写入 `~/.ariacompute/router.yml`，starter = **semantic-gateway** / **agent-gateway**（非 tiny）。交互：template → models（`base_url` / `api_key_env` / 三档 `provider_model_id`；agent 另 `endpoint`/`model`/`fallback`）→ admin。逻辑名 `ariacompute/ariamodel-{small,mid,large}` 为配方槽位默认保留；`providers.models[].tier`（`small|mid|large`）供 agent 认档，优先于名字启发式。默认 `allow_register=true`、`require_api_key=true`。flags：`--status` / `--clear` / `--template` / `--admin-user` / `--admin-password` / `--base-url` / `--api-key-env` / `--model-small|mid|large` / `--agent-endpoint|model|fallback`。`--template`+admin 齐全且未传 model flags → 静默用 gateway 默认。**不**签发 `sk-aria_`、不跑 OAuth 浏览器。`--status` 扁平 `key: value`。`--clear` 可删 `router-keys.json` / `router-users.json`。CLI help 由 **clap** derive 生成（对齐 memo：`about` / `Usage` / `Commands` / `Options`；支持 `aria-router <cmd> --help`）。无参调用打印 help 并 exit **2**；`-v` / `--version` / 子命令 `version` 打印版本。离线 CI 仍可用显式 `--config *-tiny.yaml`。换上游后 bench Track B 须自备 `--model-id` / `--pick-map` / prices。

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
- B：启发式 + 三算法 + 五插件单测。
- C：无 `ml` 时 learned 被引用 → Unsupported；未知 algorithm 同。
- R：retention sticky 同 session 第二轮粘住模型；`semantic_cache` hit 头；ratelimit 超限 429；`elo` 选高分 modelRef；`cargo test` 默认绿；`cargo test -p aria-router-signal --features ml`（及 http `-F ml`）绿。
- E：八语言跑通 `cases.json` 黄金项。
- F：`PUT /v1/router/config` 非法 YAML 不改文档；合法 tiny YAML 热重载；topology 对 semantic-tiny / agent-tiny 有预期节点；`POST /v1/router/chat` 走 keyword / canned-agent 黄金路径；`GET /v1/router/models` 含 entrypoint；`stream:true` 返回 `text/event-stream` 且透传至少 1 chunk；响应含扩展 `x-aria-router-*`；Dashboard Playground：多轮气泡 + 模型下拉 + Header 面板 + markdown + 本地会话侧栏；`--no-dashboard` 时 `/` 不提供 SPA。
- G：带 `usage` 的 mock chat 计入账本；无 usage → estimate；无 pricing → `cost=0` 且 `priced=false`；`require_api_key: true` 无 Bearer 聊天与 PUT providers → 401；合法 key → 200 且 `by_key` 有 id；吊销后 401；Cost JSON 含六因子键。
- H：register→login；`allow_register=false` 拒注册；无用户 register→503；session 门控 keys；OAuth 为 keys `kind=oauth`；`sk-bf-` Bearer → `by_serve_user`；engine 单一 `router_api_key`（双前缀）。

## 5. 目录

```
router/
  config/ signal/ decision/ algorithm/ plugin/ provider/ agent/ http/ bin/ ffi/
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
  --pick-map ariacompute/ariamodel-small=qwen3.5-flash \
  --pick-map ariacompute/ariamodel-mid=glm-5.3 \
  --pick-map ariacompute/ariamodel-large=deepseek-v4-pro \
  --pool small=https://tokenhub.tencentmaas.com \
  --pool mid=https://tokenhub.tencentmaas.com \
  --pool large=https://tokenhub.tencentmaas.com \
  --model-id small=qwen3.5-flash --model-id mid=glm-5.3 --model-id large=deepseek-v4-pro \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json --quality label \
  --corpus bench/corpus/routing_gateway_hard.json \
  --timeout 300 --report ./out/agent_vs_vsr_routing.json
```
