# aria router 特性实施计划（计划稿 · 待人审激活）

> **基线日期**：2026-11-04。**依据**：[`requirements.md`](../requirements.md) §7（HiRoute 对标候选 C1–C9）与 §8（Router Web UI 独立需求 U1）、[`task.md`](../task.md) 候选池、[`hiroute-vs-aria-router.md`](hiroute-vs-aria-router.md) 特性矩阵、[`AGENTS.md`](../AGENTS.md) 命令与硬约定，以及 2026-11-04 的源码核实结果。
>
> **状态声明（勿略过）**：本文是**实施计划（计划稿 · 待人审激活）**，不是已批准的实施清单。
> 依 `requirements.md` 第 3 行「须经人工逐项审核，审核通过后方可据其生成 / 执行 `task.md`」与 `AGENTS.md` 的 Harness 约定：
>
> - **M0 人审门禁未通过前，不进入编码**，不把 C1–C9 / U1 变成已激活待办；
> - §7 候选池、`C1–C9` 编号、T0–T89 的勾选状态与文字**保持原样**；
> - 本文与 `requirements.md` §8、`task.md` 阶段 T 统一使用「计划稿 · 待人审激活」标注，编号用表格表达而非勾选清单；
> - **本次交付是纯文档**：只生成/更新本文、`requirements.md` §8、`task.md` 阶段 T。任务卡中描述的「新增前端依赖、修改 CSS/组件、运行 impeccable 扫描、视觉重做」属于**人审通过后的实施阶段**，不在本次交付范围。

---

## 1. 范围与排除项

### 1.1 范围内

| 编号 | 任务编号 | 里程碑 | 名称 | 来源 |
|:----:|:--------:|:------:|------|------|
| C1 | T90 | M2 | 显式化逐步重判边界 | `requirements.md` §7.2 |
| C2 | T91 | M2 | afm-d 三问题模板 | §7.2 |
| C3 | T92 | M3 | 低分保护状态机 | §7.2 |
| C4 | T93 | M3 | 信任度连续调档 + 证据门槛 + 回放后发布 | §7.2 |
| C5 | T94 | M3 | Dashboard「模型表现」页 | §7.2 |
| C6 | T95 | M1 | 执行证据字段接通生产路径 | §7.2 |
| C7 | T96 | M4 | API Key 花费额度 | §7.2 |
| C8 | T97 | M2 | backend 候选链 / 同名模型 failover | §7.2 |
| C9 | T98 | M2 | 策略一致性指标与离线复现 | §7.2 |
| **U1** | **T99** | **M6** | **impeccable 渲染检查并优化 router web 页面** | `requirements.md` §8（并列独立需求） |

### 1.2 明确排除（不排期、不勾选）

- **Router Desktop / Tauri v2 / Vue 3 / `desktop/`**（上一轮需求已回退，当前文件无 §8 Desktop / 阶段 S）。
- `task.md` 中 2 条历史遗留 `- [ ]`：阶段 P 的冷启动 `compare`（`acc ≥` 且 `p50/mean ≤ vllm`）、阶段 Q 的本机 live `out/agent_vs_vsr_{routing,compare}.*`（需 Docker + 真实 key）。
- 对比文档 §7 的 README 中英三方对比表口径修订（属文档修订，§7.4 明示不入候选池）。
- HiRoute 的准确率 / 成本 / 耗时数字：事后复核子集、衡量策略符合度（**非**任务完成率）、API 等价估算，**仅作背景，不作为任何任务的验收阈值**。
- 全量 T0–T89 回顾式路线图。

### 1.3 编号基线（已核实）

- `task.md` 最大编号 **T89**，无 `T90+`；编号存在历史缺口（T30、T60–T80 缺失）且 **T18 重复** —— 本计划**不重排、不修补**既有编号。
- `requirements.md` 章节止于 §7.5，无 `## 8.` / `## 9.` —— §8 可安全新增。
- `docs/` 原无 `implementation-plan.md`（本文为新建）。

---

## 2. M0 门禁：人审与裁决

**M0 未通过 → 不激活任何任务。** 产出物为一份人审结论（记录通过/驳回/待补材料），以及下表裁决项的决定。

| # | 检查 / 裁决 | 依据 | 未通过时的行为 |
|---|-------------|------|----------------|
| G1 | 逐条确认 C1–C9 是否在产品边界内 | §7.5 (1) | 驳回出界条目，其余继续 |
| G2 | 确认 §1.2 硬不变量在任何候选下都不被破坏 | §7.5 (2) | 驳回对应设计，重做方案 |
| G3 | 确认 C6 为 C3 / C5 的前置、排序合理 | §7.5 (3) | 调整 §3 里程碑 |
| G4 | 确认 HiRoute 数字未被写成验收目标值 | §7.5 (4) | 修订任务卡验收标准 |
| G5 | 确认 §8 / U1 与 §7 互不隶属、同受本门禁约束 | 用户确认的「并列独立需求」 | 明确隔离 |
| **G6** | **C7 与 §2.1「硬 quota（软限流 429 允许）」为非目标的冲突** | `requirements.md:67`、§7.3、§7.5 (1) | **C7 不激活**；或裁决为「仅记账不拦截」的软额度 |
| G7 | C1 的「重判」不违反「一次请求禁止串跑多种决策器」 | §1.2、§7.3 | 限定为跨请求边界 |
| G8 | C2 三问答案归一为单个 typed `RouteDecision` 的规则 | §1.2「检测 ≠ 执行」、§7.3 | 驳回直至归一规则明确 |
| G9 | C3 / C4 的评分与信任度属 Preference，不得覆盖 Location | §1.2、§7.3 | 加硬剪枝顺序约束 |
| G10 | C8 按序重试与「无合格路径 fail closed」的边界 | §1.2、§7.3 | 明确重试上限与终止语义 |
| G11 | U1 是否允许为 `dashboard` 新增 vitest / jsdom 等 devDependency 与 CI job | §8.8 | 若不允许，U1 降级为「一次性检查 + 最小修复」并重新入审 |

