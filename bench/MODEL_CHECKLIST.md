# Checklist：定制 Track B 的 small / mid / large

换上游（TokenHub / Aria Gateway / 自建 OpenAI 兼容）或换三档模型时，按本清单逐项改。保持两套名字清晰：

| 角色 | 含义 | 当前示例 |
|------|------|----------|
| **逻辑名** | router 选档 / YAML `providers.models[].name` / 响应头 | `ariacompute/ariamodel-{small,mid,large}` |
| **上游名** | 真正打到 API 的 `model`（`provider_model_id` / bench `--model-id`） | `qwen3.5-flash` / `glm-5.3` / `deepseek-v4-pro` |

`--quality label` 的 corpus `expected_model` 必须与 **bench pool 解析后的上游名**一致（经 `--pick-map` 之后）。

---

## 0. 先定三元组

为每一档记下：

```text
alias:     small | mid | large
logical:   <逻辑名>          # 例 ariacompute/ariamodel-small
upstream:  <上游 model id>   # 例 qwen3.5-flash
base_url:  <OpenAI 根>       # 无尾斜杠；见 §1 注意 /v1
api_key:   <环境变量名>      # 例 GATEWAY_API_KEY（勿写进 Git）
usd_mtok:  <USD / 1M tok>    # 成本表
```

---

## 1. Router 配置（上游鉴权 + 逻辑名）

### aria — `config/examples/semantic-gateway.yaml`（及如需的 `agent-gateway.yaml`）

- [ ] 每个档位 `name:` = **逻辑名**
- [ ] `provider_model_id:` = **上游名**
- [ ] `backend_refs[].base_url:` = 上游根 URL  
  - aria provider 会拼 `/v1/chat/completions` → 通常 **不要** 在 base_url 末尾再写 `/v1`（当前 TokenHub：`https://tokenhub.tencentmaas.com`）
- [ ] `api_key_env:` = **环境变量名**（如 `GATEWAY_API_KEY`）  
  - **禁止** `api_key_env: ${GATEWAY_API_KEY:-}`（会展开成密钥明文再当变量名查，导致 401）
- [ ] recipe / decisions 里 `model:` 仍用 **逻辑名**
- [ ] `providers.defaults.default_model` 用逻辑名
- [ ] **Agent Track B（§6.8）**：`agent-gateway.yaml` 与 semantic 同 TokenHub/`provider_model_id`/`pricing`；`agent.model` 用逻辑 mid；**XOR** serve（semantic XOR agent，同 `:8899`）；bench 用 `--entrypoint aria_router=ariacompute/agent-auto`，报告 `out/agent_vs_vsr_{routing,compare}.*`

### vllm-sr — `bench/vllm-sr/config-gateway.yaml`

- [ ] 同步三档 `name` / `provider_model_id` / `base_url` / auth  
  - Envoy 路径：`base_url` 通常要带 **`/v1`**（例 `https://tokenhub.tencentmaas.com/v1`）
- [ ] `api_key_env: GATEWAY_API_KEY`（变量名，非 `${…}`）
- [ ] decisions 里的 model 引用仍用逻辑名  
- [ ] 若策略是「冻结 keyword 基线」，只改 provider 面，不要把 aria advanced 规则同步过来

重启：

```bash
export GATEWAY_API_KEY=…   # 勿写入 router.log / 仓库
# aria
./target/release/aria-router serve --config config/examples/semantic-gateway.yaml \
  --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090
# vllm
vllm-sr serve --config bench/vllm-sr/config-gateway.yaml
```

冒烟：

```bash
# 直连上游
curl -sS -o /dev/null -w "%{http_code}\n" -X POST "$BASE/v1/chat/completions" \
  -H "Authorization: Bearer $GATEWAY_API_KEY" -H "Content-Type: application/json" \
  -d "{\"model\":\"$UPSTREAM_SMALL\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"max_tokens\":8}"

# 经 aria（逻辑名 bypass）
curl -sS -D - -o /dev/null -X POST http://127.0.0.1:8899/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d "{\"model\":\"$LOGICAL_SMALL\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],\"max_tokens\":8}" \
  | tr -d '\r' | grep -i x-aria-router
```

---

## 2. Bench corpus（label 金标）

- [ ] `bench/corpus/routing_gateway.json` — `expected_model` = **上游名**
- [ ] `bench/corpus/routing_gateway_hard.json` — 同上（small/mid/large 分布 + 陷阱题）
- [ ] `mmlu_tiny.jsonl`（compare）— **无** `expected_model`，一般不用因换模型而改

---

## 3. Prices

- [ ] `bench/prices/ariamodel.json`：为每个 **上游名** 填 USD/MTok；可保留逻辑名别名以免漏 map
- [ ] 可选：同步 `bench/prices.py` 的 `DEFAULT_USD_PER_MTOK`

---

## 4. Bench CLI 命令（每次跑）

`routing` 与 `compare` 共用：

