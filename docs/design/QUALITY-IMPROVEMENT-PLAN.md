# PatternStudio 纹样提取质量改进方案（2026-11）

> 状态：提案。依据 `docs/design/QUALITY-EVAL-2026-11.md` 的实测数字（下称"评估报告"，其发现编号 F1–F7）。与 OPTIMIZATION-PLAN.md（工程侧 WP1–WP4）正交，本方案编号 QWP1–QWP6，专注提取质量。

## 1. 目标与非目标

### 目标（量化验收线，基线=评估报告实测）

1. 细线干净输入 ink F1：现状 0.37–0.86 → **≥0.85**（8 种 512px 纹样，除冰裂纹外全部；冰裂纹 ≥0.6）。
2. JPEG q60 退化输入 ink F1：现状 0.00–0.08 → **≥0.60**，PSNR ≥22。
3. 打分可信：smart 选中的候选，其内部 fidelity 与真实往返 ink F1 不再背离（同一图像上两指标排序一致；fid 高但 F1<0.5 的选择 = bug）。
4. smart 延迟：512px 图 release 构建 **≤2s**（现状 debug 最高 10.3s，release 待测）。
5. 全程不回退 greek_key/meander（F1≈1.0 必须保持）。

### 非目标（明确排除，附理由与触发条件见 §6）

| 排除项 | 理由 |
|---|---|
| 端到端深度学习矢量化（SVGDreamer++/StarVector/DeepSVG/Im2Vec） | 综述结论：慢、需人机协同、依赖训练数据；与"本地、即时、可控"定位冲突。仅作 spike |
| SAM/基础模型语义分层 | 引入重依赖；fork 的 cutout/watershed 已提供低配替代，先榨干 |
| 换追踪引擎（potrace 等） | VTracer 1.0 已钉 fork、结构指标与 greek_key 近无损证明引擎上限足够；问题在候选与度量 |
| 真实文物照片数据集建设 | 需要外部资源，列为触发项 |

## 2. 现状证据（引用评估报告）

- F1：RMSE fidelity 在白底/稀疏纹样饱和（fid 恒 0.94–1.00，掩盖 1-shape 塌缩与整线丢失）。
- F2：族路由由 `bimodality ≥ 0.45 && colorfulness < 20` 门控（`smart.rs::candidate_params`），JPEG 彩噪使二值化候选整体缺席 → scan 组 F1 0.0；而输入退化本身只丢 11–28% 墨量。
- F3：1.5px 细线 F1 0.37–0.86；弯曲细线在样条拟合处收缩。
- F4：fork 1.0.0-alpha.4 已支持 watershed/cutout/palette/max-colors/simplify，`TraceParams` 已透传，smart 候选从未尝试（`family_candidates` 只动 speckle/阈值/precision/layer_diff/simplify 微调）。
- F5：complexity 项在 512px 上 ≈0，跨族 λ 形同虚设。
- F7：CLI 参数子集 ≠ 完整参数，验收必须走 `/api/v1/trace` 完整参数路径。

## 3. 设计总原则

1. **度量先行**：先把笔画召回做成可信指标（QWP1），此后每个 QWP 用同一把尺 A/B；没有度量数字不得合入候选改动。
2. **同图同尺**：跨族比较必须用同一保真度量。对"笔画主导"图像（墨迹覆盖率 <15% 且边缘密度高）统一启用带容差的墨迹召回指标，替换 RMSE——直接消除 F1/F5 两个可比性缺陷。
3. **不碰引擎内部**：所有改动在 `patternstudio-core`（smart 候选/打分）、`patternstudio-tracer`（预处理）与 server 编排层；vendored vtracer 仅在 QWP5 需要新二值化算法时增量扩展。
4. **每包独立可回滚**，候选扩展用常数开关（候选预算）控制延迟回归。

## 4. 工作包

### QWP1 — 诚实度量：笔画召回进打分与门禁（P0）

**问题**：fid 说谎（F1），门禁只有结构计数（WP4）测不出"画得不像"。

**设计**：

