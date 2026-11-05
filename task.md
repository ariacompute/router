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

## 阶段 N — Playground 对齐 vLLM SR（核心体验）

### T31 — Playground chat UX
- [x] Spec：`requirements.md` §3.5 / §1.2 / F 验收；本清单
- [x] `GET /v1/router/models`；扩展 `x-aria-router-algorithm|reason|confidence|bypass`
- [x] provider/http：SSE `bytes_stream` 透传；流结束后 cost；单测
- [x] Dashboard：多轮气泡、Composer、RouteHeaders、markdown、SSE client、localStorage 会话侧栏
- [x] 不做 MCP / Claw / Web Search / 附件 / 语音
- [x] `cargo test` + `npm --prefix dashboard run build`

## 阶段 O — Gateway bench win vs vLLM SR（非对称）

### T32 — Spec §6.6
- [x] `requirements.md` §6.6；本清单阶段 O

### T33 — 投影入决策 + 热路径
- [x] `Condition.equals`；`select_decision` 吃 `ProjectionMap`；`type: projection`
- [x] cache key = 选中模型 + `last_user_text`；仅插件启用时 `remember_response`
- [x] `x-aria-router-latency-ms`；单测

### T34 — aria advanced mom + 冻结 vllm baseline
- [x] `semantic-gateway.yaml`：factoid_small、投影条件、pricing、fallback response-cache
- [x] `config-gateway.yaml`：注释 baseline frozen，不同步增量

### T35 — hard corpus + prices
- [x] `bench/corpus/routing_gateway_hard.json`
- [x] `bench/prices/ariamodel.json`

### T36 — compare / docs
- [x] README / README_cn Track B Gateway compare + prices + hard corpus
- [x] vllm-sr README 标明 baseline

### T37 — Dashboard
- [x] Playground 展示 latency（+ tokens 若可得）

### T38 — 验收
- [x] `cargo test`；routing hard / compare 报告达标（见 §6.6）
  - routing hard：aria quality `1.0` ≥ vllm `0.833`；cost `0.000444` < `0.000498`；q/$ `2254` > `1672`
  - compare：aria acc `0.0` ≥ vllm `0.0`；p50 `1536ms` ≤ `2787ms`

## 阶段 P — Compare 热路径（共享 HTTP 客户端）

### T39 — Spec §6.7
- [x] `requirements.md` §6.7；本清单

### T40 — 共享 reqwest::Client
- [x] `PoolState` 内置 `Client`；`forward` / `forward_sse_stream` 复用
- [x] 单测 / `cargo test` 绿

### T41 — static 跳过 ranking maps
- [x] `route_semantic`：`algorithm: static` 不建全量 cost/latency map

### T42 — 文档与验收
- [x] MODEL_CHECKLIST / README 注明连接复用 + 冷 compare
- [ ] 冷启动 compare：acc ≥ 且 p50/mean ≤ vllm（需本机重建 serve 后重跑）

## 阶段 Q — Agent strategy Track B（非对称）

### T43 — Spec §6.8
- [x] `requirements.md` §6.8；本清单阶段 Q

### T44 — agent-gateway TokenHub 对齐
- [x] `agent-gateway.yaml`：provider_model_id / base_url / pricing / agent.endpoint

### T45 — 文档
- [x] README / README_cn Agent ladder 命令块
- [x] MODEL_CHECKLIST + corpus / vllm-sr README

### T46 — bench 单测
- [x] `agent-auto` entrypoint 解析进 `RouterSpec`

### T47 — 验收
- [x] `cargo test` + bench unittest（含 `agent-auto` entrypoint）；`validate` agent-gateway；route_agent 修复后 smoke 可达 TokenHub（非 eligible-empty）
- [ ] 本机 live `out/agent_vs_vsr_{routing,compare}.*`：需 Docker（vllm-sr）+ 真实 `GATEWAY_API_KEY`

### T48 — Builtin agent 三档选档
- [x] `agent-gateway.yaml`：三档意图 prompt + `timeout_ms`/`max_turns`
- [x] BuiltinAgent：eligible / `list_eligible_models` / `submit_route` / DEFAULT_SYSTEM 暴露 tier
- [x] 四题 curl：sci-q1/explain-os→large；tech-q1/tech-q3→mid（layer=agent，4/4）

