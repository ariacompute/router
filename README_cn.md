# router

[English](README.md) | [中文](README_cn.md)

Aria Compute 推理网关：OpenAI 兼容 HTTP，两种并列决策器（**semantic** YAML v0.3 与 **轻量 builtin agent**：进程内固定工具 + 限 turns）。共享 providers、硬约束与转发。

## 构建 / 测试

```bash
cargo test
cargo clippy --workspace --all-targets -- -D warnings
./scripts/run-binding-tests.sh
```

## 配置 / 运行

路由策略为 YAML v0.3（`--config`）。密钥用 `${VAR}` / `${VAR:-default}` 展开。`entrypoint.router` 必须等于 `recipe.router`。Semantic recipe 禁止出现 `agent:`；agent recipe 禁止出现 `signals` / `decisions`。未知顶层键 → `validate` 失败。未实现的 YAML 能力返回 `Unsupported`（禁止静默空实现）。**无**顶层 `extensions`，不做 pi / deepseek-harness 子进程。

| 块 | 含义 |
|----|------|
| `listeners` | 数据面绑定（`address` + `port`；`--bind` 默认值） |
| `providers` | `defaults.default_model` + 具名模型 / `backend_refs` |
| `entrypoints` | 虚拟模型名 → `router: semantic\|agent` + `recipe` |
| `recipes` | Semantic 的 `routing.*` 或 agent 的 `agent.*`（endpoint / max_turns / timeout / fallback） |
| `global` | 鉴权 / keys / users 路径（可选） |

示例（YAML 内注释为英文）：

| 文件 | 用途 |
|------|------|
| [`semantic-tiny.yaml`](config/examples/semantic-tiny.yaml) | 日常 semantic 黄金路径 — `aria/semantic-auto`；keyword 启发式 → `local/general`；无 ML 权重即可 `validate` + `serve` |
| [`semantic.yaml`](config/examples/semantic.yaml) | Semantic catalog — 黄金 `aria/semantic-auto` + `aria/semantic-catalog`（learned signal + 未实现 algorithm → chat `Unsupported`）；演示优先 tiny |
| [`semantic-gateway.yaml`](config/examples/semantic-gateway.yaml) | Semantic + Aria Gateway — 三档 `ariamodel-{small,mid,large}` 经 `cloud/gateway`（`https://gateway.ariacompute.com`）；keyword → large，否则 small；需 `GATEWAY_API_KEY` |
| [`agent-tiny.yaml`](config/examples/agent-tiny.yaml) | 日常 agent 黄金路径 — `aria/agent-auto`；进程内 builtin tool-loop（无 `endpoint` → first-eligible）；演示 / CI |
| [`agent.yaml`](config/examples/agent.yaml) | Agent catalog — 对称 `semantic.yaml`：黄金 `aria/agent-auto` + `aria/agent-catalog`（故意 `Unsupported`）；演示优先 tiny / gateway |
| [`agent-gateway.yaml`](config/examples/agent-gateway.yaml) | Agent + Aria Gateway — 同上三档云模型；builtin agent LLM 与后端经 `cloud/gateway`；需 `GATEWAY_API_KEY` |
| [`ffi-tiny.yaml`](config/examples/ffi-tiny.yaml) | FFI / binding 黄金路径 — `fast-response` 固定回复（无需 upstream）；`cases.json` / `run-binding-tests.sh` 优先用此文件 |
| [`ffi.yaml`](config/examples/ffi.yaml) | FFI catalog — 黄金 `fast-response` + 与 `semantic.yaml` 相同的 Unsupported catalog recipe；binding 测试优先 `ffi-tiny` |