1. `smart.rs` 新增 `stroke_fidelity(seg, img) -> f64`：源图与合成分按亮度阈二值化 → 各膨胀 r=2px → ink F1。启用条件 `stroke_dominant(stats)`：墨迹覆盖率 <15% 且 `texture ≥ 0.05`（团花/回纹类全部命中；photo/flat 色块图不命中，维持 RMSE）。`Criteria` 增加 `stroke_aware`，`trial_candidates` 打分按此三分支（RMSE / ink-F1 / stroke-F1）。
2. `ImageStats` 补 `ink_coverage: f64`（<128 像素占比）——一次遍历顺带算，无额外成本。
3. 质量门禁扩展：`baseline.json` v2 增加 4 个 512px 样本（rosette/meander/ice_crack/cloud，`pattern` 生成 + rsvg 光栅化，脚本入库 `tests/fixtures/quality/record.sh`），新指标 `ink_f1`（对源位图，膨胀 r=2）与 `psnr`；`--tolerance` 对 F1 用绝对差 0.05、PSNR 用 1dB。CI quality job 镜像装 `librsvg2-bin`（rsvg-convert）；PGM 互转用 `magick -compress none` 或纯 Rust image crate，避免 CI 依赖 ImageMagick。
4. 录制初版基线时附上评估报告 §2.3/2.4 数字作 PR 说明。

**验收**：构造性测试——人为丢弃一条笔画的合成分，stroke_fidelity 显著下降而 RMSE 几乎不变（防退化回归的元测试）；8 样本 + scan 变体的 F1/PSNR 进基线，当前值（0.37/0.00 等）成为红灯起点。

**工作量**：2–3 天。**风险**：膨胀半径/覆盖率阈值需小样本调参——阈值常量集中定义，基线录制在其后。

### QWP2 — 退化线稿路由与自适应二值化（P0）

**问题**：F2 路由缺陷是 scan 组崩塌根因；修复即拿到最大单项收益。

**设计**（全部在 `smart.rs::candidate_params` 与 server 编排）：

1. 路由加 OR 分支：`stroke_like(stats) = texture ≥ 0.05 && effective_colors ≤ 24 && colorfulness < 60`（彩噪 JPEG 云纹命中：colorfulness≈272 不满足…注意用**降采样后饱和度直方图**判彩噪——JPEG 彩噪的空间高频色度在 colorfulness 上虚高，加 `noise` 修正项 `colorfulness − 2·noise`）。命中即把 lineArt + bw 候选加入，无论 bimodality。
2. 二值化参数化：lineArt/bw 候选除现有 otsu±20 外，增加 Bradley–Roth 自适应对（`binary_adaptive=true`，window 0/auto、t ∈ {15, 25}）——候选+4。
3. 自动预处理钩子：server `smart`（及 batch-trace smart 模式）中，当 `stats.noise > 12 || illumination_var > 20` 且模型可用时，先 `preprocess_image`（Real-ESRGAN，已有）→ 对增强图重跑 analyze + 全部试追踪；响应带 `preprocessed: true`，UI SmartBanner 提示。失败静默回退原图。
4. `/api/v1/smart` 响应增加每候选 `strokeFidelity`（当 stroke_dominant），前端可选展示。

**验收**：scan 组 F1 ≥0.6（对照 scan2 组 0.62–0.90 即上限证据）；干净 8 样本选择与 QWP1 基线一致或更好；preprocess 触发率在有噪样本 =100%、干净样本 =0（误触发率审计）。

**工作量**：2–3 天。**风险**：自动预处理使 512px 延迟 +1~2s——仅高噪触发且 release 复测；彩噪误判 photo 族 → QWP1 度量兜底 A/B。

**落地记录（2026-11-21）**：