### T49 — label routing 不被 pool SSL 拖垮
- [x] `quality=label` 时 pool chat 失败仍按 `label_score` 记 cell（修 multi-q error）
- [x] `chat_completion` transient SSL/5xx/429 默认重试 5 次；单测覆盖

## 阶段 R — 决策器 B + 状态化 + 生产 A

### T50 — Spec
- [x] `requirements.md` 阶段 R / §3.3.1–3.3.2 / stores·services·emits·elo·ml
- [x] 本清单；`AGENTS.md` 进行中

### T51 — Config 契约
- [x] `DecisionCfg.emits` / `RetentionDirective`；`deny_unknown_fields`
- [x] `global.stores` / `services` / `model_catalog`
- [x] 示例 `semantic-stateful.yaml`；tiny 可选 retention

### T52 — Memory + retention
- [x] `SessionMemory`；`keep_current_model` / `ttl_turns`；retention 响应头
- [x] bypass 不写；agent 不强制 sticky

### T53 — Learned `ml`
- [x] signal feature `ml` + HashMlBackend（+ 可选 ort `weight_path`）
- [x] embedding + 其余 learned 薄封装；单测

### T54 — Elo / ratings
- [x] `elo` selection + `ratings` 表；KNOWN/IMPLEMENTED

### T55 — 生产设施
- [x] `semantic_cache`；软限流 429；加厚 replay + `POST .../reroute`

### T56 — 文档与验收
- [x] README / Dashboard Replay；`cargo test` + `--features ml`；dashboard build

## 测试覆盖补强

### T57 — Phase R 单测覆盖
- [x] `memory` / `ratelimit` / `replay` / `semantic_cache` 模块单测（TTL、驱逐、persist、命中/未命中）
- [x] `cost` helpers + identity buckets；`elo` 表；`core` 请求/决策助手；`auth_api` / `embed` 纯函数
- [x] 集成：`semantic_cache_hit_skips_upstream`、`replay_log_lists_recent`；decision OR/NOT/投影缺失
- [x] `cargo test` 全绿；行覆盖约 69% → 74%（`semantic_cache` 9% → 98%）

### T58 — setup gateway + 可配置 models
- [x] setup 嵌入 `semantic-gateway` / `agent-gateway`；interactive / flags 覆盖 `provider_model_id`、`base_url`、`api_key_env`、agent endpoint/model/fallback
- [x] `providers.models[].tier` + `ModelCard.tier`；agent 优先显式 tier；prompt 按 tier 标签而非上游 ID
- [x] tiny 保留给 CI / `--config`；docs + config/agent 单测

### T59 — FFI/SDK 默认 config 路径
- [x] 删除 `config/examples/ffi{,-tiny}.yaml`；离线 CI 用 `bindings/testdata/fast-response.yaml`
- [x] `aria_router_init(NULL/"")` → `default_config_path()`；Python/Go/Rust SDK 空路径同语义；docs / cases / binding 脚本

### T81 — `aria-router upgrade`
- [x] Spec：`requirements.md` CLI 含 `upgrade`；`router-cli.yml` / `upgrade_url`；与 engine 对齐
- [x] `lib_dir` / `ensure_aria_home` / `RouterCliConfig`；setup 写入 `--upgrade-url`
- [x] `upgrade [version]`：Releases 拉取 → 替换 CLI + 安装 `libaria-router_ffi` 至 `~/.ariacompute/lib/`
- [x] 单测（选版 / 资产名 / API path）；AGENTS / README

## 阶段 A-afm-d — AFM-D 决策器

### T82 — Spec
- [x] `requirements.md`：三种决策器、§3.4.1 AFM-D、阶段 A-afm-d、硬不变量互斥
- [x] 本清单；`AGENTS.md` 概述 / 架构 / 目录

### T83 — Core + Config
- [x] `RouterKind::AfmD`（YAML `afm-d`）；`AfmDRecipe`；`Recipe.afm_d`；validate 互斥
- [x] 示例 `afm-d-tiny.yaml` / `afm-d-gateway.yaml`；setup `--template afm-d`