```bash
# 写入配置 — template + admin（require_api_key / allow_register / OAuth 改 YAML 或 Dashboard）
aria-router setup
aria-router setup --status
# Flags: --template --admin-user --admin-password

# 校验（setup 后默认路径，或传 --config）
aria-router validate
cargo run -p aria-router -- validate --config config/examples/semantic-tiny.yaml
cargo run -p aria-router -- validate --config config/examples/semantic.yaml
cargo run -p aria-router -- validate --config config/examples/semantic-gateway.yaml
cargo run -p aria-router -- validate --config config/examples/agent-tiny.yaml
cargo run -p aria-router -- validate --config config/examples/agent.yaml
cargo run -p aria-router -- validate --config config/examples/agent-gateway.yaml

# 服务 — 数据面来自 YAML listeners（示例均为 127.0.0.1:8899）；
# 管理面默认 127.0.0.1:8080。setup 后可省略 --config。
# 若要注册 aria-engine，--mgmt-bind 不要占用 engine 的 8080。

# 日常 semantic 黄金路径（aria/semantic-auto → local/general）
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Semantic catalog（aria/semantic-auto + aria/semantic-catalog；catalog chat → Unsupported）
cargo run -p aria-router -- serve \
  --config config/examples/semantic.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# 日常 agent 黄金路径（aria/agent-auto；builtin tool-loop）
cargo run -p aria-router -- serve \
  --config config/examples/agent-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Agent catalog（aria/agent-auto + aria/agent-catalog；catalog chat → Unsupported）
cargo run -p aria-router -- serve \
  --config config/examples/agent.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# Aria Gateway 后端（先 export GATEWAY_API_KEY；YAML 使用 ${GATEWAY_API_KEY:-}）
export GATEWAY_API_KEY=…   # 勿提交
cargo run -p aria-router -- serve \
  --config config/examples/semantic-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

cargo run -p aria-router -- serve \
  --config config/examples/agent-gateway.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
```

`--bind` 是 **数据面**（`POST /v1/chat/completions`、`GET /v1/models`）。`--mgmt-bind` 是 **管理面**（`/health`、validate、replay、providers、config、topology、playground chat，以及运维 Dashboard）。一次请求不会串跑 semantic 与 agent。实名 provider **bypass** recipe，直打该后端。

## Dashboard

管理面在存在 `dashboard/dist` 时托管 Vite React SPA，地址为 `http://{mgmt}/`。完成 `aria-router setup` 后先 **Login**（本地用户名/密码）：

| 段落标题（与 CLI 同名） | 页面 |
|------------------------|------|
| **Local (router Dashboard)** | Login / Register · API 密钥（`sk-aria_…`）· Users（admin） |
| **OAuth (Aria Compute)** | Account — 已关联用户、`sk-bf-…` 展示/粘贴/OAuth |

Cost 分 **Local users** 与 **OAuth users**。构建：

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
# 打开 http://127.0.0.1:8090/
```

`--no-dashboard` 只提供 JSON API（仍可用 curl 管理 keys/cost）。无 Grafana、ML wizard；默认绑 `127.0.0.1`。

### API 密钥与成本

1. Dashboard → **API 密钥** → 生成（`sk-aria_…` 只显示一次），或 `POST /v1/router/keys`。
2. setup 默认写入 `global.require_api_key: true`（chat 与 `PUT /v1/router/providers` 须 Bearer）。可在 YAML 改为 `false` 开放数据面。
3. 客户端与 `aria-engine` 使用 Bearer `sk-aria_…` 或 `sk-bf-…`（`router_api_key` / `--router-api-key`）。
4. **Cost** 页 / `GET /v1/router/cost` 展示六因子、`by_local_user` / `by_serve_user` / `by_key`（YAML `pricing`）。
5. OAuth：Dashboard → Account 配置 `sk-bf-…`（关联 / 粘贴 / 展示）。

```bash
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -H 'Authorization: Bearer sk-aria_…' \
  -d '{"model":"local/general","messages":[{"role":"user","content":"Hi"}],"max_tokens":16}'
```

硬约束（location / auth / modality / tools）在排名 **之前** 剪枝。Compute 只进排名。无合格路径 → fail closed。

## 注册 aria-engine

本进程做路由；`aria-engine serve` 可选择向本网关注册为本地 provider。端口不要撞车：engine `--bind` vs 本仓 `--mgmt-bind`（默认 `127.0.0.1:8080`）。客户端打 **数据面**。

```bash
# 1. router 仓 — 数据面 :8899，管理面 :8090
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