> §7.5 (5) 原文：「审核通过后，另行按 Harness 流程生成 / 执行 `task.md`；在此之前本章保持「未排期」」。本计划的 `task.md` 阶段 T 只是**计划稿表格**，不构成该条所述的「生成 / 执行」。

---

## 3. 里程碑、依赖与并行度

| 里程碑 | 内容 | 任务 | 前置 | 并行度 |
|:------:|------|------|------|--------|
| **M0** | 人审检查项 G1–G11 与 §7.3 边界裁决 | — | 无 | 门禁，串行 |
| **M1** | 地基：执行证据接通生产路径 | C6 (T95) | M0 | 单任务，优先落地 |
| **M2** | 低耦合并行批次 | C1 (T90)、C2 (T91)、C8 (T97)、C9 (T98) | M0 | 四者可并行；C9 纯 Python 完全不碰运行时 |
| **M3** | 决策反馈闭环 | C3 (T92) → C4 (T93)；C5 (T94) | M1 (C6)；C4 另依赖 C3 | C5 可与 C3/C4 并行，但见下方 dashboard 冲突 |
| **M4** | 运维额度 | C7 (T96) | M0 的 G6 裁决通过 + 成本记账基础 | 与 M2/M3 隔离 |
| **M5** | 全量验收门禁 | 全部任务 | M1–M4、M6 | 见 §7 |
| **M6** | UI 渲染质量独立里程碑 | U1 (T99) | M0 的 G11 通过 | 与 M2 无冲突；**与 C4/C5 有 dashboard 文件冲突** |

**依赖链**：`M0 → C6 → {C3, C5} → C4`，`M0 → {C1, C2, C8, C9, U1}`，`G6 裁决 → C7`。

**dashboard 冲突的合并顺序（基于源码核实，重要）**：C4、C5、U1 都会改 `dashboard/src/App.tsx`；C4/C5/U1 都会改 `dashboard/src/api.ts`；C5 与 U1 在 4 个 CSS 文件上重叠最大。两种可行顺序，按此优先级选择：

1. **首选：把 C4/C5 的 dashboard 部分与 U1 合并为同一个 PR/批次** —— 一次改完 `App.tsx`、`api.ts`、CSS、`package.json` 与 CI，避免三次改同一批文件。
2. 若必须分批：**U1 的设计 token / 工具链 / CI 骨架先落**，再由 C4 → C5 在新设计系统上加页面；否则 C5 新增页面的 CSS 会被 U1 的视觉重做推翻。

> 原计划曾写「U1 排在 C5 之后」，源码核实后调整为上述顺序（合并批次优先、U1 设计骨架先行）；理由：U1 是全量视觉重做，会重写 C5 新页依赖的样式基线。

---

## 4. 改动文件交叉影响矩阵

| 任务 | 主要改动文件（相对仓库根） |
|------|---------------------------|
| C1 | `http/src/lib.rs`(1329-1418, 1491-1545)、`core/src/lib.rs`(255, 302)、`http/src/memory.rs`、`config/src/lib.rs`(392, 642, 715) |
| C2 | `afm_d/src/lib.rs`(204, 266-313, 488, 534, 560-646, 648)、`config/src/lib.rs`(665, 672, 692-716, 825-939) |
| C3 | `decision/src/lib.rs`(133)、`core/src/lib.rs`(377, 403)、`config/src/lib.rs`(407-424)、`http/src/lib.rs`(1276-1280, 1550-1565, 1698-1702)、`http/src/replay.rs` |
| C4 | `config/src/lib.rs`、`http/src/lib.rs`(203, 208, 214, 272, 444, 517)、`http/src/replay.rs`、`dashboard/src/App.tsx`、`dashboard/src/api.ts`、新 page + CSS |
| C5 | `dashboard/src/App.tsx`(31-58, 166-181)、新 `dashboard/src/pages/*`、`dashboard/src/api.ts`、`dashboard/src/pages/page.module.css`、`dashboard/src/index.css`、`http/src/cost.rs`(130, 168) |
| C6 | `http/src/replay.rs`(36, 39, 42, 45, 48)、`http/src/lib.rs`(1267-1280, 1689-1702)、`core/src/lib.rs`(403) |
| C7 | `http/src/keys.rs`(55, 109, 348, 400, 487)、`http/src/cost.rs`(7, 10-42, 64, 89-92, 239)、`http/src/lib.rs`(数据面鉴权)、`http/src/ratelimit.rs` |
| C8 | `provider/src/lib.rs`(16, 79, 145, 260)、`config/src/lib.rs`(317-339)、`config/examples/*.yaml` |
| C9 | `bench/cli.py`、`bench/accuracy/runner.py`、`bench/routing/policies.py`、`bench/routing/runner.py`、`bench/report.py`、`bench/tests/`、`bench/corpus/README.md` |
| U1 | `dashboard/package.json`、`dashboard/src/App.tsx`、4 个 CSS、多数 `pages/*` / `components/*`、`dashboard/src/api.ts`、`dashboard/vite.config.ts`、`dashboard/index.html`、`dashboard/README.md`、`.github/workflows/ci.yml`、`.gitignore`、`scripts/`（新增）；`http/build.rs` 仅在产物路径变化时 |

**冲突热点**

