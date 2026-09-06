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

### T19 — 从 serve 同步 OAuth API key（用户自建 key + router 自动同步元数据）
- [x] **router 不在 serve 为 oauth 用户创建 api key**；由 oauth 用户自行在 serve 创建 `sk-bf-`，router dashboard 通过 link_token/已存 sk-bf- `GET {site}/api/api-keys` **自动同步 key 的 name/prefix/状态**用于展示
- [x] ~~sk-bf- 明文由用户在 dashboard 手动粘贴一次~~（**已废弃**：阶段 J2 serve 列表接口现返回 secret，改为 `POST /v1/router/serve/account/sync` 自动同步明文，手动粘贴入口 `POST /v1/router/serve/account/key` 已下线）
- [x] 重新链接不同 serve 账户时清除上次粘贴的 key（`oauth_clear_api_key`），避免被错误复用
- [x] `keys.rs`：`oauth_api_key()`（取 sk-bf- 凭据）+ `oauth_set_api_key_meta(name,prefix)`（仅更新展示元数据，不动 secret）+ `oauth_clear_api_key()`
- [x] `POST /v1/router/serve/account/sync`：用已存 sk-bf-（回退 link_token）`GET {site}/api/api-keys` 刷新 key 元数据并返回 `ServeAccountPublic`
- [x] Dashboard Account：Serve API key 卡显示 name/prefix + 粘贴输入框 + 「Save key」+「Auto-update」+ 说明文案
- [x] Dashboard Keys：链接后展示「Serve (Aria Compute) API key」（name/prefix，标注 auto-synced from serve）
- [x] 链接时元数据同步放 detached `tokio::spawn`，避免 handler future 持有 !Send 局部变量导致 axum `Handler` trait 不满足
- [x] `api.ts`：`syncServeAccount()`；`AppError: From<RouterError>`
- [x] 验证：`cargo clippy -p aria-router-http`（`auth_api` 无新增告警）、`cargo test -p aria-router-http`（25/25）、`npm --prefix dashboard run build`

## 阶段 J — OAuth API key 删除自动同步

### T20 — Dashboard 自动感知 serve oauth 用户删除/吊销 API key
- [x] 触发场景：serve oauth 用户在 Aria Compute 删除或吊销其 `sk-bf-` key 后，router dashboard 应自动更新，而非长期显示已失效的 key
- [x] 检测信号：sync 用已存 `sk-bf-` 拉取 serve `/api/api-keys` 时收到 401/403 → 视为 key 已删除/吊销（`keys.rs` 新增 `oauth_mark_api_key_deleted`：清掉失效 secret、保留 name/prefix 用于展示、`api_key_deleted` 落盘持久化）；网络/超时失败不改状态
- [x] `ServeAccountPublic` 新增 `api_key_deleted`；`oauth_public()` 状态改为 `api key deleted on serve`；`api_key_configured` 因 secret 清空而变 false，但 name/prefix 仍展示
- [x] 重新粘贴 key（`oauth_set_api_key`）或 sync 拿到 200（bearer 仍有效）时清除 `api_key_deleted` 标记（`oauth_unmark_api_key_deleted`）
- [x] Dashboard Account：每 30s 自动轮询 `serve/account/sync`；删除时显示醒目警告横幅 + 最后已知 key 信息 + 重新粘贴引导；Keys 与 Overview 同步展示 `deleted on serve`
- [x] 单测：`keys.rs` 单测（mark 清 secret+flag / 重贴清 flag）、`lib.rs` 三条集成（`401→deleted` 且 GET 持久化、200 有效 bearer 仅刷新 meta 不误删、200 命中更新 name）
- [x] 验证：`cargo clippy -p aria-router-http` 无告警、`cargo test -p aria-router-http`（33/33）、`npm --prefix dashboard run build` 通过

## 阶段 J2 — Serve API Key 自动同步（明文，去手动粘贴）

