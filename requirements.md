# requirements.md — aria router（Rust）

> 本文件为 `router` 仓库 **Semantic + Agent 并列网关 / OpenAI 兼容 HTTP / 八语言 SDK** 的功能边界、API、配置、异常与验收标准。**须经人工逐项审核**，审核通过后方可据其生成 / 执行 `task.md`。
>
> 架构参考：[vLLM Semantic Router](https://github.com/vllm-project/semantic-router) YAML v0.3（运行时对等，不做 Operator/Envoy）；运维 Dashboard 对齐其 dashboard 的 Config / Topology / Playground / Replay，不接 Grafana / ML / Security。agent 面参考 [earendil-works/pi](https://github.com/earendil-works/pi) JSONL RPC 与 [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 进程接入。

## 1. 目标与范围

用 **Rust** 实现独立 OpenAI 兼容网关：按 entrypoint 选择 **semantic** 或 **agent** 决策器，在硬约束剪枝后选择或组合 provider 路径并转发。

- **产品面**：`aria-router` CLI（`setup` / `validate` / `serve`）+ 进程内库 + C ABI + 八语言 SDK。
- **两种 router**：`semantic`（signals → Boolean recipe → algorithm）与 `agent`（LLM agent + extensions）并列；共享 listeners / providers / 硬约束 / 转发 / replay。
- **不做**：Operator、Helm、官网、Python `vllm-sr`、Envoy ExtProc、Grafana / Prometheus、ML wizard、Security Policy、wizmap、fleet-sim、把 TS harness 链进 crate。
- **与 engine**：engine 仅本地推理；可选向本网关注册为 provider。本仓 SDK 与 `ariacompute-engine` **两套包**，`.so` 名互不覆盖。

### 1.1 阶段

| 阶段 | 名称 | 交付深度 |
|------|------|----------|
| **A-semantic** | 黄金路径 | YAML 加载、keyword、Boolean、static、chat/SSE、bypass、fail closed |
| **A-agent** | 并列黄金路径 | `builtin` tool-call JSON、硬剪枝、越权拒绝、与 semantic 入口隔离 |
| **B** | 启发式管线 | 启发式 signals、projections、latency-aware / multi-factor、核心 plugins |
| **C** | 运行时对等 | learned signals（ONNX feature `ml`）、剩余 algorithm/looper/plugin；未实现显式 Unsupported |
| **D** | 第三方 extensions | `pi` JSONL RPC、`deepseek-harness` 进程；CI 用 mock command |
| **E** | SDK | C ABI + 八语言；`cases.json`；`run-binding-tests.sh` |
| **F-dashboard** | 运维面 | 管理面 SPA：Overview / Config（可写热重载）/ Topology / Providers / Replay / Playground |
| **G-cost** | 成本 + API key | 六因子成本账本；YAML `pricing`；Dashboard「API 密钥」签发；数据面 / provider 注册 Bearer；engine 传 `router_api_key` |
| **H-accounts** | 本地用户 + OAuth | Dashboard 注册/登录（密码）；本地 `sk-aria_` 归属用户；OAuth（Aria Compute）关联 + `sk-bf-`；cost `by_local_user` / `by_serve_user`；CLI 扁平 setup |

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
| 1 | **config** | YAML v0.3 结构：`version` / `listeners` / `providers` / `extensions` / `entrypoints` / `recipes` / `global`；`${VAR}` 替换；引用校验 |
| 2 | **semantic** | signals、projections、Boolean AST、priority/confidence、algorithms、plugins |
| 3 | **agent** | `AgentExtension`；`builtin` / `pi` / `deepseek-harness`；schema 校验；timeout / max_turns |
| 4 | **provider** | OpenAI 兼容转发、加权 `backend_refs`、health、latency 采样 |
| 5 | **http** | 数据面 `:8899` chat/SSE/`/v1/models`；管理面默认 `127.0.0.1` health/validate/replay/providers + Dashboard API |
| 6 | **ffi** | `libaria-router_ffi`：init/connect/complete/stream/models/last_route |
| 7 | **bindings** | rust/python/go/typescript/react-native/flutter/swift/kotlin |
| 8 | **dashboard** | 管理面同端口 SPA（`dashboard/`）；`--no-dashboard` 仅 JSON API；Cost / API 密钥页 |
| 9 | **cost** | 内存六因子账本；`GET /v1/router/cost`；按 model/layer/entrypoint/key/`local_user`/`serve_user` 分桶 |
| 10 | **api-keys** | Dashboard 签发 `sk-aria_`（`owner_user_id`）；`keys_path` 只存 sha256；数据面与 `PUT /providers` Bearer；`require_api_key` |
| 11 | **local-users** | `users_path` argon2；Register/Login session；`allow_register`；admin 管用户 |
| 12 | **oauth-account** | OAuth 为 `router-keys.json` 中 `kind: oauth` 条目；关联 ariacompute.com/cn；`sk-bf-` 存储/展示（Dashboard Account） |

### 2.1 非目标

- Envoy ExtProc、Operator、Helm、fleet-sim、Grafana / Prometheus、ML Setup、Security Policy、wizmap、独立 dashboard 端口 / 本地 OIDC/SSO（本地仅用户名+密码）
- Hybrid 本增量用 `bfvk` 转发 gateway（仅存储与鉴权分桶）；邮箱验证码；自助注册升 admin
- 硬 quota、Slack 告警、把 Dashboard `sk-aria_` 当 HF/ModelScope token
- Vendoring Pi / DeepSeek Harness 源码
- 在 Rust 内重写 Cordis

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
extensions: []
entrypoints:
  - model_names: [aria/semantic-auto]
    router: semantic
    recipe: mom
recipes:
  - name: mom
    router: semantic
    routing:
      strategy: priority
      signals: { keywords: [...] }
      decisions: [...]
global:
  require_api_key: true           # default true；false → 数据面可不带 Bearer
  allow_register: true            # Dashboard 普通用户自助注册
  keys_path: ~/.ariacompute/router-keys.json   # keys[]：kind=local (sk-aria_) | kind=oauth (sk-bf-)
  users_path: ~/.ariacompute/router-users.json
```

密钥：`${VAR}` / `${VAR:-default}`。未知顶层块 → validate 失败。未知 `global` / `pricing` 子键 → validate 失败。`backend_refs.api_key` 仅转发上游，与 Dashboard 签发的客户端 key **不是同一把**。本地 `sk-aria_` 与 OAuth `sk-bf-` 同文件 `keys[]`，以 `kind` 区分（无独立 `router-serve.json`）。

### 3.2 Semantic signals

**启发式（阶段 B，A 至少 `keyword`）**：`keyword`、`language`、`context`、`authz`、`conversation`、`metadata`、`event`、`structure`。

**Learned（阶段 C，feature `ml` / ONNX `ort`）**：`classifier`、`complexity`、`domain`、`embedding`、`fact-check`、`jailbreak`、`kb`、`modality`、`pii`、`preference`、`reask`、`user-feedback`。无权重且被 decision 引用 → `Unsupported`，禁止跳过。

规则：`(type, name) → {match: bool, confidence: 0..1}`。仅计算被引用类型。

### 3.3 Projections / decisions / algorithms / plugins

- Projections：`partition` / `score` / `mapping`。
- Decision：Boolean `AND`/`OR`/`NOT` 树 + `priority` + `modelRefs` + 可选 algorithm/plugins。
- Strategy：`priority`（默认）或 `confidence`。
- Selection：阶段 A `static`；B 增 `latency-aware`、`multi-factor`；C 其余（`automix`、`hybrid`、`kmeans`、`knn`、`mlp`、`prompt`、`router-dc`、`svm`）未实现则 Unsupported。
- Looper（C）：`confidence`、`fusion`、`ratings`、`remom`、`workflows`。
- Plugins：B 至少 `header-mutation`、`request-params`、`system-prompt`、`fast-response`、`response-cache`（exact）；C 其余引用即须实现或 Unsupported。

### 3.4 Agent

`RouteDecision { model, algorithm?, reason, confidence }`。`model` ∈ eligible pool。

Tools（Rust 实现）：`list_eligible_models`、`get_backend_health`、`get_recent_latency`、`get_request_view`（脱敏）。

Extensions：

| type | 接入 | 阶段 |
|------|------|------|
| `builtin` | 进程内对 OpenAI 兼容路由模型 tool-call；单测可注入假客户端 | A-agent |
| `pi` | subprocess `pi --mode rpc` JSONL | D |
| `deepseek-harness` | subprocess `command` + `workdir`；IPC = stdin JSON 一行决策（与 mock 一致） | D |

未知 type → validate 失败。缺二进制 → serve 失败。超时/非 JSON/越权 → fail closed 或 recipe `fallback`（须为已声明 default）。

移动 / 无 subprocess：`pi`/`deepseek-harness` → `Unsupported`。

### 3.5 HTTP

**数据面**（listener）：

- `POST /v1/chat/completions` JSON + SSE
- `GET /v1/models`：entrypoint 虚拟名 + 实名 provider 名
- 响应头 `x-aria-router-layer`、`x-aria-router-decision`、`x-aria-router-model`
- `global.require_api_key: true` 时须 `Authorization: Bearer` 或 `x-api-key` 命中未吊销 **本地** `sk-aria_` **或** 已配置的 OAuth `sk-bf-`，否则 **401**
- Cost 事件：`identity` = `local_user` | `local` | `serve` | `anonymous` | `playground`；报告含 `by_local_user` / `by_serve_user`

**管理面**（默认 `127.0.0.1:8080`）：

- `GET /health`
- Auth（公开）：`POST /v1/router/auth/register`、`POST /v1/router/auth/login`、`GET /v1/router/auth/register-status`；无用户且未 setup → register **503**
- Auth（会话）：`POST /v1/router/auth/logout`、`GET /v1/router/auth/me`、`POST /v1/router/auth/password`
- Users（admin）：`GET/POST /v1/router/users`、禁用/重置密码、`PUT /v1/router/settings/allow_register`
- 既有 validate/replay/config/overview/providers/topology/chat/cost/keys；keys 带 `owner_user_id`；用户非空时除公开 auth/health/OAuth callback 外须 session
- OAuth（公开登录）：`POST /v1/router/auth/oauth/start`（返回 serve authorize_url）→ `GET /v1/router/auth/oauth/callback`（换 code、upsert 绑定 serve 用户并签发本地会话；回跳仅 `127.0.0.1|localhost` loopback）。遗留 `/v1/router/serve/link/*` mgmt 端点与回调已删除；serve 账户（会话）：`GET /v1/router/serve/account`
- `GET /` 与 SPA fallback → `dashboard/dist`

管理面默认只绑 `127.0.0.1`。本地密钥明文只在 POST 响应出现一次；`keys_path` 只存 sha256；密码 argon2id。

CLI：`aria-router setup` 仅 template + admin（默认 `allow_register=true`、`require_api_key=true`；OAuth/`sk-bf-` 与开关改 YAML 或 Dashboard）。flags：`--status` / `--clear` / `--template` / `--admin-user` / `--admin-password`。**不**签发 `sk-aria_`、不跑 OAuth 浏览器。`--status` 扁平 `key: value`。`--clear` 可删 `router-keys.json` / `router-users.json`。CLI help 由 **clap** derive 生成（对齐 memo：`about` / `Usage` / `Commands` / `Options`；支持 `aria-router <cmd> --help`）。无参调用打印 help 并 exit **2**；`-v` / `--version` / 子命令 `version` 打印版本。

**与 engine**：单一 `router_api_key` 字段可传 `sk-aria_` 或 `sk-bf-`；router 按前缀解析 `keys[]`（`kind: local|oauth`）。

### 3.6 错误

`RouterError::{Io, Config, Unsupported, InvalidParam, FailClosed, Upstream, Timeout, Extension, Unauthorized}`。禁止 panic 当控制流。

### 3.7 FFI / SDK

C API（`include/aria_router.h`）：

| C API | 语义 |
|------|------|
| `aria_router_init(config_path)` | 进程内加载 YAML → opaque handle |
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
- A-agent：合法 JSON 采纳；非法/越权/超时 fail closed；与 semantic 入口不串扰。
- B：启发式 + 三算法 + 五插件单测。
- C：learned 无权重且被引用 → Unsupported；未知 algorithm 同。
- D：pi/dsh mock command 产出决策；缺 command 启动失败。
- E：八语言跑通 `cases.json` 黄金项。
- F：`PUT /v1/router/config` 非法 YAML 不改文档；合法 tiny YAML 热重载；topology 对 semantic-tiny / agent-tiny 有预期节点；`POST /v1/router/chat` 走 keyword / fake-agent 黄金路径；`--no-dashboard` 时 `/` 不提供 SPA。
- G：带 `usage` 的 mock chat 计入账本；无 usage → estimate；无 pricing → `cost=0` 且 `priced=false`；`require_api_key: true` 无 Bearer 聊天与 PUT providers → 401；合法 key → 200 且 `by_key` 有 id；吊销后 401；Cost JSON 含六因子键。
- H：register→login；`allow_register=false` 拒注册；无用户 register→503；session 门控 keys；OAuth 为 keys `kind=oauth`；`sk-bf-` Bearer → `by_serve_user`；engine 单一 `router_api_key`（双前缀）。

## 5. 目录

```
router/
  config/ signal/ decision/ algorithm/ plugin/ provider/ agent/ ext/ http/ bin/ ffi/
  dashboard/   # Vite React SPA；产物 dashboard/dist 由管理面托管
  bindings/{rust,python,go,typescript,react-native,flutter,swift,kotlin,testdata}/
  config/examples/
  AGENTS.md requirements.md task.md README.md Cargo.toml
```