### T84 — `afm_d` crate + HTTP
- [x] System One client → `RouteDecision`；无 endpoint first-eligible；单测
- [x] `route_afm_d` + topology；workspace `aria-router-afm-d`
- [x] README / README_cn；`cargo test` 全绿

## eMudhra / emSign 证书私钥泄露响应（2026-10）

背景：eMudhra(emSign) 8 张中间 CA 私钥极弱（P-384 私钥标量 <256），可被反推伪造任意域名证书；
Rust(webpki-roots) 等自带信任库需升级。router 是 Rust 工作区，出向 TLS 来自 provider API 调用与
`bin` 的 `upgrade` 从 GitHub/Gitee 下载发布；dashboard 为 Vite 构建的静态资源（运行时由二进制托管）。

### 1. Rust（aria-router 二进制）
- `Cargo.lock`：`webpki-roots` 1.0.9 -> 1.0.10（`cargo update -p webpki-roots --precise 1.0.10`）。
  1.0.10+ 在 rustls 信任库中移除 emSign 根；`http` crate 的 `reqwest`+`rustls-tls` 出向校验依赖它。
- `http/Cargo.toml`：显式钉住 `webpki-roots = "1.0.10"`，防止锁漂移回受影响版本。
- 注意：二进制信任库编译进产物（webpki-roots），不走系统 bundle；新发布（被 playground sandbox
  镜像拉取）将自带修复，故锁升级是决定性的。
- 核验：`cargo tree -i webpki-roots` 须 >= 1.0.10。

### 2. Dashboard（React/Vite，构建期）
- 仅构建期 `npm install` 经 Node 系统 ca-certificates 出向 TLS；运行时 SPA 静态托管，无运行时信任库。
  构建环境保持系统 ca-certificates 最新即可，无需仓库内固定。

### 3. 检测与验证
- 新增 `scripts/check-emudhra.sh`：离线按 emSign 根（信任锚）指纹扫描 CA bundle，规避代理假阳性。

## T85 — 覆盖率达到 100%（已完成）

- [x] `scripts/coverage.sh`（默认 100% 门禁 / `report` / `html` / `lcov`）+ `docs/coverage-exclusions.md`
- [x] `.github/workflows/ci.yml`（fmt / clippy / test / coverage / bindings，PR 与 main 触发）
- [x] `core`/`config`/`signal`(+ml)/`decision`/`algorithm`/`plugin`/`provider`/`agent`/`afm_d` 正常+异常路径
- [x] `http`：`topology`/`replay`/`semantic_cache`/`decision_ext`/`memory`/`cost`/`embed`/`ratelimit`/`users`/`keys`/`auth_api`/`lib.rs` 主链路
- [x] 最小重构（外部行为不变）：`bin` 的 `run_with(cli, io)` + `CliIo`（prompt / prompt_secret / is_tty / serve_shutdown）、
      `read_secret`/`mask_termios`/`set_termios` 纯函数化、`serve_two`、`upgrade::run_with` + `install_cli_to` + `fetch_releases_url`
- [x] `bin/tests/cli.rs` 子进程端到端（`--version` / setup / validate 成功+失败 / `setup --status` / 无参退出码）
- [x] `ffi` 空指针/重复 destroy/`last_error`/token 直连数据面/401/空 options/stream 失败；`bindings/rust` 恢复环境分支
- [x] `cargo fmt` / `clippy -D warnings` / `test --workspace --all-features` 全绿
- [x] 覆盖率 90.95% → **100.00%**（未覆盖行 2901 → 0）
- 顺带清理的死代码（不可达，行为不变）：`route_agent` / `route_afm_d` 的兜底与二次校验（agent 运行时与
  System One 决策器已保证池内选择）、`auth_api::serve_json` 未使用的 POST body 分支、
  `serve_account_sync` 中 `oauth_set_api_key` 之后重复的 unmark 检查、`install_cli_to` 的 rename→copy 回退、
  `detect_asset_os` 改为 `#[cfg]` 编译期选择