1. 第 1 条按实测数据落地为 `stroke_like = texture ≥ 0.025 && effective_colors ≤ 24 && colorfulness − 2·noise < 60`。与提案差异：真实根因是 scan 的 bimodality 塌缩为 0（稀疏墨量），而样本是灰度 JPEG、colorfulness 实测恒为 0——彩噪假设与噪修正项在真实数据上不起作用，命中靠 texture 与 colors；texture 阈值由 0.05 降至 0.025（云纹类 scan 的 texture 落在 0.025–0.045 区间；poster/photo 族合成样本验证不误触发）。第 3 条触发条件加 stroke 前提 `(stroke_dominant || stroke_like) && (noise > 12 || illumination_var > 20)`，防止照片/色块图用自身色块的照度方差误触发预处理。
2. **度量 bug**：A/B 脚本把 435×435 的 scan 样本与 512×512 光栅按公共 n×n 前缀截断对齐，每行错位 77px——评估报告 §2.4 中干净 scan 的 ink F1（0.00–0.08）系该错位产物，非管线真实损失。修复（按源图原始尺寸光栅化）后，干净 scan 在既有选型下即得 0.918–0.997。
3. 最终 A/B（验收线 scan 组 F1 ≥0.6，全部通过）：干净 scan ice_crack 0.918 / cloud 0.954 / meander 0.997 / rosette 0.946；scan2 组 ice_crack 0.909 / cloud 0.952 / meander 0.970 / rosette 0.933（自动预处理较基线 0.625–0.903 的提升真实有效）；干净 8 样本 preprocess 触发率 0/8、有噪 4/4 触发；F1 0.617–1.000（含 ±2px 膨胀的往返度量口径，与报告 §2.3 表不直接可比），其中两处选型因新增候选参与试评而改变：ice_crack photo→lowPoly（0.372→0.617）、wave flat→bw（0.562→0.903），其余六处选型不变，无回退。

### QWP3 — fork 1.0 开关进候选集（P1）

**问题**：F4，cutout/palette/watershed/simplify 从未被评估。

**设计**：

1. 候选族 +3：`cutout`（hierarchical=cutout + color cluster，色块纹样无缝拼镶）、`palette4/palette8`（max_colors=4/8 + 现有色族，纹样配色收敛、文件减半）、`watershed`（clustering=watershed，detail 64/128，仅 texture <0.1 的色块图）。
2. 候选预算：按像素数限制试追踪次数（≤0.5MP 全量；更大图先降采样打分、胜者原尺寸复验），保证 QWP4 延迟线；server 已有按 segment_key 的 session 缓存天然复用。
3. cutout 图 ink 一致性最好（无缝隙），预期直接修 wave/ice_crack 的缝隙漏白。

**验收**：8 样本往返指标不劣化；至少两个样本 svg_bytes ↓≥30% 或 F1 ↑；延迟达标。

**工作量**：2 天（含基线重录）。**风险**：候选翻倍→延迟翻倍，由预算与缓存兜住；cutout 与分层 layers>0 的交互——首期候选固定 layers=0。

**落地记录（2026-11-21）**：

1. 第 1 条按设计落地：色块分支加 `cutout`/`palette4`/`palette8`（+ texture<0.1 时 `watershed64`）、混合分支加 `cutout`/`palette4`/`palette8`；fork 原生支持这些参数，未动 fork。第 2 条候选预算未做（留给 QWP4，session 缓存天然复用分割）。cutout 候选固定 layers=0。
2. **A/B 落地附带修复**：逐候选往返扫描证实试评选择与往返 F1 强相关（ice_crack：lowPoly 试评 0.622↔往返 0.617，bw 0.932↔0.849），但原实现只在 `stroke_dominant` 时用 stroke F1，scan 类 `stroke_like` 源上 lowPoly/photo 等非线稿族仍按 color RMSE 打分——wave 上 color 族 RMSE 0.9197 而往返 F1=0.000 的盲区在选型端复现。修正为 `stroke_basis = stroke_dominant || stroke_like` 时**所有非 ink 族**按 stroke_fidelity 打分（ink_aware 的 lineArt 保持 ink 召回）。
3. **确定性 tie-break**：color RMSE 饱和使 taotie 上 watershed64/palette4/lowPoly 分差 ~9e-5，原排序由试跑噪声决定（QWP2 A/B 曾因此选 watershed64，0.949）。`decision_from` 在距最优分 <1e-3 的候选中选 svg_bytes 最小者升至首位（remove+insert，保持其余降序），taotie 稳定回 lowPoly。
4. 最终 A/B（8 干净原图 + 4 干净 scan + 4 scan2，dev 构建）：**ice_crack 原图 lowPoly→bw，F1 0.617→0.849**（第 2 条打分修正生效）；taotie 回 0.962（第 3 条）；其余样本选型与 F1 与 QWP2 基线一致，scan 两组数值集合与 QWP2 完全相同（0.918–0.997 / 0.909–0.970），无回退。新候选 cutout/palette4/palette8/watershed64 **在 16 样本中 0 次胜出**，无一例 svg_bytes 降 ≥30%——**验收标准（≥2 样本改善）未达**。新候选作为更多试评数据落地（UI 可选展示），是否调评分或展示由后续数据决定，本轮不回调阈值凑验收。smart 延迟因候选 11→13 升至 2.9–10.0s（dev 构建，交由 QWP4 压回）。