| 文件 | 涉及任务 | 处理 |
|------|----------|------|
| `http/src/lib.rs` | C1、C3、C4、C5、C6、C7 | 最热点。建议 C6 先行，C1+C3 同批次合入 |
| `config/src/lib.rs` | C1、C2、C3、C4、C8 | `deny_unknown_fields` 与 `validate`（825-939）集中；C2/C8 的配置改动并入同一次 PR |
| `http/src/replay.rs` | C3、C4、C6 | C6 先定义证据字段语义，C3/C4 再消费 |
| `core/src/lib.rs` | C1、C3、C6 | 同批次 |
| `http/src/cost.rs` | C5、C7 | C7 的额度不得改变 C5 的报表口径 |
| `dashboard/src/App.tsx`、`api.ts`、CSS | C4、C5、U1 | 见 §3 合并顺序 |
| `.github/workflows/ci.yml` | U1（+C9 可选） | U1 新增 dashboard job 时一并处理 |

相对隔离：`afm_d/src/lib.rs` 仅 C2、`provider/src/lib.rs` 仅 C8、`http/src/keys.rs` 仅 C7、`bench/` 仅 C9。

---

## 5. 可执行任务卡

> 每张卡的「目标文件」「单测位置」均来自 2026-11-04 源码核实；「改动要点」「验收标准」为**计划建议**，人审时可调整。验证命令见 §7 总表，卡内只列该任务专属命令。

### T90 / C1 — 显式化逐步重判边界（M2）

- **目标**：把「新用户消息 / 上下文压缩 / 工具续跑」三类边界从隐式检测提升为显式配置与可观测响应头。
- **目标文件**：`http/src/lib.rs:1329-1418`（`prefix_reuse` 门与 sticky pin）、`core/src/lib.rs:255,302`（`DecisionCache` / `observe_and_check_compact`）、`http/src/memory.rs:17-141`（`SessionMemory::pin` / `tick_if_new_turn`）、`config/src/lib.rs:392,642,715`（`prefix_reuse` 字段）。
- **改动要点**：新增边界枚举与配置项；在冻结 / 重判两路径写出响应头；保持 `x-aria-session` / body `session` 语义不变。
- **单测（正常）**：`http/src/lib.rs` 既有 `prefix_reuse_same_turn_reuses_decision`(:3967)、`compaction_clears_sticky_allows_redecide`(:4352)、`tools_fingerprint_change_compacts_decision_cache`(:4796) 继续绿；新增「三类边界各产生预期响应头」用例。
- **单测（异常）**：无法识别同 turn / 历史不连续 → fail closed，不错误复用冻结决策。
- **验证命令**：`cargo test -p aria-router-http`；`cargo run -p aria-router -- validate --config config/examples/semantic-stateful.yaml`。
- **验收标准**：三类边界有明确判定规则与响应头；不改变既有 sticky/TTL 行为；§1.2「一次请求禁止串跑多种决策器」不被破坏。
- **依赖 / 裁决**：G7；与 C3 同批次合入 `http/src/lib.rs`。
- **源码核实修正**：`observe_and_check_compact` 已检测上下文压缩并清缓存、`tick_if_new_turn` 已按新用户轮次 tick TTL —— **C1 缺的是显式配置与可观测性，不是完全没有检测**（§7.2 C1 表述需按此收窄）。

### T91 / C2 — afm-d 三问题模板（M2）

- **目标**：利用 engine 侧 `questions` Map 的多键能力，提供「上一步评分 / 这一步难度 / 问题是否延续」三问题模板，默认保持单 `route` choice。
- **目标文件**：`afm_d/src/lib.rs:560`（`build_systemone_body`，`questions` 单键块在 `:572-578`，`synthesize_description` :582）、`:599`（`decision_from_systemone`，解析 `/answers/route` :604、`choice` :607、`min_confidence` :621）、`:648`（`task_from`）、`:266-313`（决策缓存）、`:204/237/262/513`（tier 快捷键）；`config/src/lib.rs:665`（`AfmDTrack`）、`:672`（`max_choice_options`）、`:692-716`（`AfmDRecipe`）、`:825-939`（校验）。
- **改动要点**：模板可选、默认关闭；`track` 上限（Encoder 255 / Decoder 16）仍只做候选池门控、不写入请求体；三问答案归一为单个 typed `RouteDecision`。
- **单测（正常）**：`afm_d/src/lib.rs` 既有 `body_uses_descriptions_and_synth`(:835)、`parse_choice_and_reject_unknown`(:858)、`http_choice_ok`(:926)、`task_from_builds_state`(:959) 继续绿；新增三问请求体构造与归一用例。
- **单测（异常）**：某问缺失 / 越权 choice / 超时 → fail closed 或走 `fallback`（`apply_fallback_or_err` :534）；`min_confidence_rejects`(:885) 语义不变；decoder track 超 16 仍拒绝(:989)。
- **验证命令**：`cargo test -p aria-router-afm-d`；`cargo run -p aria-router -- validate --config config/examples/afm-d-tiny.yaml`。
- **验收标准**：默认配置行为完全不变；模板启用时三问均发出且归一结果合法；不改 C ABI。
- **依赖 / 裁决**：G8；`config/src/lib.rs` 与 C1/C8 共享，配置改动合并一次。
- **源码核实修正**：`questions` 块实际在 `:572-578`（§7.2 C2 写的 `:560-597` 含 `synthesize_description`）；engine 契约文件在工作区同级 `engine` 仓，**不在本仓**，无法在本仓核实，实施前须跨仓确认。

### T92 / C3 — 低分保护状态机（M3）

