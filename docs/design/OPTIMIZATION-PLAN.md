# PatternStudio 工程优化方案（2026-11）

> 状态：提案。源于对 mesh-avatar-studio 的代码级对比评审，全部改进点均以本仓库现有代码证据为前提，不做无证据的照搬。

## 1. 目标与非目标

### 目标

1. 错误链路结构化：用户在任何入口（Web、Tauri、CLI）遇到的失败都能得到**可行动的分类提示**，而不是裸错误字符串。
2. 落盘安全：服务器写项目文件时具备原子性与崩溃安全，与 Tauri 桌面端并发写同一目录时不产生半截文件。
3. 交互回归可验证：核心用户流程（上传→smart→trace→图层编辑→保存）进入 CI，破坏即红灯。
4. 质量可度量：样本图的 trace 指标进入快照回归，参数调优不再靠目测。

### 非目标（明确排除，附理由）

| 排除项 | 理由 |
|---|---|
| 优先级调度器改造 | 现有 `web/src/traceQueue.js`（两级优先级 + 300ms 合并 + 抢占）与 engine 的 cancel/busy 机制已优于对比项目，不动 |
| IndexedDB 结果缓存改造 | `web/src/traceCache.js` 已实现按参数键持久缓存，保留 |
| 引入 changesets / npm workspaces | 6 个 crate 同拍发布，`cargo workspaces release` 足够 |
| stale（带外修改）签名检测 | 当前唯一的项目落盘写点是 `save-image`，不存在多写者场景；**触发条件**见 §6 |
| 跨绑定 conformance fixtures | 绑定尚未发布；**触发条件**见 §6 |

## 2. 现状证据（改造前事实基线）

- **错误契约**：engine 以 `Result<_, String>` 传递错误，`"cancelled"` / `"busy"` 是魔法字符串（`crates/patternstudio-engine/src/lib.rs:376,398,693`）；server 返回 `Json({"error": <裸字符串>})`（`main.rs:255,300,315`）；desktop 命令同样返回 `Result<_, String>`（`desktop/src/commands.rs`）。前端 `handle()` 直接 `throw new Error(msg)`（`web/src/api.js:16-21`），仅对 cancelled/busy 特判为 `TraceFlowError`。用户看到的是引擎原始报错文本。
- **落盘**：`LineageFile::save` 已是 tmp+rename 原子写（`main.rs:112-116`），但 `save_image` 用 `std::fs::copy` 直接覆盖目标（`main.rs:983`，注释自述 "Overwrites an existing file"）。路径穿越已有黑名单防护（`:213,878,950`），此项**不改**。
- **测试**：CI 仅 `cargo test/clippy/fmt`（`.github/workflows/ci.yml`），前端 vitest 不在 CI 中，无任何 e2e。engine 侧 `Result<_, String>` 无错误类型，错误分支不可测试。
- **CI 已钉住 vtracer fork commit `13435cf`** —— 这是质量快照可以做确定性基线的前提。

## 3. 设计总原则

1. **只增不改**（吸取对比项目的兼容性纪律）：HTTP 响应保留 `error: string` 字段，新增可选 `code`；旧前端对新 server 照常工作，新前端对旧 server 优雅降级（`code` 缺失→回退裸文本）。engine `EngineError` 的 `Display` 必须逐字保留 `"cancelled"` / `"busy"`，保证 Tauri 路径与既有测试不破。
2. **单一错误真相源**：分类定义在 `patternstudio-engine`，server/desktop/cli 只做映射，不各自发明字符串。
3. **每包可独立发布**：WP1→WP2→WP3→WP4 依赖链严格线性，任一包合入即有收益，无中间破坏态。

## 4. 工作包

### WP1 — 错误码契约（P0）

**问题**：Real-ESRGAN 缺环境、图片解码失败、参数非法等失败对用户不可区分；agent/API 用户无法程序化分支。

**设计**：

1. `patternstudio-engine` 新增：

   ```rust
   pub enum EngineError {
       Cancelled,          // Display = "cancelled"
       Busy,               // Display = "busy"
       InvalidInput(String), // 非图片/尺寸非法/参数组合非法
       Dependency(String),   // Real-ESRGAN 模型缺失、uv/python 缺失等环境问题
       Io(String),
       Internal(String),
   }
   ```

   用 `thiserror`；`lib.rs` 公开函数签名改为 `Result<_, EngineError>`。preprocess 的模型检查（`patternstudio-tracer`）从裸字符串升为 `Dependency`。

2. server：错误响应改为 `{"error": msg, "code": "cancelled"|"busy"|"invalid_input"|"dependency"|"io"|"internal"}`。`error` 语义不变（人读文本），新增字段向后兼容。

3. desktop `commands.rs`：错误经 `EngineError` 序列化返回；Tauri invoke 的 reject 载荷带 `code`。

4. web：
   - `api.js handle()`：解析 `body.code`，挂到 `err.code`；`TraceFlowError` 判定改以 `code` 优先、字符串匹配回退。
   - 新增 `web/src/errorText.js`：`code + i18n` → 文案键（如 `dependency` → "预处理依赖未就绪，运行 `make preprocess-setup`"）；6 个 locale 各加 ~6 个键。
   - 展示层照搬对比项目 JobFeedback 模式：一句话可行动文案 + `<details>` 折叠原始日志。

**验收**：
- 每条错误路径有单测（engine 侧 Rust 测试断言 `code`；前端 `api.test.js` 断言 `err.code`）。
- 拔掉 Real-ESRGAN 模型后 UI 显示"依赖缺失+修复命令"，不再是 traceback 文本。
- 新旧 server/前端四象限手工冒烟各一次，行为符合 §3.1。

