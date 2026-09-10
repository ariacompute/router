# AGENTS.md — aria router

工程上下文入口。逐层展开：先看「概述/架构/目录」，动手时再看「规范/命令/进行中/注意」。

## 概述
`router` 仓库 = aria 推理网关（Rust）：独立 OpenAI 兼容 HTTP，**不走 Envoy**。
两种并列决策器：**semantic**（对齐 vLLM Semantic Router YAML v0.3）与 **agent**
（进程内轻量 builtin：固定工具 + 限 turns）。共享 providers、硬约束、转发。
产物：`aria-router` CLI + C ABI + 八语言 SDK（与 engine SDK **两套包**）。

## 架构
OpenAI 面 → Entrypoint（`semantic` | `agent`）→ 硬约束剪枝 → 决策 → plugin → provider。
Semantic：signals → projections → Boolean decision → algorithm/looper。
Agent：`BuiltinAgent` tool-loop → `submit_route` → typed `RouteDecision`。
Location / auth / modality 硬剪枝在决策前；fail closed；实名模型 bypass recipe。

## 目录
- `config/` `signal/` `decision/` `algorithm/` `plugin/` `provider/` `agent/` `http/`：运行时 crate
- `bin/`：`aria-router`（setup / validate / serve / upgrade）
- `ffi/`：`ariacompute-router-ffi`（`libaria-router_ffi`）
- `bindings/`：rust / python / go / typescript / react-native / flutter / swift / kotlin
- `dashboard/`：管理面 SPA（Login/Account/Keys/Users/Cost + Overview…）
- `bench/`：Python report-only（ADR-040 / DRACO / vs vLLM SR `compare`）
- 根：`AGENTS.md` / `requirements.md` / `task.md` / `README.md` / `Cargo.toml`

## 开发规范
- Rust edition 2021+；错误统一 `RouterError`；禁止吞错与静默 no-op。
- 未实现 YAML 能力 → `Unsupported`（validate 与运行时均失败）。
- 新增功能须单测（正常 + 异常）；合入前 `cargo test` 全绿。
- Harness：半天以上须 `requirements.md`（人审）→ `task.md` → 编码。
- AGENTS.md ≤100 行；信号/算法清单下沉 `requirements.md`。
- C ABI 变更须更新 `bindings/testdata/cases.json` 与宿主测。

## 常用命令
- `cargo test`
- `cargo test -p aria-router-signal --features ml`
- `cargo run -p aria-router -- setup`
- `cargo run -p aria-router -- validate`
- `cargo run -p aria-router -- upgrade`
- `cargo run -p aria-router -- serve --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090`
- `npm --prefix dashboard ci && npm --prefix dashboard run build`
- `./scripts/run-binding-tests.sh`
- `python -m unittest discover -s bench/tests -t .`
- `python -m bench routing --router aria_router=… --router vllm_sr=… …`
- `python -m bench compare --corpus bench/corpus/mmlu_tiny.jsonl …`

## 进行中需求
阶段 **R**（T50–T56）已落地。**T81** `aria-router upgrade` 已落地。见 `requirements.md` / `task.md`。

## 注意事项
- 黄金路径：keyword → static；retention sticky；agent → `submit_route`。
- 四维：Models / Compute / Location / Preference。不做 Envoy / HaluGate / Prometheus。
- 一次请求禁止串跑 semantic+agent。`cargo test`；learned：`--features ml`。
- Bench report-only；对标 aria `:8899`、vLLM SR `:8890`。
