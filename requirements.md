# requirements.md — aria router（Rust）

> 本文件为 `router` 仓库 **Semantic + Agent 并列网关 / OpenAI 兼容 HTTP / 八语言 SDK** 的功能边界、API、配置、异常与验收标准。**须经人工逐项审核**，审核通过后方可据其生成 / 执行 `task.md`。
>
> 架构参考：[vLLM Semantic Router](https://github.com/vllm-project/semantic-router) YAML v0.3（运行时对等，不做 Dashboard/Operator/Envoy）；agent 面参考 [earendil-works/pi](https://github.com/earendil-works/pi) JSONL RPC 与 [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 进程接入。

## 1. 目标与范围

用 **Rust** 实现独立 OpenAI 兼容网关：按 entrypoint 选择 **semantic** 或 **agent** 决策器，在硬约束剪枝后选择或组合 provider 路径并转发。

- **产品面**：`aria-router` CLI（`validate` / `serve`）+ 进程内库 + C ABI + 八语言 SDK。
- **两种 router**：`semantic`（signals → Boolean recipe → algorithm）与 `agent`（LLM agent + extensions）并列；共享 listeners / providers / 硬约束 / 转发 / replay。
- **不做**：Dashboard、Operator、Helm、官网、Python `vllm-sr`、Envoy ExtProc、把 TS harness 链进 crate。
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
| 5 | **http** | 数据面 `:8899` chat/SSE/`/v1/models`；管理面默认 `127.0.0.1` health/validate/replay/`PUT` provider |
| 6 | **ffi** | `libaria_router_ffi`：init/connect/complete/stream/models/last_route |
| 7 | **bindings** | rust/python/go/typescript/react-native/flutter/swift/kotlin |

### 2.1 非目标

- Envoy ExtProc、Dashboard、Operator、fleet-sim
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
global: {}
```

密钥：`${VAR}` / `${VAR:-default}`。未知顶层块 → validate 失败。

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

**管理面**（默认 `127.0.0.1:8080`）：

- `GET /health`
- `POST /v1/router/validate`
- `GET /v1/router/replay?n=`
- `PUT /v1/router/providers`：engine serve upsert（name、endpoint、provider_model_id）

CLI：`aria-router validate --config`；`aria-router serve --config [--bind] [--mgmt-bind]`。

### 3.6 错误

`RouterError::{Io, Config, Unsupported, InvalidParam, FailClosed, Upstream, Timeout, Extension}`。禁止 panic 当控制流。

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

实例 `auth`：仅内存 `base_url` / `token`；禁止写 `config.yml`。

测试：共享 `cases.json`（lifecycle / chat / stream / models / last_route / connect / fail-closed）；`cargo test -p ariacompute-router-ffi`；`./scripts/run-binding-tests.sh`。

发布：fail-pass 多 registry，不阻断 CLI。

## 4. 验收

- A-semantic：keyword 命中转发；实名 bypass；无路径 fail closed；SSE 至少 1 chunk。
- A-agent：合法 JSON 采纳；非法/越权/超时 fail closed；与 semantic 入口不串扰。
- B：启发式 + 三算法 + 五插件单测。
- C：learned 无权重且被引用 → Unsupported；未知 algorithm 同。
- D：pi/dsh mock command 产出决策；缺 command 启动失败。
- E：八语言跑通 `cases.json` 黄金项。

## 5. 目录

```
router/
  config/ signal/ decision/ algorithm/ plugin/ provider/ agent/ ext/ http/ bin/ ffi/
  bindings/{rust,python,go,typescript,react-native,flutter,swift,kotlin,testdata}/
  config/examples/
  AGENTS.md requirements.md task.md README.md Cargo.toml
```