# 2. engine 仓 — OpenAI 在 :8080，再 PUT 到管理面
# 若 router 开启 require_api_key，需带 Dashboard 签发的 secret：
aria-engine serve gemma-4-e2b-it_q4 \
  --bind 127.0.0.1:8080 \
  --router http://127.0.0.1:8090 \
  --router-api-key sk-aria_… \
  --compute auto

# engine 侧也可写入配置，不必每次带旗标：
#   aria-engine setup  # router URL + 可选 router API key
#   # 或 ~/.ariacompute/engine.yml：
#   # router: http://127.0.0.1:8090
#   # router_api_key: sk-aria_…

# 3. 经本网关对话（实名 = bypass → 已注册 engine）
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "gemma-4-e2b-it_q4",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .
```

engine 的 `serve` 会 `PUT {router}/v1/router/providers`（`{name, endpoint, provider_model_id, locality}`），失败则 **退出**。`aria/semantic-auto` / `aria/agent-auto` 只有在 YAML 的 `modelRefs` / `default_model` 写成同一注册名时才会打到该 engine。

手动 upsert（同一契约）：

```bash
curl -s -X PUT http://127.0.0.1:8090/v1/router/providers \
  -H 'content-type: application/json' \
  -d '{
    "name": "gemma-4-e2b-it_q4",
    "endpoint": "127.0.0.1:8080",
    "provider_model_id": "gemma-4-e2b-it_q4",
    "locality": "local"
  }' | jq .
```

## OpenAI API

假设数据面 `http://127.0.0.1:8899`、管理面 `http://127.0.0.1:8090`：

```bash
# 列出入口名 + provider 名
curl -s http://127.0.0.1:8899/v1/models | jq .

# Semantic 入口（semantic-tiny.yaml 的 keyword recipe）
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/semantic-auto",
    "messages":[{"role":"user","content":"please explain rust"}],
    "max_tokens": 32
  }' | jq .

# Agent 入口（agent-tiny.yaml）
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/agent-auto",
    "messages":[{"role":"user","content":"Hello"}],
    "max_tokens": 32
  }' | jq .

# Chat（SSE）
curl -sN http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "aria/semantic-auto",
    "messages":[{"role":"user","content":"please explain rust"}],
    "stream": true
  }'

# 实名 bypass
curl -s http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{
    "model": "local/general",
    "messages":[{"role":"user","content":"Hello"}]
  }' | jq .

# 路由响应头（layer = semantic | agent | bypass）
curl -sD - -o /dev/null http://127.0.0.1:8899/v1/chat/completions \
  -H 'content-type: application/json' \
  -d '{"model":"aria/semantic-auto","messages":[{"role":"user","content":"please explain rust"}]}' \
  | grep -i x-aria-router

# 管理面
curl -s http://127.0.0.1:8090/health | jq .
curl -s -X POST http://127.0.0.1:8090/v1/router/validate | jq .
curl -s 'http://127.0.0.1:8090/v1/router/replay?n=20' | jq .
curl -s http://127.0.0.1:8090/v1/router/overview | jq .
curl -s http://127.0.0.1:8090/v1/router/providers | jq .
curl -s http://127.0.0.1:8090/v1/router/topology | jq .
curl -s http://127.0.0.1:8090/v1/router/config | jq .
```

响应头：`x-aria-router-layer`、`x-aria-router-decision`、`x-aria-router-model`。

## SDK Bindings

C ABI（`ariacompute-router-ffi` / `libaria-router_ffi`）加 `bindings/` 薄封装。**不要**与 `libaria_ffi` / `ariacompute-engine` 混用。

| Binding | 路径 | 包 |
|---------|------|-----|
| Rust | `bindings/rust` | `ariacompute-router`（原生；不 dlopen） |
| Python | `bindings/python` | `aria_router` |
| Go | `bindings/go` | Go module |
| TypeScript | `bindings/typescript` | npm `@ariacompute/router-ts` |
| React Native | `bindings/react-native` | npm `@ariacompute/router-rn` |
| Flutter | `bindings/flutter` | pub.dev |
| Swift | `bindings/swift` | CocoaPods |
| Kotlin | `bindings/kotlin` | Maven |