- **目标**：把一次性离散降档升级为「保护触发 → 问题解除 → 释放」状态机，评分须新鲜、完整、同类目、同计划版本、同评估标准。
- **目标文件**：`decision/src/lib.rs:133`（`apply_competence`，测试模块 :232-233）、`core/src/lib.rs:377`（`CompetenceTracker`：`record` :386、`score` :395）、`config/src/lib.rs:407-424`（`CompetenceCfg`：`floor` :412、`simple_threshold` :416）、`http/src/lib.rs:1550-1565`（调用点）、`http/src/replay.rs`（评分证据）。
- **改动要点**：新增保护状态与解除条件（问题延续 < 0.5 解除，仅解除历史约束，难度仍决定组）；未评分 ≠ 0 分；旧低分不重复生效。
- **单测（正常）**：`decision/src/lib.rs` 既有用例(:435, :833) 与 `http/src/lib.rs:6367,6385` 继续绿；新增保护触发/解除成对用例。
- **单测（异常）**：部分评分不触发；评分过期 / 类目不符 / 计划版本不符不触发；无评分时维持现有 fail closed 行为；保护不得覆盖 Location 硬剪枝。
- **验证命令**：`cargo test -p aria-router-decision`；`cargo test -p aria-router-http`。
- **验收标准**：保护与解除成对、可追溯；§1.2「Preference 不得覆盖 Location」保持。
- **依赖 / 裁决**：**依赖 C6（T95）** 提供评分来源；G9；与 C1 同批次。
- **源码核实修正**：`apply_competence` 实际在 `decision/src/lib.rs:133`（§7.2 C3 的 `:121+` 为文档注释起始）。

### T93 / C4 — 信任度调档 + 证据门槛 + 回放后发布（M3）

- **目标**：档位阈值可配、证据不足不产生建议、回放只估算不改写线上路由、发布需显式动作。
- **目标文件**：`config/src/lib.rs`（新键 + `validate` :825-939）、`http/src/lib.rs:266-270`（`RerouteQ`）、`:272`（`replay_reroute_ep`，`entrypoint` 取第一个 :283-289）、`:444`（`put_config`）、`:517`（`cost_by_competence_ep`）、`http/src/replay.rs:17-97`（`ReplayRecord` / `ReplayLog` / `configure` :58 / `push` :63 / `recent` :85 / `get` :90）、`dashboard/src/App.tsx`、`dashboard/src/api.ts`、新 page + CSS。
- **改动要点**：新增档位与门槛配置；发布流程与 `PUT /v1/router/config` 的「配置即刻生效」语义隔离；建议不改变线上路由。
- **单测（正常）**：`http/src/lib.rs:2617` `replay_endpoints_preview_and_reroute`、`http/src/replay.rs:126,155,172,202,223` 继续绿；新增「门槛不足无建议」「回放不改写记录」「发布需显式动作」用例。
- **单测（异常）**：空 replay、跨 entrypoint 记录、草稿态编辑不生效、配置未知键被 `deny_unknown_fields` 拒绝。
- **验证命令**：`cargo test -p aria-router-http`；`npm --prefix dashboard ci && npm --prefix dashboard run build`。
- **验收标准**：阈值可配、建议在证据不足时不产生、回放仅估算、发布显式；不改变既有 Cost 页口径。
- **依赖 / 裁决**：依赖 C6 与 C3；G9；dashboard 部分与 C5/U1 冲突，见 §3。
- **源码核实修正**：`RerouteQ` 起于 `:266`、handler 起于 `:272`（§7.2 C4 的 `:266-320` 为区间写法，实施按 handler 定位）。

### T94 / C5 — Dashboard「模型表现」页（M3）

- **目标**：阶段评分、承担比例、费用估算按 model / tier 分列；无证据显示「无数据」而非 0。
- **目标文件**：`dashboard/src/App.tsx:31-58`（导航组）、`:166-181`（13 条路由）、新增 `dashboard/src/pages/*`、`dashboard/src/api.ts`、`dashboard/src/pages/page.module.css`、`dashboard/src/index.css`、`http/src/cost.rs:130`（`by_competence_tier`）、`:168-245`（`report`）、`http/src/lib.rs:517`。
- **单测（正常）**：`http/src/cost.rs:355` `by_competence_tier_aggregates_and_estimates_savings` 继续绿；新增页面数据绑定与空态用例。
- **单测（异常）**：无证据 / 404 / 401 / 429 的页面状态；不引入 Grafana / Prometheus（§2.1）。
- **验证命令**：`npm --prefix dashboard ci && npm --prefix dashboard run build`；`cargo test -p aria-router-http`。
- **验收标准**：空态不显示 0；按 model/tier 分列；既有 Cost 页口径不变。
- **依赖 / 裁决**：**依赖 C6**；与 U1 的 CSS 冲突见 §3。
- **口径提醒**：`http/src/cost.rs:141-146` 的 `notional_large` 权重（small 16 / mid 4 / large 1）是**硬编码启发式估算，非真实定价**；`CostEvent.competence_tier` 已在 `http/src/lib.rs:1165` 写入，故 `/v1/router/cost/by-competence` 已有 tier 数据。页面须标注估算口径，不要写成"实测费用"。

### T95 / C6 — 执行证据字段接通生产路径（M1，地基）

- **目标**：让 `ReplayRecord` 的执行证据字段在生产路径产生可查值。
- **目标文件**：`http/src/replay.rs:36`（`task_category`）、`:39`（`selected_group`）、`:42`（`tool_outcomes`）、`:45`（`accepted_output`）、`:48`（`stage_competence`）；生产恒 `None` 处 `http/src/lib.rs:1276-1280` 与 `:1698-1702`；`core/src/lib.rs:403`（`tier_of_model`）。
- **改动要点**：先定义字段语义与取值来源，再在两条生产路径赋值；不写入凭据与完整原始请求体（与 `prompt_preview` 截断惯例一致）。
- **单测（正常）**：`http/src/replay.rs:126` `evidence_fields_roundtrip`、`:155` `selected_group_derives_tier` 继续绿；新增「生产路径产生可查证据」「JSONL 向后兼容」用例。
- **单测（异常）**：关闭 replay 时无额外开销；工具调用失败、SSE 中断时字段状态明确；旧 JSONL 缺字段可读。
- **验证命令**：`cargo test -p aria-router-http`；`cargo test --workspace --all-features`。
- **验收标准**：生产路径可查证据；零新增日志敏感信息；关闭 replay 无开销。
- **依赖 / 裁决**：无前置；是 **C3 / C4 / C5 的前置**，须最先落地。
- **源码核实修正**：生产恒 `None` 的是 **4 个字段**；第 5 个 `selected_group` **已由 `tier_of_model` 在生产赋值**（`http/src/lib.rs:1277,1699`）。任务卡不得写成「5 个字段全空」。