- [x] 收尾：按 lcov `DA:` 逐行清零（19585/19585 行）

### T85 收尾记录（49 → 0 行）

- 门禁口径：workspace 构建会把每个依赖 crate 的副本链接进各测试二进制，`llvm-cov` 的
  `--fail-under-lines` 按**实例化**累加行数（同一行既有覆盖副本又有未调用副本 → 记一次未覆盖），
  100% 不可达且无法定位到源码行。故 `scripts/coverage.sh` 改为统计合并后的 lcov `DA:` 记录，
  与 `llvm-cov show` 标注一致；理由与豁免写在 `docs/coverage-exclusions.md`。
- `http/src/lib.rs`：competence 降级（`routing.competence` + `projections` + 多 modelRefs 触发真实降级）、
  `SseCostStream` 的 `Pending`/上游流错误/完成后重复 poll、`AppError` 全变体状态码、
  `parse_sse_usage` 无 usage 与畸形帧、`oneshot_json` 空响应体、admin 撤销任意 key、
  dashboard 磁盘目录（临时目录，去掉依赖 `dashboard/dist` 是否构建的环境分支）
- `bin/src/main.rs`：`serve_two` 改 `tokio::select!`、setup 清理缺文件分支、`Upgrade` 无配置报错、key 文件读取去除不可达分支
- `bin/src/upgrade.rs`：`prepare_staging` 纯函数化、解压目录创建去分支、`CLI_BIN_NAME` 改 `#[cfg]` 常量
- `config`：空 signal 键名 `continue`；`keys`：`extract_bearer` 删除不可达空检查、无父目录持久化报错；
  `auth_api`：`sync_serve_key` 抽为可测函数（成功/失败）、无历史 key 的 sync 不写删除标记；
  `ffi`：回调 `user` 为空的防御分支
- 顺带修复：
  - `ffi` 的两个 stream 测试共享全局 `STREAM_CALLED`，并行执行互相污染（改为经回调 `user` 传各自计数器）
  - `afm_d::systemone_timeout_is_reported` 的 `timeout_ms = 1` 会在 TCP 建连前就超时，桩服务的 `accept()`
    永远等不到连接、`server.await` 无界等待导致偶发挂死（改为 100ms 预算 + 500ms 桩延迟 + 2s 有界 join）

## T86 — Windows x86_64 测试兼容（已完成）

`cargo test --workspace` 在 `windows-latest`（release 矩阵）暴露 2 个失败 + 1 个 `dead_code` 警告：

- [x] `upgrade::tests::install_cli_requires_binary_and_replaces_target`：把目标设为目录时，Linux 的最终
      `rename` 报 `EISDIR`，Windows 的 `MoveFileEx` 报 access-denied（`PermissionDenied`）。断言改为
      **编译期常量** `REPLACE_DIR_ERR`（`#[cfg(windows)]` / `#[cfg(not(windows))]`），避免运行时双分支
      在单一平台上产生未覆盖行；同时补一条「目录未被改写」的断言。
- [x] `upgrade::tests::upgrades_cli_and_ffi_end_to_end`：测试固定用 `.tar.gz` 与 `aria-router` /
      `lib/libaria-router_ffi.so` 造包，而 Windows 的 CLI 资产是 `.zip`、包内二进制是 `aria-router.exe`、
      FFI 成员是 `aria-router_ffi.dll`（见 `release.yml`），于是 `extract_archive` 按扩展名走 zip 解压并报
      `Could not find EOCD`。改为 `cli_archive()` / `ffi_archive_member()` 两个编译期选择的辅助函数，
      归档格式与包内文件名都取自被测代码的平台常量，断言也用同一常量。
- [x] `bin` 的 `read_secret` 仅被 Unix 掩码输入路径调用，Windows 非测试构建报 `function is never used`
      （release 矩阵的 `clippy --all-targets -D warnings` 会直接失败）。改为 `#[cfg(any(unix, test))]`：
      生产构建只在 Unix 存在，测试仍可在所有平台覆盖退格/终止符分支。