C 头文件：[`ffi/include/aria_router.h`](ffi/include/aria_router.h) — `aria_router_init`（进程内加载 YAML）、`aria_router_connect`（HTTP 连已运行的 `serve`）、`aria_router_complete` / `_stream`、`aria_router_models`、`aria_router_last_route`、`aria_router_destroy`、`aria_router_last_error`。

动态库顺序：`ARIA_ROUTER_FFI_LIB` → 包内捆绑路径 → `~/.ariacompute/lib/`。实例 `setup` 仅内存 `base_url` / `token`，**禁止**写 `router.yml`。`init` 进程内跑 semantic 与轻量 builtin agent（固定工具 + `max_turns`）；无 subprocess harness。

```bash
cargo test -p ariacompute-router-ffi -p ariacompute-router
./scripts/run-binding-tests.sh   # 宿主矩阵（Rust / Python / Go / TS / RN setup）
```

C ABI 变更必须同步 [`bindings/testdata/cases.json`](bindings/testdata/cases.json) 与宿主测。

### 示例

**Python**（需 `ARIA_ROUTER_FFI_LIB` 或捆绑/缓存的 `libaria-router_ffi`）：

```python
from aria_router import Router

r = Router().init("config/examples/ffi-tiny.yaml")
print(r.models())
print(r.complete(
    [{"role": "user", "content": "hi"}],
    {"model": "aria/semantic-auto"},
))
print(r.last_route())
r.close()

# 或连接已运行的 serve（数据面）：
r = Router().connect("http://127.0.0.1:8899")
r.setup(base_url="http://127.0.0.1:8899", token="")  # 仅内存
```

**Rust**（`ariacompute-router` — 原生 API，不 dlopen `libaria-router_ffi`）：

```rust
use ariacompute_router::Router;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut r = Router::new();
    r.init("config/examples/ffi-tiny.yaml")?;
    let out = r.complete(
        json!([{"role": "user", "content": "hi"}]),
        json!({"model": "aria/semantic-auto"}),
    )?;
    println!("{out}");
    Ok(())
}
```

## Release 资产

每次 GitHub Release，[`.github/workflows/release.yml`](.github/workflows/release.yml) 上传与 `aria-router` CLI 并列的平台包：

| 资产 | 内容 |
|------|------|
| `aria-router_<ver>_linux_x86_64.tar.gz` | `aria-router` |
| `aria-router_<ver>_linux_arm64.tar.gz` | `aria-router` |
| `aria-router_<ver>_macos.tar.gz` | `aria-router` |
| `aria-router_<ver>_windows_x86_64.zip` | `aria-router.exe` |
| `libaria-router_ffi_<ver>_<os>.tar.gz` | `libaria-router_ffi.so` / `.dylib` / `aria-router_ffi.dll` |

```bash
# 示例：Linux x86_64
tar -xzf aria-router_0.1.0_linux_x86_64.tar.gz
chmod +x aria-router
./aria-router --version
```

创建 **GitHub Release** — 语言包发布为 **fail-pass**，不阻塞 CLI / FFI 资产。版本号 = tag 去掉前缀 `v`。

## Bench

