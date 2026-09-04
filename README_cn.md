# router

[English](README.md) | [中文](README_cn.md)

Aria Compute 推理网关：OpenAI 兼容 HTTP，两种并列决策器（**semantic** YAML v0.3 与 **agent** extensions）。共享 providers、硬约束与转发。

## 构建 / 测试

```bash
cargo test
cargo clippy --workspace --all-targets -- -D warnings
./scripts/run-binding-tests.sh
```

## 配置 / 运行

路由策略为 YAML v0.3（`--config`）。密钥用 `${VAR}` / `${VAR:-default}` 展开。`entrypoint.router` 必须等于 `recipe.router`。Semantic recipe 禁止出现 `agent:`；agent recipe 禁止出现 `signals` / `decisions`。未知顶层键 → `validate` 失败。未实现的 YAML 能力返回 `Unsupported`（禁止静默空实现）。未安装 `pi` / `deepseek-harness` 二进制时 **serve 失败**，不会静默改走 semantic。

| 块 | 含义 |
|----|------|
| `listeners` | 数据面绑定（`address` + `port`；`--bind` 默认值） |
| `providers` | `defaults.default_model` + 具名模型 / `backend_refs` |
| `extensions` | Agent 适配：`builtin` / `pi` / `deepseek-harness` |
| `entrypoints` | 虚拟模型名 → `router: semantic\|agent` + `recipe` |
| `recipes` | Semantic 的 `routing.*` 或 agent 的 `agent.*` |
| `global` | 可观测 / 分类器资产（可选） |

示例（YAML 内注释为英文）：

| 文件 | 用途 |
|------|------|
| [`semantic-tiny.yaml`](config/examples/semantic-tiny.yaml) / [`agent-tiny.yaml`](config/examples/agent-tiny.yaml) / [`ffi-tiny.yaml`](config/examples/ffi-tiny.yaml) | 黄金路径 — 可 `validate` + `serve` / FFI |
| [`semantic.yaml`](config/examples/semantic.yaml) | 启发式 + `aria/semantic-catalog`（learned signal 与未实现 algorithm → chat `Unsupported`） |
| [`agent.yaml`](config/examples/agent.yaml) | `builtin` + `pi` + `deepseek-harness`。`validate` 通过；`serve` 需本机二进制 |
| [`ffi.yaml`](config/examples/ffi.yaml) | 黄金 `fast-response` + 同上 catalog recipe |

```bash
# 写入配置 — 两段凭证：
#   [1/2] Local (router Dashboard) — 本地管理员；sk-aria_ 仅在 Dashboard → Keys 签发
#   [2/2] OAuth (Aria Compute)     — 可选 bfvk-… 写入 keys[] kind=oauth（~/.ariacompute/router-keys.json）
aria-router setup
aria-router setup --status
# Local flags: --admin-user --admin-password --allow-register --require-api-key
# OAuth flags: --serve-site com|cn --serve-api-key bfvk-…

# 校验（默认路径，或传 --config）
aria-router validate
cargo run -p aria-router -- validate --config config/examples/semantic-tiny.yaml

# 服务 — 数据面来自 YAML listeners（semantic-tiny：127.0.0.1:8899）；
# 管理面默认 127.0.0.1:8080
cargo run -p aria-router -- serve --config config/examples/semantic-tiny.yaml

# 显式绑定（若要注册 aria-engine，管理面不要占用 engine 的 8080）
cargo run -p aria-router -- serve \
  --config config/examples/semantic-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090

cargo run -p aria-router -- serve \
  --config config/examples/agent-tiny.yaml \
  --bind 127.0.0.1:8899 \
  --mgmt-bind 127.0.0.1:8090
```

`--bind` 是 **数据面**（`POST /v1/chat/completions`、`GET /v1/models`）。`--mgmt-bind` 是 **管理面**（`/health`、validate、replay、providers、config、topology、playground chat，以及运维 Dashboard）。一次请求不会串跑 semantic 与 agent。实名 provider **bypass** recipe，直打该后端。

## Dashboard

管理面在存在 `dashboard/dist` 时托管 Vite React SPA，地址为 `http://{mgmt}/`。完成 `aria-router setup` 后先 **Login**（本地用户名/密码）：

| 段落标题（与 CLI 同名） | 页面 |
|------------------------|------|
| **Local (router Dashboard)** | Login / Register · API 密钥（`sk-aria_…`）· Users（admin） |
| **OAuth (Aria Compute)** | Account — 已关联用户、`bfvk-…` 展示/粘贴/OAuth |

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
2. `aria-router setup` 可打开 `global.require_api_key`，使 chat 与 `PUT /v1/router/providers` 需要 Bearer。
3. 客户端与 `aria-engine` 使用同一把 **Local** secret（engine：`router_api_key` / `--router-api-key`；勿粘贴 `bfvk-`）。
4. **Cost** 页 / `GET /v1/router/cost` 展示六因子、`by_local_user` / `by_serve_user` / `by_key`（YAML `pricing`）。
5. OAuth：Dashboard → Account 或 `aria-router setup` [2/2] 写入 `bfvk-…`。

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

动态库顺序：`ARIA_ROUTER_FFI_LIB` → 包内捆绑路径 → `~/.ariacompute/lib/`。实例 `setup` 仅内存 `base_url` / `token`，**禁止**写 `router.yml`。`init` 进程内跑 semantic 与 `builtin` agent；无 subprocess 的平台上 `type: pi` / `deepseek-harness` 显式 `Unsupported`。

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

## 工程约定

本仓库遵循 Harness Engineering：

- [`AGENTS.md`](AGENTS.md)：Agent 工程上下文入口与目录索引
- [`requirements.md`](requirements.md)：需求规格（功能边界 / 异常 / 验收，人审后编码）
- [`task.md`](task.md)：实施清单