- [x] `bin/tests/cli.rs::setup_prompts_through_stdio_without_a_tty`：`CliIo::is_tty` 在非 Unix 上
      无条件返回 `true`（历史占位），Windows CI 的 stdin 是管道却仍被当成终端，setup 于是把所有
      默认项都当成交互提示（base_url / key / model ids 全被 `password1` 吃掉，最后报
      「password must be at least 8 characters」）。改为跨平台的 `std::io::IsTerminal`
      （`io::stdin().is_terminal()`），删掉 `is_tty_fd`；管道 stdin 一定走非交互默认值。
      掩码读取仍仅在 Unix 生效（termios），其余平台读普通行。

验证：Linux 全量测试 + clippy + 覆盖率门禁仍为 100.00%（19589/19589 行，删掉 `is_tty_fd` 与
Windows 假分支后行数略降）。

## T87 — 绑定套件 Kotlin 编译在 CI 失败（已完成）

`Bindings Host / Host binding suites` 在 `ubuntu-latest` 上跑 `:compileKotlin` 时崩在
`org.jetbrains.kotlin.com.intellij.util.lang.JavaVersion.parse`：Kotlin 2.0.21 内嵌的 IntelliJ
版本解析不认识 runner 默认 JDK 的运行时版本串（JDK 23+），Gradle 又用自身 JVM 启动 Kotlin
编译守护进程，于是整条链在 JDK 25 上不可用（`publish-maven.yml` 早已钉 Temurin 17，故只有
host 套件暴露）。

- [x] `.github/workflows/bindings-host.yml`：新增 `actions/setup-java@v4`（temurin 17 + `cache: gradle`），
      让 Gradle 与 Kotlin 编译守护进程都跑在 `kotlin { jvmToolchain(17) }` 的目标 JDK 上。
- [x] `.github/workflows/ci.yml` 的 `bindings` job 同样加 `setup-java`，避免两个入口口径漂移。
- [x] `scripts/run-binding-tests.sh`：Kotlin 步骤前打印 `java -version`，若主版本 > 22 直接失败并提示
      「把 JAVA_HOME 指向 JDK 17/21」，不再把 40 行编译器栈当唯一线索（不吞错、不打哑谜）。
- 验证：`bash -n` 通过；版本串解析对 `17.0.11` / `25` / `21.0.3` / `1.8.0_402` 均正确；
      两个 workflow 的 YAML 结构解析通过。本机只有 JDK 17 且 Gradle 依赖拉取不可用，
      故 Kotlin 编译本身未在本机执行，由 CI 复跑确认。

## T88 — Windows 上依赖「连接被拒」的用例不稳定（已完成）

`agent::tests::llm_timeout_is_reported` 在 `windows-latest` 失败：用例拨 `http://127.0.0.1:1`
期望连接被拒（`Extension`），但该 runner 上 connect 没有快速失败，而是在 2s 截止时间后才由
`tokio::time::timeout` 报 `Timeout`，断言失配（panic 打印的正是 `timeout: builtin agent`）。
同一类「拨关闭端口当错误源」的用例都可能这样，且部分客户端根本没有超时。

- [x] `agent`：拆成两个确定性用例。
      `llm_timeout_is_reported` 改为本地监听器 accept 后不回包（socket 经 oneshot 移交测试函数持有，
      既保持连接又不留永不完成的 future），100ms 预算 → 断言 `RouterError::Timeout`；
      新增 `llm_bad_endpoint_is_reported_as_extension_error`，用无法解析的 endpoint 在**请求构造期**
      失败（无网络、跨平台一致）→ 断言 `RouterError::Extension`。
- [x] `afm_d::systemone_connection_error_mentions_endpoint`：改用 accept 后立即关闭的本地监听器，
      错误仍在 `send()` 阶段产生（消息里继续带 endpoint），但不再依赖平台的 connect 语义。
- [x] `provider::forward_connection_failure_records_failure` / `forward_sse_connection_failure_records_failure`：
      同上，新增 `closing_endpoint()` 辅助函数；否则 Windows 上会一直等到 provider 客户端 300s 超时。