### T96 / C7 — API Key 花费额度（M4）

- **目标**：转发前预留、完成后结算、额度用完跳过该 Key；一次性 / 日历周期补充 / 多 Key 共享。
- **目标文件**：`http/src/keys.rs:13`（`SECRET_PREFIX`）、`:55`（`KeyRecord`）、`:109`（`KeyStore`）、`:348-367`（`create` / `create_for`）、`:400-423`（`authenticate` / `resolve_bearer`）、`:487`（`revoke`）；`http/src/cost.rs:7`（`MAX_EVENTS=4096`，驱逐 :76-78）、`:10-42`（`CostEvent`）、`:64`（`by_key`）、`:89-92`、`:239`；`http/src/ratelimit.rs:10-47`（60s 窗口）。
- **改动要点**：额度字段与累计存储（**不能只依赖内存账本**）；预留—结算闭环；SSE 中断补偿。
- **单测（正常）**：`http/src/keys.rs` 测试模块(:977-978)、`http/src/ratelimit.rs:83` `window_rolls_over_after_sixty_seconds` 继续绿；新增预留/结算/多 Key 共享用例。
- **单测（异常）**：额度耗尽跳过并返回可区分错误；SSE 流中断结算补偿；额度数值不写入日志；事件驱逐后累计不丢失。
- **验证命令**：`cargo test -p aria-router-http`；`./scripts/run-binding-tests.sh`（若触及鉴权头）。
- **验收标准**：闭环可测、耗尽行为可区分、不改 `keys_path` 只存 sha256 的口径。
- **依赖 / 裁决**：**G6 裁决通过前不激活**。全仓 `*.rs` 对 `quota|spend|credit|balance` 命中 0；`budget` 仅 3 处无关文本。金额按转发流量估算（非供应商余额）。
- **风险**：`MAX_EVENTS=4096` 环形驱逐会丢历史，基于 `CostLedger` 做额度**不可行**，须另设持久化累计存储 —— 这是 §7.2 C7 未写明的前置设计。

### T97 / C8 — backend 候选链 / 同名模型 failover（M2）

- **目标**：按候选顺序尝试，首个失败接下一个；耗尽才显式失败。
- **目标文件**：`provider/src/lib.rs:16`（`PoolState`）、`:79`（`forward`）、`:126`（`forward_sse_text`）、`:145`（`forward_sse_stream`）、`:260-265`（`pick_backend`，`max_by_key(|b| b.weight)` 单取最大）；`config/src/lib.rs:317-339`（`BackendRef`，`weight` :327，默认 100 :337）；`config/examples/*.yaml`。
- **单测（正常）**：`provider/src/lib.rs:365` `pick_backend_prefers_highest_weight` 继续绿；新增「单 backend 配置行为不变」「按序尝试」用例。
- **单测（异常）**：首个 backend 超时 / 非 2xx / 流中途断开；响应已开始后**不切换模型**；耗尽后 fail closed，不能静默回落到任意模型。
- **验证命令**：`cargo test -p aria-router-provider`；`cargo run -p aria-router -- validate --config config/examples/semantic-gateway.yaml`。
- **验收标准**：默认行为不变；重试上限、幂等与终止语义有测试覆盖。
- **依赖 / 裁决**：G10；`config/src/lib.rs` 与 C2 共享，合并一次。
- **源码核实修正**：全仓 `*.rs` 对 `retry|failover` 命中 0，成立；但存在**决策级** `fallback`（`afm_d/src/lib.rs:534`、`config/src/lib.rs:632,703`），文档须区分「backend failover」与「决策器错误路径回退单模型」，勿混用。

### T98 / C9 — 策略一致性指标与离线复现（M2）

- **目标**：新增「符合声明策略」的选组一致性指标，支持对已保存决策记录离线复算；报告标注语料来源与排除清单。
- **目标文件**：`bench/cli.py:42`（子命令）、`:88-92`（`--quality`）、`:113-192`（8 个子命令）；`bench/accuracy/runner.py:36`（`run_routing_accuracy`）、**`:189-200`**（指标字段：`overall_accuracy` :194、`recall_by_expected_tier` :195、`by_expected_tier` :196、`confusion` :198、`errors` :200、`mode/ci_fail` :179-180）；`bench/routing/policies.py:155,204-209,213,235-240`；`bench/routing/runner.py:394-437`；`bench/report.py:73-85`；`bench/tests/`（11 个测试文件）；`bench/corpus/README.md:3`。
- **改动要点**：指标命名为「符合声明策略」而非任务完成率；离线模式不连 live router；外部数据集下载失败沿用现有 skip 约定，不伪造分数。
- **单测（正常）**：`bench/tests/test_routing_accuracy.py`、`test_policies.py`、`test_cli.py` 继续绿；新增 consistency 指标与离线复算用例。
- **单测（异常）**：空语料、缺字段记录、下载失败 → `skipped` 且 `ci_fail: false`、坏 CLI 参数 → exit 2。
- **验证命令**：`python -m unittest discover -s bench/tests -t .`；`python -m bench routing-accuracy --router http://127.0.0.1:8899 --corpus bench/corpus/routing_gateway_eval.json`。
- **验收标准**：新指标有单测；离线可复算；报告标注语料与排除清单；Bench 仍 report-only。
- **依赖 / 裁决**：无 Rust 改动，可完全并行；CI 加单测命令需改 `.github/workflows/ci.yml`（与 U1 的 CI 改动合并）。
- **源码核实修正**：指标字段实际在 `bench/accuracy/runner.py:189-200`（§7.2 C9 的 `:36-48` 是函数起始）；bench Python 中无同名/等价 `consistency` 指标成立（仅 `bench/build_routing_eval.py:152,159` 的英文短语）。