```bash
export GATEWAY_API_KEY=…

python3 -m bench routing \   # 或 compare
  --router aria_router=http://127.0.0.1:8899 \
  --router vllm_sr=http://127.0.0.1:8890 \
  --entrypoint aria_router=ariacompute/semantic-auto \
  --entrypoint vllm_sr=auto \
  --pick-header aria_router=x-aria-router-model \
  --pick-header vllm_sr=x-vsr-selected-model \
  --pick-map <逻辑名-small>=<上游名-small> \
  --pick-map <逻辑名-mid>=<上游名-mid> \
  --pick-map <逻辑名-large>=<上游名-large> \
  --pool small=<base_url> \
  --pool mid=<base_url> \
  --pool large=<base_url> \
  --model-id small=<上游名-small> \
  --model-id mid=<上游名-mid> \
  --model-id large=<上游名-large> \
  --api-key small=$GATEWAY_API_KEY \
  --api-key mid=$GATEWAY_API_KEY \
  --api-key large=$GATEWAY_API_KEY \
  --prices bench/prices/ariamodel.json \
  --timeout 300 \
  …
```

核对：

- [ ] `--pool` 的 base_url：**无**尾 `/v1`（bench 自己拼 `/v1/chat/completions`）
- [ ] `--model-id` = 上游名（与 corpus `expected_model` 一致）
- [ ] `--pick-map` 逻辑名 → 上游名（routing **必须**；compare 用于 `routed_model` / cost）
- [ ] Agent Track B：`--entrypoint aria_router=ariacompute/agent-auto`；serve `agent-gateway.yaml`；报告 `./out/agent_vs_vsr_{routing,compare}.json`
- [ ] `--api-key alias=$GATEWAY_API_KEY` 用双引号或裸变量，**不要**单引号包住 `$VAR`
- [ ] routing：`--corpus …/routing_gateway_hard.json --quality label`
- [ ] compare：`--corpus …/mmlu_tiny.jsonl`（无需 label）
- [ ] **公平 latency compare**：跑前重启 aria-router，清空进程内 `response-cache`（`PUT /config` 不清理 cache；未重启时 p50 可到数 ms）
- [ ] aria 热路径：进程内共享 `reqwest::Client`（TLS/连接复用）；`algorithm: static` 不建全量 ranking maps（§6.7）

```bash
# terminal1 — clear cache = restart (keep GATEWAY_API_KEY in env)
# Ctrl-C the running serve, then:
./target/release/aria-router serve \
  --config config/examples/semantic-gateway.yaml \
  --bind 127.0.0.1:8899 --mgmt-bind 127.0.0.1:8090

# terminal4 — only after :8899 is up again
python3 -m bench compare …   # same flags as §4
```

bench 自检：

```bash
python3 - <<'PY'
import os
from bench.http_client import EndpointConfig, chat_completion, probe_models
cfg = EndpointConfig("<base_url>", api_key=os.environ["GATEWAY_API_KEY"], timeout_s=60)
print("probe", probe_models(cfg))
print(chat_completion(cfg, model="<上游名-small>", prompt="hi", max_tokens=8).status)
PY
```

---

## 5. 文档（可选但建议）

- [ ] 根 `README.md` / `README_cn.md` Track B 示例（pool / model-id / pick-map / prices）
- [ ] `bench/corpus/README.md` 语料说明
- [ ] `bench/vllm-sr/README.md` base_url / baseline 说明

---

## 6. 验收（改完必看）

### routing

- [ ] `cells_ok == questions * 3`，`cells_error == 0`
- [ ] `live_router_errors == 0`（否则 ladder 会被 timeout / pick 失败偏置）
- [ ] label 模式：pool completion SSL/EOF 不应把正确选档打成 `status=error`（`completion_error` 可记在 cell detail）
- [ ] ladder：`aria_router` / `vllm_sr` 的 `mean_quality` / `mean_cost` 可比
- [ ] 抽查 hard 陷阱题：aria → small 上游名；vllm baseline 仍可能误 mid

### compare

- [ ] `results` 里 router 行：`routed_model_raw` = 逻辑名，`routed_model` = 上游名
- [ ] `cost_usd` 按上游名价格计算
- [ ] accuracy / p50 latency 可用于 §6.6

---

## 常见翻车

| 症状 | 原因 |
|------|------|
| TokenHub / 上游 `401002` | `api_key_env` 写成了 `${VAR:-}`；或 bench 未带 `--api-key`；或 key 未 export |
| `pick … not in pool` | 缺 `--pick-map`，或 map 目标 ≠ `--model-id` |
| label 全 0 / always_* 质量怪 | corpus `expected_model` 仍是旧逻辑名或旧上游名 |
| vllm 405 | `base_url` 缺 `/v1`（Envoy） |
| aria 通、bench pool 401 | pool URL/key 与 router 不一致；用 §4 自检对齐 |
| ladder 里对方更「便宜」但有 live error | error 题被排除出均值 → 先清零 `live_router_errors` |

---

## 当前默认（TokenHub，2026-09）

| alias | 逻辑名 | 上游名 |
|-------|--------|--------|
| small | `ariacompute/ariamodel-small` | `qwen3.5-flash` |
| mid | `ariacompute/ariamodel-mid` | `glm-5.3` |
| large | `ariacompute/ariamodel-large` | `deepseek-v4-pro` |

- aria `base_url`: `https://tokenhub.tencentmaas.com`
- vllm `base_url`: `https://tokenhub.tencentmaas.com/v1`
- bench `--pool`: `https://tokenhub.tencentmaas.com`
- env: `GATEWAY_API_KEY`