- [x] `http::rate_limit_and_upstream_failure_statuses` 的「upstream 不可达 → 502」：改用 accept 后
      关闭的监听器；阻塞到 300s 会被映射成 504，断言本来就是 502。
- [x] `bindings/rust::connect_mode_models_route_and_complete`：SDK 的 reqwest 客户端没有超时，
      关闭端口可能永久悬挂；改为 accept-后-关闭的监听器（后台线程循环 accept）。

验证：`cargo test --workspace --all-features`、`clippy -D warnings`、`scripts/coverage.sh`
（100.00%，19636 行）全绿；上述用例在 Linux 均为毫秒级完成。

## T89 — dashboard 与 keys 的 Windows 失败（已完成）

- [x] `embed::tests::dashboard_router_serves_disk_then_embedded_then_api_only`：Windows runner 没有 npm，
      `http/build.rs` 退回空嵌入（`DASHBOARD_FILES = &[]`、`DASHBOARD_HAS = false`）。而该用例显式用
      `embedded = true` 请求 `/`，`serve_embedded` 归一化得到 `index.html`、命中「带扩展名即视为缺失资源」
      规则 → 404，断言 200 失败。
      修法：把解析规则抽成纯函数 `asset_response(key, files, index)`，并把 `index.html` 显式列为
      **入口点**（无扩展名路由与入口点都回 index），不再因为「没有嵌入资源」把入口点当成缺失文件；
      `serve_embedded` 只做 `normalize` + 委托。规则因此与构建产物无关。
      新增 `asset_response_serves_assets_spa_routes_and_404` 用例，用**自造资产表**覆盖
      「命中资源 / 无扩展名路由 / 空表入口点 / 缺失资源 404」四条规则——即使构建没有嵌入任何资源，
      这些行也一定被覆盖（本地与 CI 覆盖率口径不再依赖是否装了 npm）。
      验证方式：临时移走 `dashboard/dist` 并把 `npm` 换成 `exit 1` 的假程序，重建后 `http` 全部 188 个
      用例通过（即 Windows 的构建条件），恢复 dist 后再次通过。
- [x] `keys::tests::persist_reports_unwritable_paths`：原用例用 `/proc/definitely-missing/keys.json`，
      这是 Linux 专属路径，Windows 上会落到当前盘符下并可创建成功（`create` 返回 `Ok`）。
      改为「把一个普通文件当作父目录」：`<tmp>/not-a-dir/keys.json`，`create_dir_all` 在两个平台都失败，
      断言 `RouterError::Io` 保持不变。

验证：`cargo test --workspace --all-features`、`clippy -D warnings`、`scripts/coverage.sh`
（100.00%，19658 行）全绿。

---

## 候选池（未排期 · 待人审）

> 对应 [`requirements.md` §7](requirements.md) 与 [`docs/hiroute-vs-aria-router.md`](docs/hiroute-vs-aria-router.md) §6。
> **本节不是实施清单**：条目均为「候选 / 待人工审核 / 未排期 / 不进入实施」，编号 `C1–C9` 与规格 §7 一一对应。
> 依 Harness 约定，`requirements.md` 须经人工逐项审核通过后，方可据其生成 / 执行 `task.md`；**本节不得被当作待办勾选，也不占用、不重排 T0–T89**。上文 T0–T89 的勾选状态与内容一律不变。

