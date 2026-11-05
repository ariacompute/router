# aria-router 发版说明

本说明覆盖本次发布的 `T90–T99` 特性集（C1–C9 / U1）。运行时契约（硬约束剪枝
+ fail closed + semantic / agent / afm-d 三决策器隔离）保持不变；以下新增均为
可开启能力，且绝不绕过该契约。

## 新增

- **重判边界（C1）** — `core::RedecisionTracker` 判定跨请求边界（`new-user-message`
  / `context-compaction` / `tool-continuation` / `same-turn` / `unknown`）。历史
  不连续（`unknown`）直接 fail closed，不复用冻结决策。所选边界通过新增响应头
  `x-aria-router-redecision` 在每个响应上可观测。由 `global.services.redecision`
  配置（默认关闭）。
- **afm-d 三问模板（C2）** — `afm-d.question_template: single|three`。`three` 时在
  `route` 之外再问 阶段能力 / 难度 / 延续性，并归一为一个 typed `RouteDecision`；
  `hard` 难度在硬过滤后的 eligible 池内升档，答案同时写入执行证据。
- **低分保护（C3）** — `routing.competence.protection` 在「新鲜 / 完整 / 同类目 /
  同 plan+rubric 版本」证据显示低分时，对**已通过硬剪枝**的 eligible 池重排。
  状态机：触发 → 解除 → 释放；旧低分不重复触发；未评分 ≠ 0；保护永远不能覆盖
  location / auth / modality 剪枝。
- **信任档位（C4）** — `global.services.trust` 以最小证据量、置信度、升/降档阈值
  为闸门（含 plan / rubric 版本）。新增接口：`GET /v1/router/trust`（列出草稿）、
  `POST /v1/router/trust/suggest`（下一步建议档位变化；证据不足 / 低置信度 → 无
  建议）、`POST /v1/router/trust/publish`（**仅在** `confirm: true` 时应用草稿，
  纯离线重算，**不**调用 `PUT /v1/router/config`）。`?estimate=true` 返回启发式
  离线估算。由新增 **Trust Tiers** Dashboard 页面驱动。
- **模型表现（C5）** — `GET /v1/router/cost/model-performance` 返回按模型 / 档位
  的成本占比、请求数与阶段能力评分。无证据时报告 `has_evidence: false`（Dashboard
  显示「No data」而非 0）。在新增 **Model Performance** Dashboard 页面展示。
- **生产路径执行证据（C6）** — 生产路由路径现写入 任务类目 / 阶段能力 /
  接受输出 / 工具结果 / plan+rubric 版本 / 延续性 / entrypoint / 重判边界 这一
  证据链。JSONL 向后兼容；关闭 replay 时零额外证据计算。
- **API key 软额度（C7）** — `global.services.quota`（默认关闭）按 key 与共享
  `group` 持久累计花费；每次请求先预留、再按实际流量结算（含 SSE 中断）。除非 key
  显式声明 `spend_limit_usd`，否则不施加任何上限——记账本身永不阻塞。额度耗尽
  返回 `QuotaExceeded`（429），与限流区分。`GET /v1/router/quota` 暴露待结算预留
  与结算错误（结算失败只记录、绝不吞掉）。
- **Backend 故障转移（C8）** — `global.services.backend_failover`（默认关闭）按权重
  从模型 `backend_refs` 生成候选链，失败时切到下一候选（SSE 与非 SSE 两条路径）；
  全部失败时聚合报错且不泄露密钥。
- **离线策略一致性 bench（C9）** — `python -m bench policy-consistency` 离线重算
  4 种确定性策略（cheapest / strongest / expected-label / by-config-expected），
  输入为已保存的决策记录——无需 live router；同一摘要也会内嵌进 live 报告。
  report-only，恒含 `ci_fail: false`。
- **Dashboard 重做与审计（U1）** — 新增 **Model Performance** 与 **Trust Tiers**
  页面。Dashboard 现为单色 / 扁平 / 直角 / AA / ≥12px，动效仅 transform/opacity，
  使用本地字体栈（无 web-font CDN）；设计契约见 [`PRODUCT.md`](PRODUCT.md)。
  `scripts/audit-ui.sh` 以五种模式运行 impeccable 风格审计（`--static` / `--dom` /
  `--url` / `--browser` / `--list-rules`）；无法运行的模式记为 `skipped`，绝不
  伪造成功。详见 [README_cn.md](README_cn.md) 的「本地渲染检查（重跑）」一节。

## 变更

- `GET /v1/router/cost` 原本即报告六因子花费，现与新增的模型表现接口及 Cost by
  Competence 页面配套。
- 响应头新增 `x-aria-router-redecision`。
- CI 新增必过的 `dashboard`（build + UI 护栏 + `--static` + `--dom`）与 `bench`
  job；并强制 100% 行覆盖门禁（`scripts/coverage.sh`）。

## 备注

本地渲染检查 / 重跑：

```bash
npm --prefix dashboard ci
npm --prefix dashboard run build
npm --prefix dashboard run test
./scripts/audit-ui.sh --list-rules
./scripts/audit-ui.sh --static
./scripts/audit-ui.sh --dom
# 在有 Chrome 与 impeccable 二进制的机器上：
IMPECCABLE_BROWSER=/usr/bin/google-chrome ./scripts/audit-ui.sh --url
```
