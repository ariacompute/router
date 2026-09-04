# task.md — aria router 实施清单

依据 [`requirements.md`](requirements.md)。完成后勾选。

## 规格

### T0 — 文档
- [x] `AGENTS.md` ≤100 行
- [x] `requirements.md` / 本文件 / `README.md`

## 阶段 A-semantic

### T1 — Workspace
- [x] 根 `Cargo.toml`；crate：config / signal / decision / algorithm / plugin / provider / agent / ext / http / bin / ffi
- [x] 共享 `RouterError`
- [x] `cargo test` 可运行

### T2 — Config + semantic 黄金路径
- [x] YAML v0.3 解析、`${VAR}`、entrypoint↔recipe 一致校验
- [x] `keyword` + Boolean AND/OR/NOT + priority + `static`
- [x] 实名 bypass；default_model；fail closed
- [x] `POST /v1/chat/completions` JSON + SSE；`GET /v1/models`；路由响应头
- [x] `validate` / `serve` CLI
- [x] 单测：解析、命中、bypass、转发、失败闭环

## 阶段 A-agent

### T3 — builtin agent
- [x] 硬剪枝 → `builtin` → schema `RouteDecision` → 同一转发
- [x] 非法 JSON / 越权 model / 超时 fail closed
- [x] 与 semantic 入口互不串扰
- [x] 管理面 `PUT /v1/router/providers`

## 阶段 B

### T4 — 启发式与核心算法/插件
- [x] signals：authz / context / conversation / event / keyword / language / metadata / structure
- [x] projections：partition / score / mapping
- [x] algorithms：static / latency-aware / multi-factor
- [x] plugins：header-mutation / request-params / system-prompt / fast-response / response-cache
- [x] provider health + 加权 backend_refs

## 阶段 C

### T5 — 运行时对等门控
- [x] learned signal 类型可解析；无 `ml` 或无权重且被引用 → `Unsupported`
- [x] 其余 selection/looper/plugin 名可解析，未实现 → `Unsupported`
- [x] YAML 未知块失败

## 阶段 D

### T6 — pi / deepseek-harness
- [x] `type: pi` JSONL RPC adapter；fixture 不强制本机安装 pi
- [x] `type: deepseek-harness` stdin JSON 决策；CI mock command
- [x] 缺二进制 serve 失败

## 阶段 E

### T7 — C ABI
- [x] `aria_router.h`：init / connect / complete / stream / models / last_route / destroy / last_error
- [x] `bindings/testdata/cases.json`
- [x] `cargo test -p ariacompute-router-ffi`

### T8 — 八语言 SDK
- [x] rust / python / go / typescript / react-native / flutter / swift / kotlin
- [x] `./scripts/run-binding-tests.sh`
- [x] 包名与 `libaria_ffi` 不冲突

## engine 协同（engine 仓）

### T9 — 去 hybrid
- [x] 删除 `cloud_api_key` / `cloud_url` / `hybrid_*`；chat 仅本地
- [x] `list` 只扫本地 models
- [x] auth 不再强制 API key；可选 `router`
- [x] `--router` 覆盖不回写；bindings 同步

### T10 — 注册
- [x] serve 在 `router` 非空时 PUT upsert；失败退出

### T11 — `setup` + 默认 `router.yml`
- [x] CLI `aria-router setup [--status|--clear]` 写入 `~/.ariacompute/router.yml`
- [x] `validate` / `serve` 的 `--config` 可选，缺省该路径
- [x] 八语言 SDK `Router.setup` / `setup_status` / `setup_clear`；不写 yml

## 阶段 F — 运维 Dashboard

### T12 — 管理面 SPA
- [x] Spec：`requirements.md` 阶段 F；Dashboard 移出非目标；仍不做 Grafana / ML / Security
- [x] `GET/PUT /v1/router/config`、overview、providers GET、topology、`POST /v1/router/chat`
- [x] `serve` 托管 `dashboard/dist`；`--no-dashboard`
- [x] `dashboard/`：Overview / Config / Topology / Providers / Replay / Playground
- [x] 单测：非法 PUT 不更新、topology 黄金 YAML、playground chat、providers 列表

## 阶段 G — 成本 + API key

### T13 — 六因子账本与密钥
- [x] Spec：`requirements.md` 阶段 G；`pricing` / `require_api_key` / `keys_path`
- [x] `setup` 询问 API key 开关；密钥仅 Dashboard / `POST /v1/router/keys` 签发
- [x] 数据面 chat 与 `PUT /providers`：`require_api_key` → Bearer/401
- [x] `CostLedger` + `GET /v1/router/cost`（含 `by_key`）；overview 摘要
- [x] Dashboard：Cost 页 + API 密钥页
- [x] engine：`router_api_key` / `--router-api-key`；`register_with_router` 带 Authorization
- [x] 单测：usage 计费、401、CRUD、by_key、engine Bearer

## 阶段 H — 本地用户 + OAuth

### T14 — Spec
- [x] `requirements.md` 阶段 H；§2.1 / §3.1 / §3.5；serve router-link；engine `serve_*`
- [x] 本清单与 AGENTS / README

### T15 — 本地用户
- [x] `router-users.json` argon2；register/login/session；`allow_register`；admin Users
- [x] keys `owner_user_id`；mgmt session 门控

### T16 — OAuth 账户
- [x] serve `/api/router-link/*`；`router-keys.json` 含 `kind: oauth`；mgmt serve/account APIs
- [x] Dashboard Account 展示；Cost `by_local_user` / `by_serve_user`；dual Bearer

### T17 — CLI
- [x] `aria-router setup` 仅 template + admin；`--status` 扁平字段
- [x] `aria-engine setup`；`router_api_key` 双前缀；router `keys[]` kind local|oauth

## CLI help 对齐 memo（clap）

### T18 — clap help usage
- [x] `aria-router` 迁 clap derive；删除手写 `print_usage`
- [x] `-v` / `--version` / `version`；无参 exit 2；子命令 `--help`
- [x] `cargo test` + `--help` 冒烟

## 阶段 I — Serve API Key 同步

### T19 — 从 serve 同步 OAuth API key
- [x] OAuth 回调：exchange 后若有 `link_token`，**同账户重新链接自动复用既有 serve 签名 key（按 owner_user_id 匹配，不新建、不累积）**；仅当无可用 key（首次链接或切换账户）时，才 `POST {site}/api/api-keys`（name `aria-router`）签发 serve `bfvk-` 并存入 oauth 记录（proxying/后续同步的持久凭据）；签发/复用失败则退化为仅存 exchange 返回的元数据
- [x] `keys.rs`：`oauth_api_key()`（取 bfvk- 凭据）+ `oauth_set_api_key_meta(name,prefix)`（仅更新展示元数据，不动 secret）
- [x] `POST /v1/router/serve/account/sync`：用已存 bfvk-（回退 link_token）`GET {site}/api/api-keys` 刷新 key 元数据并返回 `ServeAccountPublic`
- [x] Dashboard Account：Serve API key 卡显示 name/prefix + 「Auto-update」按钮
- [x] Dashboard Keys：链接后展示「Serve (Aria Compute) API key」（name/prefix，标注 auto-synced from serve）
- [x] `api.ts`：`syncServeAccount()`；`AppError: From<RouterError>`
- [x] 验证：`cargo clippy -p aria-router-http`（`auth_api` 无新增告警）、`cargo test -p aria-router-http`（25/25）、`npm --prefix dashboard run build`