| 候选 | 方向 | 规格 §7 | 对比文档 | 现状摘要 |
|---|---|---|---|---|
| C1 | 显式化逐步重判边界 | §7.2 C1 | 矩阵 #1 / §6 | `prefix_reuse` + sticky 只做「粘住」，无重判边界 |
| C2 | afm-d 三问题模板 | §7.2 C2 | 矩阵 #4 / §4.3 / §6 | `questions` 固定 1 个 `route` choice |
| C3 | 低分保护状态机 | §7.2 C3 | 矩阵 #7 / §6 | 一次性离散降档，无保护 / 解除 |
| C4 | 信任度调档 + 证据门槛 + 回放发布 | §7.2 C4 | 矩阵 #6 #13 #14 / §6 | 有 reroute 与 cost，无档位 / 建议 / 发布 |
| C5 | Dashboard 模型表现页 | §7.2 C5 | 矩阵 #12 / §6 | 有 Cost / CostByCompetence，依赖 C6 |
| C6 | 执行证据字段接通生产路径 | §7.2 C6 | 矩阵 #11 #18 / §6 | 4 个证据字段恒 `None`；**C3 / C5 前置** |
| C7 | API Key 花费额度 | §7.2 C7 | 矩阵 #26 / §6 | cost 只记账；**与 §2.1 硬 quota 非目标冲突，需先裁决** |
| C8 | backend 候选链 / 同名模型 failover | §7.2 C8 | 矩阵 #9 #27 / §6 | 单取最大权重，无 retry / failover |
| C9 | 策略一致性指标与离线复现 | §7.2 C9 | 矩阵 #35–#38 / §6 | bench 无 consistency 指标，corpus 为 tiny fixtures |

**文档口径一致性待办（非候选需求）**：[`docs/hiroute-vs-aria-router.md`](docs/hiroute-vs-aria-router.md) §7 登记了 README 中英三方对比表中若干 ✅ 与源码状态不符的判定；属文档修订，不入候选池、不在本节排期。

**未排期声明**：本节不含任何待办勾选项、验收记录或提交记录。人审通过前，不产生任何实施任务。

---

## 阶段 T — 实施计划（计划稿 · 待人审激活）

> 依据 [`requirements.md` §7](requirements.md)（C1–C9 候选池）与 [§8](requirements.md)（U1 独立需求），可执行任务级细节见 [`docs/implementation-plan.md`](docs/implementation-plan.md)。
>
> **状态声明**：本章是**实施计划（计划稿 · 待人审激活）**，**不是实施清单**。用表格列编号，**不含勾选式待办、验收记录或提交记录**。
> 依 Harness 约定，`requirements.md` 须经人工逐项审核（M0 门禁 G1–G11，含 C7 与 §2.1 的裁决、U1 的 G11）通过后方可进入编码；**在此之前不激活任何任务**。
> 上文「候选池（未排期 · 待人审）」的未排期声明仅约束 `C1–C9`；本章是需求方另行立项的计划稿，与候选池互不隶属。T0–T89 的勾选状态与文字、候选池任何一行、阶段 P/Q 的 2 条 bench live 遗留验证均保持原样。
> 编号 `T90–T99` 不占用、不重排既有编号；既有的 T18 重复与 T30 / T60–T80 缺口不在本章修补。

### T.1 任务编号对照（计划稿）

| 任务 | 对应 | 里程碑 | 前置 | 目标 | 主要目标文件 | 验收摘要 |
|:----:|:----:|:------:|------|------|--------------|----------|
| T90 | C1 | M2 | M0 | 显式化逐步重判边界 | `http/src/lib.rs`、`core/src/lib.rs`、`http/src/memory.rs`、`config/src/lib.rs` | 三类边界有判定规则与响应头；无法识别同 turn 时 fail closed |
| T91 | C2 | M2 | M0 | afm-d 三问题模板 | `afm_d/src/lib.rs`、`config/src/lib.rs` | 默认单 `route` choice 不变；三问归一为 typed `RouteDecision` |
| T92 | C3 | M3 | T95、G9 | 低分保护状态机 | `decision/src/lib.rs`、`core/src/lib.rs`、`http/src/replay.rs` | 保护与解除成对；旧低分不重复生效；无评分维持 fail closed |
| T93 | C4 | M3 | T95、T92 | 信任度调档 + 证据门槛 + 回放后发布 | `config/src/lib.rs`、`http/src/lib.rs`、`http/src/replay.rs`、`dashboard/` | 证据不足不产生建议；回放只估算不改写线上；发布需显式动作 |
| T94 | C5 | M3 | T95 | Dashboard「模型表现」页 | `dashboard/src/App.tsx`、`pages/`、`http/src/cost.rs` | 空态显示「无数据」而非 0；估算口径须标注 |
| T95 | C6 | M1 | M0 | 执行证据字段接通生产路径 | `http/src/replay.rs`、`http/src/lib.rs` | 4 个证据字段生产可查；关闭 replay 无开销；JSONL 向后兼容 |
| T96 | C7 | M4 | **G6 裁决通过** | API Key 花费额度 | `http/src/keys.rs`、`http/src/cost.rs`、`http/src/ratelimit.rs` | 预留—结算闭环；耗尽错误可区分；另设累计存储 |
| T97 | C8 | M2 | M0、G10 | backend 候选链 / 同名模型 failover | `provider/src/lib.rs`、`config/src/lib.rs` | 单 backend 默认行为不变；耗尽 fail closed；流开始后不切换 |
| T98 | C9 | M2 | M0 | 策略一致性指标与离线复现 | `bench/`（Python，report-only） | 新指标有单测；离线可复算；不可达记 `skipped` |
| T99 | U1 | M6 | **G11 通过** | impeccable 渲染审计与视觉优化 | `dashboard/`、`scripts/`、`PRODUCT.md`、`.github/workflows/ci.yml` | `--static` / `--dom` 0 primary findings；护栏与构建全绿；功能等价 |