### T18 — 从 serve 自动同步 OAuth API key 明文（替换手动粘贴）
- [x] serve `GET /api/api-keys` 现返回明文 secret（`sk-bf-`）；`serve_pick_api_key` 由 `(name,prefix)` 改为 `(name,prefix,secret)`
- [x] `POST /v1/router/serve/account/sync` 在 200 时取最近创建的 active key，调 `oauth_set_api_key` 自动写入明文（新增 key 自动采用）；空列表（或列表无可用 secret）→ `oauth_mark_api_key_deleted`（删除 key 自动清除、保留 name/prefix）
- [x] `oauth_callback` link 完成 detached task 改用 `oauth_set_api_key` 直接存 secret（链接即可用，无需再手动粘贴；保留 `oauth_set_api_key_meta` 仅单测引用不删）
- [x] **下线手动粘贴入口**：删 `POST /v1/router/serve/account/key` 路由与 `serve_account_set_key` handler；`dashboard/src/api.ts` 删 `setServeApiKey`
- [x] Dashboard Account：移除粘贴输入框与「Save key」+ `saveKey`；保留「Sync now」按钮 + 30s 自动轮询；文案改为 auto-synced from serve；删除/不可达显示降级提示（不再提示粘贴）
- [x] Dashboard Keys：serve-deleted 提示去掉「paste it again in Account」，改自动同步引导（在 Aria Compute 重新生成 / 重新关联账户）
- [x] 单测：`lib.rs` 更新 `serve_sync_refreshes_meta_on_200_valid_bearer` / `serve_sync_updates_meta_when_key_present`（mock 补 `secret`），新增 `serve_sync_marks_deleted_when_empty_list`、`serve_sync_adopts_newly_created_key`（覆盖新增/删除两类路径）
- [x] `requirements.md` OAuth 小节补充 `POST /v1/router/serve/account/sync` 自动同步 + 手动粘贴入口下线说明

## 阶段 K — Router bench（ADR-040 + DRACO）

### T21 — Spec
- [x] `requirements.md` §6 路由评测；§5 目录含 `bench/`
- [x] 本清单阶段 K；`AGENTS.md` 目录 + 命令；`README.md` / `README_cn.md` Bench 节

### T22 — 骨架 + quality
- [x] `bench/` 包：cli / http_client / prices / report / corpus tiny
- [x] `quality`：label / overlap / judge / rubric
- [x] unittest 壳（mock，无网络）

### T23 — routing + research
- [x] `routing`：矩阵、always/oracle/analyse、domain/knn、live router picks、`mode: router_routing`
- [x] `research`：JSONL + always_X / aria_router、四轴、`mode: router_research`
- [x] `download-draco`（失败 skip exit 0）

### T24 — 文档与验收
- [x] corpus README；Bench README 节验收说明
- [x] `python -m unittest discover -s bench/tests -t .` 全绿（27/27）

## 阶段 L — aria-router vs vLLM Semantic Router

### T25 — Spec
- [x] `requirements.md` §6 多 router + `compare`；端口约定
- [x] 本清单阶段 L；`AGENTS.md` / README 双语对标示例

### T26 — 多 router CLI / HTTP
- [x] `--router NAME=URL`（裸 URL → `aria_router`）；`--entrypoint` / `--pick-header` / `--pick-map`
- [x] `picked_model(pick_headers)`；api-key `router_<NAME>`

### T27 — runners + compare
- [x] `routing` / `research` 多 live router ladder/systems
- [x] `compare`：grade + runner + `mmlu_tiny.jsonl` + `download-mmlu`
- [x] unittest：双 router mock、compare、裸 URL 兼容

### T28 — 文档与验收
- [x] corpus README（MMLU）；README 双语
- [x] `python -m unittest discover -s bench/tests -t .` 全绿

## 阶段 M — 轻量 builtin agent（去 ext）

### T29 — 去 pi/dsh + 进程内 builtin tool-loop
- [x] Spec：`requirements.md` §3.4 / 目录；本清单；`AGENTS.md`
- [x] 删除 `ext/` crate 与顶层 `extensions:`；publish 去掉 `aria-router-ext`
- [x] `BuiltinAgent`：固定工具 + `max_turns` / `timeout_ms`；`submit_route` 终态
- [x] `agent-tiny` / setup template；http `route_agent`；topology / Dashboard 文案
- [x] engine README / requirements 同步（semantic + builtin agent）
- [x] `cargo test` 全绿