### T99 / U1 — impeccable 渲染检查并优化 router web 页面（M6，独立里程碑）

> **并列独立需求**，依据 `requirements.md` §8；与 C1–C9 互不隶属，同受 M0 门禁约束。**本卡描述的是人审通过后的实施内容，本次纯文档交付不创建其中任何实体文件。**

- **目标**：全量接入 impeccable 4.8.0 渲染审计工具链，按审计结论完成 `dashboard/` 视觉重做，并建立回归护栏。
- **目标文件（新增，实施阶段）**：
  - `PRODUCT.md` —— `<!-- impeccable:product-schema 1 -->` 设计契约，**按 router 运维面板定位重写**（不照抄 playground 的四个 playground / 13 语言描述）。
  - `dashboard/.impeccable/config.json` —— 引擎映射；playground 的 `frontend/.impeccable/config.json` 仅配 `.vue → html`，**router 是 `.tsx`，支持情况未核实**，见下方风险。
  - `scripts/audit-ui.sh` —— `--static` / `--dom` / `--url` / `--browser` / `--list-rules` 五模式；runner 按 **npm / node_modules** 适配（playground 用 bun，不可照抄）；环境变量对齐 `IMPECCABLE_VERSION`(默认 4.8.0)、`IMPECCABLE_BIN`、`IMPECCABLE_BROWSER`、`PREVIEW_PORT`、`AUDIT_BASE_URL`、`AUDIT_ROUTES`、`AUDIT_THEMES`、`AUDIT_OUT_DIR`(`detect-out`)、`AUDIT_PAGE_TIMEOUT`。
  - `docs/impeccable-audit.md` —— 基线记录（Tooling 表 / Detection modes / findings → 修复 → 残余项 / 复跑命令）。
  - `dashboard/tests/style.spec.*` —— 静态样式护栏（对齐 playground `frontend/tests/style.spec.js`：读 stylesheet + `index.html`，校验 token / 字号 / 效果）；`dashboard/tests/render-snapshot.spec.*` —— jsdom 渲染快照。