### QWP4 — smart 性能与工程项（P1）

1. release 构建跑通全部质量脚本，测得真实延迟基线；profile 试追踪热点（预期在多次 `segment`），利用现有 session 缓存跨候选共享分割（同 segmentation 键的参数组合合并候选，如 palette4/palette8 共享一次 segment）。
2. smart 响应加 `elapsedBreakdown`（analyze/trials/preprocess）。
3. CI quality job 用 release 二进制。

**验收**：512px 延迟 ≤2s；候选数 ↑ 而延迟与改前差 ≤30%。

**工作量**：1–2 天。

### QWP5 — 二值化算法扩展：Sauvola 对照（P2）

**问题**：F2 修复后残留的软边/光照不均长尾。

**设计**：vendored fork `clustering::Binary` 增加 `binarization: otsu|bradley|sauvola`（Sauvola 用积分图 Θ(HW) 实现，arXiv:1905.13038；窗口默认 W=15/k=0.2，参数走 TraceParams）。用 QWP1 度量在 scan 组与文档类样本上和 Bradley–Roth 横评，胜者为默认。不引入 SauvolaNet。

**验收**：scan 组 F1 中位提升 ≥0.05 才改默认，否则保留开关。

**工作量**：2–3 天。**风险**：动 fork = 基线全量重录；改动隔离在 binarization 模块，上游合并友好。

### QWP6 — 对称感知纹样追踪 spike（P2，差异化特性）

**问题**：中式纹样大面积对称被重复追踪 N 次：文件大、对称不严格、编辑不同步。

**设计**（先 spike 后立项）：

1. spike（1 周内出数据）：`patternstudio-core::symmetry`——FFT 旋转对称评分（Brubaker et al., CVPR 2010 思路：梯度方向图按质心分角度段做循环自相关，n∈3..12）+ 镜像评分（翻转差分）；在 8 生成样本上报告检出率/误检率与 n 估计误差。
2. 判定 pipeline 可行性后再设计产品路径：检出后按扇区切图 → 每扇区试追踪（候选数 ÷n 抵消）→ 镜像装配成 layered SVG（每对称组一层，编辑 1/n 同步全图）。UI 走"对称纹样"预设。

**验收**：spike 报告：rosette/duo 类检出 ≥90%、meander 平移对称检出、误报 <5%；若成立，立项装配器（另行 WP）。

**工作量**：spike 3–4 天；产品化预估 1–2 周（不含）。

## 5. 实施顺序与依赖

```
QWP1(度量) ══> QWP2(路由/预处理) ══┐
      ╚══════> QWP3(1.0开关)  ════╬══> 基线重录+验收报告
      ╚══════> QWP4(性能)  ══════┘
QWP5(Sauvola) 依赖 QWP1/QWP2 落地后的残留长尾数据
QWP6(spike) 可与 QWP3/QWP4 并行（度量同样用 QWP1）
```

合并顺序：QWP1 → QWP2 → QWP3+QWP4 → 重录基线 → QWP5 → QWP6。每包独立 PR。

## 6. 明确延后项与触发条件

| 项 | 触发条件 | 届时方案 |
|---|---|---|
| 学习型矢量化（SVGDreamer++/StarVector） | QWP1–QWP5 全部落地后，某类真实输入 F1 仍 <0.5 且量大 | 蒸馏为轻量后处理或独立可选服务，不进主链路 |
| SAM 语义分层 | cutout/watershed 分层满意度 <70%（用户反馈） | onnxruntime 可选依赖，区域预切后逐区追踪 |
| 真实文物样本集 | 有授权图源 | 进 quality fixtures 分区（synthetic/real），real 区只做监控不做门禁 |
| 对称装配器产品化 | QWP6 spike 达标 | 新 WP：扇区追踪+镜像装配+UI 对称图层 |

## 7. 总体度量

- ink F1（QWP1 尺）：干净细线中位 0.73→≥0.85；scan 中位 0.06→≥0.6。
- smart 选择与往返指标背离样本数：→0。
- 门禁：结构 4 指标 + F1/PSNR × (5+8+8) 样本；基线更新一律附指标 diff 说明。
- 延迟：512px smart release ≤2s；候选数增长被预算/缓存吸收。