**工作量**：2–3 天。**风险**：签名改动波及 `trace_file_once` 等公开函数 → 通过 `Display` 保字面量 + 全 workspace 编译回归兜底；回滚 = revert（响应字段是纯增量）。

### WP2 — save-image 原子落盘（P0）

**问题**：`save_image` 直接 `fs::copy` 覆盖；写入中途崩溃/断电留下半截 PNG，且与 `GET projects/{id}/images/{file}` 并发时可能读到截断内容。

**设计**：复用 `LineageFile::save` 已有模式——写 `.{filename}.{uuid}.tmp` → `rename` 到目标；tmp 与目标同目录保证同卷 rename 原子。失败路径 unlink tmp。不改文件名黑名单校验。

**验收**：新增 server 集成测试：save-image 与 images 读取并发 100 轮，读端 sha256 恒等；`kill -9` 后目录内无残留 tmp（启动时清理 `.*.tmp` 一次）。

**工作量**：0.5–1 天。**风险**：极低；回滚 = revert。

### WP3 — 前端测试进 CI + 最小 e2e（P1）

**问题**：`traceQueue`/`traceCache` 等已有 vitest 单测但 CI 不跑前端；用户流程零回归保护。

**设计**：

1. CI 增加 `web` job：`npm ci && npm run build && npx vitest run`。
2. `web/` 引入 `@playwright/test`（devDependency），`playwright.config.ts` 用 `webServer` 起 `patternstudio-server`（`HOST=127.0.0.1 PORT=8123`，指向 `tests/fixtures/e2e-project`），端口与 8000 隔离避免与本机 dev 冲突。
3. 首批仅 3 条 spec（对齐对比项目按"流程"而非"组件"组织的做法）：
   - `workflow.spec.ts`：上传 fixture → smart 建议出现 → trace 完成 → 图层面板隐藏/恢复图层 → save-image 成功；
   - `errors.spec.ts`：上传非图片 → 显示 WP1 的分类文案；并发第二请求触发 busy → UI 静默不报错；
   - `empty.spec.ts`：空态文案与上传入口可达性（键盘 Tab 顺序）。
4. Makefile 增加 `test-web` / `test-e2e`；e2e 仅进 PR job，不进每次 push（时长控制）。

**验收**：故意把 `traceQueue` 的 debounce 常量改坏或删掉 SmartBanner 渲染，CI 必须红。

**工作量**：3–4 天。**风险**：CI 里 server 需要 release 构建的 Vite dist——复用 `make web`；Tauri 分支在 e2e 中不可测，`isTauri()` 恒 false，天然隔离。

### WP4 — 质量指标快照回归（P1）

**问题**：smart preset、分层策略、VTracer fork 参数调优没有量化门；v0.3 的 5 种策略与 L0/L1/L2 profile 改坏了无人知晓。

**设计**：

1. CLI `trace --json` 输出（已存在）定为快照 schema：`shapeCount, layerCount, colorCount, svgBytes`；新增 `--baseline <path> --tolerance 0.05` 三态输出（pass/fail + 逐项 diff）。
2. `tests/fixtures/quality/`：4 张固定样本（line art / bw / color / gradient，正好覆盖 smart preset 的四个族）+ `baseline.json`。
3. CI 增加 `quality` job（仅 ubuntu——CI 的 vtracer 已钉 `13435cf`，同平台浮点一致才做基线；跨平台差异列为已知限制，不进门）。
4. 基线更新走 PR diff 评审，改基线必须附指标变化说明。

**验收**：把分层策略的某处容差改大 → shapeCount 漂移超 5% → CI 红。

**工作量**：1–2 天。**风险**：换机重编导致基线漂移 → 用 CI 同 runner 镜像；容差而非精确匹配。

### 快速得点（随 WP1 顺路做）

- 仓库根加 `AGENTS.md`：把「接收图片 → `/smart` → `/trace` → `--json` 校验」的配方写成 agent 可执行步骤，为 v1.0 的 API/绑定用户（同样是 agent）服务。

## 5. 实施顺序与依赖

```
WP1(错误契约) ──┬──> WP3(errors.spec 依赖 WP1 的 code)
                └──> WP4(失败时报错可读,非硬依赖)
WP2(原子落盘)  ──独立──> 可与 WP1 并行
WP4 基线录制须在 WP3 的 quality job 骨架就绪后
```

合并顺序：WP2 → WP1 → WP3 → WP4。每个 WP 独立 PR、独立可回滚。

## 6. 明确延后项与触发条件

| 项 | 触发条件 | 届时方案 |
|---|---|---|
| stale 签名检测 | 出现第二个项目文件写者（如 agent 通过 API 保存 trace 参数、或新增 project-save 端点）时 | 项目参数文件加 `version` + 语义签名（参考对比项目 `layerSignature`），保存端点带 If-Match 式校验 |
| conformance fixtures | `patternstudio-py` / `-wasm` 发布首个 alpha | `tests/conformance/` 共享 JSON fixtures，三绑定同一套件断言 `LayeredDoc` 序列化一致 |
| WebSocket 通道 | 需要双向实时（如桌面预览窗实时重放 trace） | 现在 SSE 单向足够，不预铺 |

## 7. 总体度量

- 错误分类覆盖率：server 全部 `Err(...)` 路径带 `code`（grep 审计 = 0 处裸字符串响应）。
- CI：前端测试从 0 → 进 CI；e2e 3 条主链路；质量快照 4 样本。
- 兼容性：HTTP 契约变更为零破坏（新增可选字段）；engine 公开 API 仅错误类型收敛。