[`bench/`](bench/) 下 Python ≥3.10 **report-only** 评测（标准库；对齐 `engine/bench`）。**不**拉起 `aria-router`、[vLLM Semantic Router](https://github.com/vllm-project/semantic-router) 或 provider——需自行 serve 后把 `--router` / `--pool` 指到 OpenAI 兼容 URL。报告恒含 `ci_fail: false`。

| 子命令 | 用途 |
|--------|------|
| `routing` | ADR-040 矩阵 + always / oracle / domain / knn / **多 live router** ladder |
| `research` | DRACO 形 JSONL + rubric，对比 always_X 与各 router |
| `compare` | MCQ **accuracy + E2E latency + tokens**（对齐 VSR 公开 bench 叙事） |
| `list-corpus` | 打印内置 tiny 语料 |
| `download-draco` / `download-mmlu` | 可选 HF 拉取（失败 skip） |

`--router` 可重复 `NAME=URL`（裸 URL → `aria_router`）。配合 `--entrypoint` / `--pick-header` / `--pick-map`。

并排对标建议端口：**aria-router `:8899`**、**vLLM SR `:8890`**、共享 backend `:8000` / `:9001+`。

routing / research 的质量模式（`--quality`）：`label`、`overlap`、`judge`（需 `--judge-url`）。

两条常用 track（同一套 `routing` / `compare` CLI，测的对象不同）：

| Track | 实际打谁 | 主要 flags | 测什么 |
|-------|----------|------------|--------|
| Gateway 仅 pool | **直接** chat Aria Gateway 模型 URL | 仅 `--pool` + `--model-id` + `--api-key`；**无** `--router` | `ariamodel-{small,mid,large}` 后端质量 / 成本阶梯（不启本地 router） |
| 多 router ladder | 经 **live router** chat，再（可选）共享 pool | `--router` + `--entrypoint` / `--pick-header`，外加 `--pool` 做 always/oracle 基线 | 选路质量（aria-router vs vLLM SR 等）在 ADR-040 / MCQ 上的表现 |

可选中间步：本地 serve `semantic-gateway` / `agent-gateway`，在 Gateway track 上追加 `--router aria_router=…`，使选路经 router 再进同一云 pool。

```bash
python -m unittest discover -s bench/tests -t .

# --- Track A：Aria Gateway 仅 pool（可不启本地 router）---
export GATEWAY_BASE=https://gateway.ariacompute.com
export GATEWAY_API_KEY=…   # 勿提交
# expected_model 改为 ariacompute/ariamodel-{small,large}（见 out/routing_gateway.json）

python -m bench routing \
  --pool small=$GATEWAY_BASE --pool mid=$GATEWAY_BASE --pool large=$GATEWAY_BASE \
  --model-id small=ariacompute/ariamodel-small \
  --model-id mid=ariacompute/ariamodel-mid \
  --model-id large=ariacompute/ariamodel-large \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --quality label \
  --corpus ./out/routing_gateway.json \
  --report ./out/gateway_routing.json

python -m bench compare \
  --pool small=$GATEWAY_BASE --pool mid=$GATEWAY_BASE --pool large=$GATEWAY_BASE \
  --model-id small=ariacompute/ariamodel-small \
  --model-id mid=ariacompute/ariamodel-mid \
  --model-id large=ariacompute/ariamodel-large \
  --api-key small=$GATEWAY_API_KEY --api-key mid=$GATEWAY_API_KEY --api-key large=$GATEWAY_API_KEY \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --report ./out/gateway_compare.json

# 可选：先 serve semantic-gateway 或 agent-gateway，再在上述命令追加：
#   --router aria_router=http://127.0.0.1:8899 \
#   --entrypoint aria_router=aria/semantic-auto \   # 或 aria/agent-auto
#   --pick-header aria_router=x-aria-router-model
```

```bash
# --- Track B：多 router ADR-040 ladder（aria-router vs vLLM Semantic Router）---
# 需本地 aria-router（:8899）、vLLM SR（:8890）以及 pool 后端（:9001+ / :8000）。
# --router 评估 live 选路质量；--pool 仍跑 always_* / oracle 基线。

python -m bench routing \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=aria/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pool small=http://127.0.0.1:9001 --pool large=http://127.0.0.1:9002 \
  --model-id small=local/small --model-id large=local/large \
  --quality label \
  --corpus bench/corpus/routing_tiny.json \
  --report ./out/vs_vsr_routing.json

# MCQ compare（经各 router 的 accuracy / latency / tokens）
python -m bench compare \
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=aria/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pool base=http://127.0.0.1:8000 \
  --model-id base=Qwen/Qwen3-0.6B \
  --corpus bench/corpus/mmlu_tiny.jsonl \
  --report ./out/vs_vsr_compare.json
```

详见 [`bench/corpus/README.md`](bench/corpus/README.md)。规格：[`requirements.md`](requirements.md) §6。

## 工程约定

本仓库遵循 Harness Engineering：

- [`AGENTS.md`](AGENTS.md)：Agent 工程上下文入口与目录索引
- [`requirements.md`](requirements.md)：需求规格（功能边界 / 异常 / 验收，人审后编码）
- [`task.md`](task.md)：实施清单