- **目标文件（修改，实施阶段）**：
  - `dashboard/package.json` —— 现仅 `dev` / `build` / `preview` 三个 scripts，无 test/lint/a11y；deps 为 react 18.3.1 / react-dom / react-markdown 10.1 / react-router-dom 6.28 / remark-gfm 4.0.1，devDeps 为 @types/* / @vitejs/plugin-react 4.3.4 / typescript 5.6.3 / vite 5.4.11。需新增 `test`、`audit`、`audit:dom`、`audit:url`、`audit:browser`、`audit:rules` 与 vitest / jsdom / testing-library 等 devDependency（**G11 裁决项**）。
  - `dashboard/src/App.tsx:31-58,166-181`（路由表）、`dashboard/src/api.ts`、多数 `pages/*` / `components/*`、**4 个 CSS**：`dashboard/src/index.css`(11.99KB，设计 token :10-28、dark 变量 :79-88)、`App.module.css`(3.66KB，外壳/侧栏)、`pages/page.module.css`(2.48KB，页面排版)、`components/playground/playground.module.css`(5.6KB，Playground 布局)。
  - `dashboard/index.html` —— `:2` 硬编码 `<html lang="en">`、`:7-12` 外部 Google Fonts preconnect + stylesheet（与设计契约「无 web font CDN」冲突，审计关注点）。
  - `dashboard/vite.config.ts` —— 现仅 `plugins:[react()]` 与 `server.proxy`(`/v1`、`/health` → `http://127.0.0.1:8090`)，无 base/build/test 配置。
  - `dashboard/README.md`、`.github/workflows/ci.yml`、`.gitignore`（现无 `detect-out`；已有 `**/node_modules/` :8、`dashboard/dist/` :9、`out/` :19、`lcov.info` :21）；`http/build.rs:5,74-77` 仅在产物路径变化时。
- **页面矩阵（按 router 实际，不照抄 playground）**：`App.tsx:166-181` 的 **13 条路由** —— `/login`、`/register`、`/`、`/account`、`/cost`、`/cost-competence`、`/keys`、`/users`(admin)、`/config`、`/topology`、`/providers`、`/replay`、`/playground`；× **light / dark**（`dashboard/src/theme.ts:3`、`components/ThemeSwitch.tsx:55,65`）。**不含语言维度**：`dashboard/src` 无 i18n（仅 `Account.tsx:86` 的 `toLocaleString()`），文案硬编码英文。
- **视觉重做范围**：设计 token（单色 / 直角 / 字号下限 / AA 对比度）、去渐变与不一致圆角、动效只用 transform+opacity、本地字体栈替代外部 CDN；**不改变 dashboard 功能行为**（API 契约、SSE 流式、路由响应头面板、localStorage 会话侧栏保持）。
- **单测（正常）**：style 护栏覆盖字号下限、对比度、`radius`、禁渐变 / uppercase / letter-spacing、动效属性白名单；render-snapshot 覆盖 13 路由 × 双主题快照可生成且非空；`npm --prefix dashboard run build` 全绿。
- **单测（异常）**：`audit-ui.sh` 未知参数报错退出；`--browser` 找不到浏览器退出 1 并给安装提示；`--static` 退出码 0/2；`--dom` 无浏览器可运行；`PREVIEW_PORT` 占用时拒绝复用；扫描超时有明确日志；CI 无浏览器时 `--url` 记录 `skipped`，**不伪造成功**。
- **验证命令**：`npm --prefix dashboard ci && npm --prefix dashboard run build`；`npm --prefix dashboard run test`；`./scripts/audit-ui.sh --static` / `--dom` / `--browser` / `--url` / `--list-rules`。
- **验收标准**：`--static` 0 primary findings；`--dom` 全路由 × 双主题 0 primary findings；`--url` 在有 Chrome 的机器执行并写入 `docs/impeccable-audit.md`；护栏测试与 dashboard 构建全绿；功能行为不变。
- **依赖 / 裁决**：G11；与 C4/C5 的 dashboard 文件冲突见 §3（合并批次优先）。
- **风险与待核实**：
  1. **impeccable 静态引擎对 `.tsx`/JSX 的支持未核实**（playground 只映射 `.vue → html`）。**U1 首个调查项**；降级方案：静态扫描限定 `.css` + `index.html`，元素级规则交给 `--dom`（jsdom 渲染后 DOM）与 `--url`。
  2. CI **无任何 dashboard 构建/检查 job**；dashboard 目前仅由 `http/build.rs:5,74-77` 隐式触发 `npm --prefix dashboard run build`，且失败仅告警 → CI 可能静默跳过。U1 应新增显式 job。
  3. 新增前端 devDependency 影响 `npm ci` 与 CI 时间；`detect-out` 需加入 `.gitignore`。
  4. `dashboard/index.html` 的 `lang="en"` 与外部 Google Fonts 是已可预见的审计 findings，需在 `docs/impeccable-audit.md` 记录处理结论。

---

## 6. §7 表述与源码的差异（实施前须确认，不改动 §7 原文）

| # | 条目 | §7 表述 | 源码事实 |
|---|------|---------|----------|
| 1 | C1 | 「无重判边界」 | `core/src/lib.rs:302` 已检测压缩并清缓存、`http/src/memory.rs:93` 已按新轮次 tick TTL；缺的是显式配置与可观测响应头 |
| 2 | C2 | `afm_d/src/lib.rs:560-597` 发单 choice | `questions` 块在 `:572-578`，`:582` 起为 `synthesize_description` |
| 3 | C2 | `engine/core/src/systemone.rs` 契约 | 在工作区同级 `engine` 仓，不在本仓，须跨仓确认 |
| 4 | C3 | `decision/src/lib.rs:121+` | `pub fn apply_competence` 在 `:133`（`:121-132` 为注释） |
| 5 | C6 | 4 个证据字段恒 `None` | 成立；但第 5 个 `selected_group` 生产已赋值 |
| 6 | C5 | 「模型表现/费用估算」 | `CostEvent.competence_tier` 已写入；`notional_large` 16/4/1 为硬编码启发式估算，非真实定价 |
| 7 | C4 | `http/src/lib.rs:266-320` reroute | `RerouteQ` :266、handler :272、`entrypoint` 取第一个 :283-289 |
| 8 | C9 | `bench/accuracy/runner.py:36-48` 指标 | `:36` 是函数起始，指标在 `:189-200` |
| 9 | C7 | 基于 cost 账本做额度 | `MAX_EVENTS=4096` 环形驱逐会丢历史，须另设累计存储 |
| 10 | C8 | 「无 failover」 | 成立；但决策级 `fallback` 存在，须区分 |

> 这些差异是**实施前的确认项**，不是对 §7 的修订；§7 原文在人审前保持不动。

---

## 7. 验证命令总表与 CI 门禁

### 7.1 既有 CI 精确命令（`.github/workflows/ci.yml`）

| 用途 | 命令 | 位置 |
|------|------|------|
| 格式 | `cargo fmt --all -- --check` | `ci.yml:27` |
| Lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `ci.yml:40` |
| 单测 | `cargo test --workspace --all-features` | `ci.yml:51` |
| 100% 行覆盖门禁 | `chmod +x scripts/coverage.sh` → `./scripts/coverage.sh` | `ci.yml:67-68`；脚本 `:28-29` 用 `cargo llvm-cov --workspace --all-features --ignore-filename-regex '(^|/)(build\.rs)$'`，`:33-54` `enforce_gate` |
| 绑定回归 | `./scripts/run-binding-tests.sh` | `ci.yml:105-106`、`bindings-host.yml:47-48` |
| 发布校验 | `cargo test --workspace` | `release.yml:49` |

### 7.2 仓库常用命令（`AGENTS.md:38-50`）

`cargo test` · `cargo test -p aria-router-signal --features ml` · `cargo run -p aria-router -- setup|validate|serve|upgrade` · `npm --prefix dashboard ci && npm --prefix dashboard run build` · `./scripts/coverage.sh` · `./scripts/run-binding-tests.sh` · `./scripts/run-routing-accuracy.sh` · `python -m unittest discover -s bench/tests -t .` · `python -m bench routing --router aria_router=… --router vllm_sr=…` · `python -m bench routing-accuracy --router http://127.0.0.1:8899 --corpus bench/corpus/routing_gateway_eval.json` · `python -m bench compare --corpus bench/corpus/mmlu_tiny.jsonl …`

### 7.3 本计划新增的验证入口（实施阶段）

- `npm --prefix dashboard run test`（U1 新增 vitest）
- `./scripts/audit-ui.sh --static` / `--dom` / `--url` / `--browser` / `--list-rules`
- C9 若加 bench 单测 job，与 U1 的 `ci.yml` 改动合并

