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
| 6 | **ffi** | `libaria_router_ffi`：init/connect/complete/stream/models/last_route |
| 7 | **bindings** | rust/python/go/typescript/react-native/flutter/swift/kotlin |
| 8 | **dashboard** | 管理面同端口 SPA（`dashboard/`）；`--no-dashboard` 仅 JSON API；Cost / API 密钥页 |
| 9 | **cost** | 内存六因子账本；`GET /v1/router/cost`；按 model/layer/entrypoint/key 分桶 |
| 10 | **api-keys** | Dashboard 签发；`keys_path` 只存 sha256；数据面与 `PUT /providers` Bearer；`require_api_key` |

### 2.1 非目标

- Envoy ExtProc、Operator、Helm、fleet-sim、Grafana / Prometheus、ML Setup、Security Policy、wizmap、独立 dashboard 端口 / OIDC / 密码登录
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
  require_api_key: false          # true → 数据面 chat 与 PUT providers 须 Bearer
  keys_path: ~/.ariacompute/router-keys.json
```

密钥：`${VAR}` / `${VAR:-default}`。未知顶层块 → validate 失败。未知 `global` / `pricing` 子键 → validate 失败。`backend_refs.api_key` 仅转发上游，与 Dashboard 签发的客户端 key **不是同一把**。

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
- `global.require_api_key: true` 时须 `Authorization: Bearer` 或 `x-api-key` 命中未吊销密钥，否则 **401**

**管理面**（默认 `127.0.0.1:8080`）：

- `GET /health`
- `POST /v1/router/validate`
- `GET /v1/router/replay?n=`
- `PUT /v1/router/providers`：engine serve upsert；`require_api_key: true` 时同样须 Bearer，否则 401
- `GET /v1/router/config`：内存文档 JSON + YAML 文本
- `PUT /v1/router/config`：YAML 或 JSON；`validate` / 缺 extension 二进制失败则 **不替换**；成功换内存文档并写回 `--config` 路径
- `GET /v1/router/overview`：entrypoint / recipe / provider 计数、`last_route`、health、`cost` 摘要、api key 计数
- `GET /v1/router/providers`：模型 + `backend_refs` + pool 延迟/失败
- `GET /v1/router/topology`：entrypoint → recipe（signals / decisions / algorithm / plugins 或 agent extension）→ models
- `POST /v1/router/chat`：同进程复用数据面管线（Playground；记 `user=playground`）；不要求客户端 Bearer
- `GET /v1/router/cost`：六因子 totals/factors、`by_model` / `by_layer` / `by_entrypoint` / `by_key`、`recent`
- `GET /v1/router/keys`：元数据列表（无明文 secret）
- `POST /v1/router/keys`：`{name}` → `{id,name,prefix,secret}`（secret 仅此一次）
- `DELETE /v1/router/keys/:id`：吊销（幂等）
- `GET /` 与 SPA fallback（非 `/v1/*`、非 `/health`）→ `dashboard/dist`；`--no-dashboard` 或无构建产物则不提供 SPA

管理面默认只绑 `127.0.0.1`；绑 `0.0.0.0` 视为运维自担。v1 **无登录**（本机 SPA 可签发密钥）。密钥明文只在 POST 响应出现一次；磁盘 `keys_path` 只存 sha256。

CLI：`aria-router setup` 在 template 后询问 `require API key on data plane?`，写入 `global.require_api_key` + `keys_path`；**不**在 CLI 签发 secret。`--status` 显示开关、路径、key 数量。`--clear` 默认只删 `router.yml`。`validate` / `serve` 同前。

**与 engine**：engine `setup` 写入 `router_api_key`；`serve --router` / `--router-api-key` 在 `PUT /v1/router/providers` 带 Bearer。

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

发布：fail-pass 多 registry，不阻断 CLI。

## 4. 验收

- A-semantic：keyword 命中转发；实名 bypass；无路径 fail closed；SSE 至少 1 chunk。
- A-agent：合法 JSON 采纳；非法/越权/超时 fail closed；与 semantic 入口不串扰。
- B：启发式 + 三算法 + 五插件单测。
- C：learned 无权重且被引用 → Unsupported；未知 algorithm 同。
- D：pi/dsh mock command 产出决策；缺 command 启动失败。
- E：八语言跑通 `cases.json` 黄金项。
- F：`PUT /v1/router/config` 非法 YAML 不改文档；合法 tiny YAML 热重载；topology 对 semantic-tiny / agent-tiny 有预期节点；`POST /v1/router/chat` 走 keyword / fake-agent 黄金路径；`--no-dashboard` 时 `/` 不提供 SPA。
- G：带 `usage` 的 mock chat 计入账本；无 usage → estimate；无 pricing → `cost=0` 且 `priced=false`；`require_api_key: true` 无 Bearer 聊天与 PUT providers → 401；合法 key → 200 且 `by_key` 有 id；吊销后 401；Cost JSON 含六因子键。

## 5. 目录

```
router/
  config/ signal/ decision/ algorithm/ plugin/ provider/ agent/ ext/ http/ bin/ ffi/
  dashboard/   # Vite React SPA；产物 dashboard/dist 由管理面托管
  bindings/{rust,python,go,typescript,react-native,flutter,swift,kotlin,testdata}/
  config/examples/
  AGENTS.md requirements.md task.md README.md Cargo.toml
```