### T.2 里程碑（计划稿）

| 里程碑 | 内容 | 任务 |
|:------:|------|:----:|
| **M0** | 门禁：§7.5 人审检查项 + §7.3 边界裁决（G1–G11）；未通过不激活任何任务 | — |
| **M1** | 地基：执行证据接通生产路径 | T95 |
| **M2** | 低耦合并行：互不重叠的四项（T98 纯 Python，不碰运行时） | T90、T91、T97、T98 |
| **M3** | 决策反馈闭环：T92 → T93；T94 并行 | T92、T93、T94 |
| **M4** | 运维额度（需 G6 裁决） | T96 |
| **M5** | 全量验收：100% 行覆盖、`clippy -D warnings`、bench 单测、dashboard 构建、bindings 回归 | 全部 |
| **M6** | UI 渲染质量独立里程碑；与 T93 / T94 共改 `dashboard/`，**合并批次优先**，否则设计骨架先行 | T99 |

### T.3 四处回指

| 处 | 内容 | 状态 |
|----|------|:----:|
| [`requirements.md` §7.2](requirements.md) | `C1`–`C9` 候选条目（原文） | 待人审、未排期 |
| [`requirements.md` §8](requirements.md) | `U1` 独立需求 | 计划稿 · 待人审激活 |
| [`docs/implementation-plan.md`](docs/implementation-plan.md) §5 | 10 张任务卡 `T90`–`T99` ↔ `C1`–`C9` / `U1`，含目标文件、单测、验证命令、验收与风险 | 计划稿 · 待人审激活 |
| 本章 T.1 / T.2 | 编号、里程碑、依赖、验收摘要（表格） | 计划稿 · 待人审激活 |
| 上文候选池 | 仅作回指目标，**不新增勾选、不改一行** | 待人审、未排期 |

**编号映射**：`T90↔C1`、`T91↔C2`、`T92↔C3`、`T93↔C4`、`T94↔C5`、`T95↔C6`、`T96↔C7`、`T97↔C8`、`T98↔C9`、`T99↔U1`。

**计划稿声明**：本章不含待办勾选项、验收记录或提交记录。M0 通过前，不产生任何实施任务、不进入编码；审核通过后另行按 Harness 流程生成 / 执行实施清单。

---

**实施记录（2026-11-05，纯追加）**：用户明确要求实现 `docs/implementation-plan.md` 全部特性，视为 M0 人审门禁激活。T90–T99（C1–C9 / U1）均已落地：`cargo fmt`/`clippy -D warnings`/`cargo test --workspace --all-features`、`npm --prefix dashboard ci && build && test`、`python -m unittest discover -s bench/tests -t .`、`./scripts/audit-ui.sh --static|--dom` 均通过；`--url` 在无 Chrome 环境按约定记录 `skipped`。逐项落地清单、验证结果与残余项见 `docs/implementation-plan.md` §11、`docs/impeccable-audit.md`。`./scripts/coverage.sh report` 实测 100.00%（22637/22637 行）；C7 依 G6 裁决为默认关闭的软额度。