### 7.4 M5 全量验收门禁

`cargo fmt` + `clippy -D warnings` + `cargo test --workspace --all-features` + `./scripts/coverage.sh`（100% 行）+ `npm --prefix dashboard ci && npm --prefix dashboard run build` + `python -m unittest discover -s bench/tests -t .` + `./scripts/run-binding-tests.sh`（仅 C ABI 变更时）。

**硬约定**：Rust edition 2021+、错误统一 `RouterError`、禁止吞错与静默 no-op、新功能须单测（正常 + 异常）、未实现 YAML 能力 `Unsupported`(501)、C ABI 变更须同步 `bindings/testdata/cases.json` 与宿主测、Bench report-only（`ci_fail:false`、不可达 → `skipped`、exit 0、坏 CLI → exit 2）、`AGENTS.md` ≤100 行。

---

## 8. 风险、回退与 §1.2 硬不变量保护

| 风险 | 影响 | 缓解 / 回退 |
|------|------|-------------|
| M0 未裁决即实施 | 绕过 Harness、C7 违反 §2.1 | G1–G11 全通过才激活；C7 单独 gate |
| `http/src/lib.rs` 六任务共改 | 合并冲突、回归 | C6 先行，C1+C3 同批次，其余排后；每批跑 `cargo test -p aria-router-http` |
| dashboard 三任务共改 | C5 新页 CSS 被 U1 推翻 | 合并为同一批次，或 U1 设计骨架先行 |
| impeccable 不支持 TSX 静态解析 | U1 静态 pass 失效 | 首个调查项；降级为 `.css` + `index.html` 静态扫描 + `--dom`/`--url` 元素级 |
| CI 无 dashboard job、构建失败仅告警 | 视觉回归静默漏过 | U1 新增显式 CI job，把 `npm --prefix dashboard run build` 与 vitest 设为必过 |
| 新增前端 devDependency | `npm ci` / CI 时间增加 | G11 裁决；不允许则 U1 降级重新入审 |
| 100% 行覆盖门禁 | 新增 Rust 分支缺测即失败 | 每任务卡已列正常 + 异常用例；用 `./scripts/coverage.sh report/html` 定位缺口 |
| 额度基于内存账本 | 4096 事件驱逐丢历史 | 另设持久化累计存储，不复用 `CostLedger` |

**§1.2 硬不变量逐条保护**

| 条款 | 相关任务 | 保护方式 |
|------|----------|----------|
| Location / auth / modality / tools 硬剪枝在决策前；fail closed | C1、C2、C3、C8 | C8 重试不得绕过 `hard_filter`；G2 审核 |
| Compute 进 pool 排名不进 eligibility | C8 | 候选链只影响 pool 内顺序 |
| Preference 不得覆盖 Location | C3、C4 | G9：评分与信任度在硬剪枝之后生效 |
| 检测 ≠ 执行；输出必须 typed `RouteDecision` | C2 | G8：三问归一规则先定义 |
| 一次请求禁止串跑多种决策器；实名模型 bypass | C1 | G7：重判只发生跨请求边界 |
| `entrypoint.router == recipe.router` + recipe 字段互斥 | C2、C4、C8 | `config/src/lib.rs:825-939` 校验保持；新增键走 `deny_unknown_fields` |

---

## 9. 与既有文档的关系

| 文档 | 关系 | 本次是否改动 |
|------|------|:------------:|
| `requirements.md` §1–§6 | 功能边界与硬不变量，本计划的保护对象 | **否** |
| `requirements.md` §7 | C1–C9 候选池，待人审、未排期；编号与声明保持原样 | **否** |
| `requirements.md` §8 | U1 的并列独立需求章节（新增） | 是（末尾追加） |
| `task.md` T0–T89 / 候选池 | 已完成清单与候选池，勾选状态与文字保持原样 | **否** |
| `task.md` 阶段 T | T90–T99 计划稿表格（新增） | 是（末尾追加） |
| `hiroute-vs-aria-router.md` | 特性矩阵与差距依据，仅被引用 | **否** |
| `AGENTS.md` / `README.md` / `README_cn.md` / `docs/coverage-exclusions.md` | 只读引用，`AGENTS.md` 保持 ≤100 行 | **否** |
| 全部 Rust / TS / Python 源码、`Cargo.toml`、`dashboard/package.json` | 本计划只描述将来的改动目标，本次不触碰 | **否** |

---

## 10. 编号回写方案与四处对照

| 处 | 内容 | 状态 |
|----|------|:----:|
| `requirements.md` §7.2 | `C1`–`C9` 候选条目（原文） | 待人审、未排期 |
| `requirements.md` §8 | `U1` 独立需求 | 计划稿 · 待人审激活 |
| `docs/implementation-plan.md`（本文）§5 | 10 张任务卡 `T90`–`T99` ↔ `C1`–`C9` / `U1` | 计划稿 · 待人审激活 |
| `task.md` 阶段 T | 表格列 `T90`–`T99`、里程碑、依赖、验收摘要与四处回指 | 计划稿 · 待人审激活 |
| `task.md` 候选池 | 仅作为回指目标，**不新增勾选、不改一行** | 待人审、未排期 |

**编号映射**：`T90↔C1`、`T91↔C2`、`T92↔C3`、`T93↔C4`、`T94↔C5`、`T95↔C6`、`T96↔C7`、`T97↔C8`、`T98↔C9`、`T99↔U1`。

**写入方式**：两个既有文件均在**文件末尾纯追加**、零删除；`task.md` 相对 HEAD 已有 47 行历史 CRLF→LF 规范化差异（既有 staged 状态），本次不得用 `git show HEAD:task.md` 恢复，也不得引入新的行尾规范化。
