# 纹样提取质量评估与改进调研（2026-11-21）

> 实测环境：`main@e154b08`；debug 二进制（`target/debug/patternstudio`、`-server`，server `PORT=8123` loopback）；vendored VTracer fork `1.0.0-alpha.4`（commit `8989194`）。
> 除特别注明外，所有数字均为本机实测，非估算。

## 结论（TL;DR）

- **质量门禁本身是绿的、结构指标是稳的**：`quality_baseline.rs` 2 个测试通过；CLI `poster` 预设对 4 个门禁样本的输出与 `baseline.json` 逐字节吻合。
- **但门禁只能防"结构漂移"，测不出"画得不像"**。用 8 种程序化纹样（512px、1.5px 细线）做端到端往返（SVG→PNG→trace→SVG→PNG，rsvg-convert + ImageMagick 独立度量）：smart 内部 fidelity 全部 0.94–1.00，而笔画召回（ink F1）实际只有 **0.37–1.00**，冰裂纹仅 **0.37**。
- **对退化输入（JPEG q60 + 缩小后的扫描纹样）存在灾难性失败**：ink F1 跌到 **0.00–0.08**、PSNR 14–18，而 smart fidelity 依然 0.96——它完全看不见笔画已经整体丢失。根因在 smart 的**族路由**：JPEG 彩噪抬高了 colorfulness，双峰门控把二值化（lineArt/bw）候选直接排除，Bradley–Roth 自适应二值化根本没被尝试。对照组证明输入本身只丢 11–28% 墨量，损失主体发生在追踪阶段。
- 最有性价比的三步：①把笔画召回纳入 smart 打分与质量门禁；②修复退化线稿的路由/自动预处理；③让 smart 尝试 fork 已支持但从未试过的 1.0 开关（watershed / cutout / palette / simplify）。

## 1. 评估对象与方法

管线：image → smart（分析 + 候选族试追踪打分）→ VTracer 1.0 追踪 → layered SVG。

三层度量：

| 层 | 指标 | 来源 |
| --- | --- | --- |
| 结构 | shape/layer/color count、svg_bytes | CLI `trace --json`、`--baseline` 门禁 |
| smart 内部 | fidelity（RMSE 合成回色图，或 lineArt 的墨迹 F1）、complexity（log bpp）、score=fidelity−λ·complexity | `POST /api/v1/smart` |
| 端到端独立 | PSNR、RMSE（ImageMagick compare）、ink F1（50% 二值化后像素级，纯 Python 对 PGM 计算） | rsvg-convert 光栅化源 SVG 与追踪 SVG |

样本：
- 门禁样本 5 张：`crates/patternstudio-cli/tests/fixtures/quality/`（**64/96px 微型合成图**）。
- `tests/fixtures/lineart.png`（256px）。
- **代表性补充**：项目自带 8 种程序化纹样生成器（团花/回纹/卷草/饕餮/冰裂纹/云纹/水纹/希腊回纹，`--size 512 --seed 7`），rsvg-convert 光栅化为 512px 白底细线（1.5px stroke）位图。
- **退化变体**：`scan` = 512→435px 缩小 + JPEG q60（模拟翻拍/扫描压缩）；`scan2` = 再放大回 512 + 高斯噪声。含输入退化对照组（源位图 vs 退化位图的 ink F1）。

## 2. 实测结果

### 2.1 质量门禁（WP4）

`cargo test -p patternstudio-cli --test quality_baseline`：**2 passed**。
CLI 结构指标 vs 基线（`poster` 预设）：bands 8 shapes/1707B、colors 9/1919B、duo 2 色 2 shapes/618B、lineart 2 色 3 shapes/919B——**与 baseline.json 完全一致**（基线即由 poster 预设生成）。gradient 不在基线内。

### 2.2 smart 自动模式（6 个位图样本）

| 样本 | 选定族 | 候选得分（降序） | 耗时 |
| --- | --- | --- | --- |
| bands | photo | photo .984 / color .983 / lowPoly .982 / flat .968 | 80ms |
| colors | color | color .991 / lowPoly .991 / flat .981 | 92ms |
| duo | color | color .993 / lowPoly .991 / flat .990 | 24ms |
| gradient | **photo ✓** | photo .955 / flat .922 / color .755 / lowPoly .753 | 114ms |
| lineart64 | color | color .989 / lineArt .915 / bw .816 | 28ms |
| lineart256 | lineArt | lineArt .998 / color .973 / bw .973 | 333ms |

gradient 上 photo 族以明显优势胜出，说明当 RMSE **不饱和**时打分是有效的。

### 2.3 8 种纹样端到端往返（smart 完整参数 → `/api/v1/trace` → 光栅化对比）

| 样本 | 选定族 | smart fid | shapes | SVG B | PSNR | ink F1 |
| --- | --- | --- | --- | --- | --- | --- |
| 团花 rosette | photo | 0.965 | 23 | 21.8K | 25.8 | 0.689 |
| 回纹 meander | flat | 0.985 | 36 | 14.4K | 36.9 | **0.997** |
| 卷草 scroll | lowPoly | 0.984 | 18 | 3.3K | 28.1 | 0.730 |
| 饕餮 taotie | lowPoly | 0.985 | 12 | 3.4K | 34.8 | 0.855 |
| 冰裂纹 ice_crack | photo | 0.955 | 29 | 60.6K | 22.5 | **0.372** |
| 云纹 cloud | lowPoly | 0.976 | 99 | 22.6K | 27.4 | 0.716 |
| 水纹 wave | flat | 0.941 | 30 | 32.5K | 24.1 | 0.562 |
| 希腊回纹 greek_key | lowPoly | 1.000 | 10 | 1.9K | ∞（像素级一致） | 1.000 |

### 2.4 退化变体与对照组

| 变体 | 选定族 | smart fid | ink F1 | PSNR | smart 耗时 |
| --- | --- | --- | --- | --- | --- |
| cloud-scan | lowPoly | 0.968 | **0.000** | 16.5 | 2.3s |
| cloud-scan2 | lowPoly | 0.962 | 0.775 | 26.4 | 5.0s |
| rosette-scan | photo | 0.962 | **0.032** | 15.8 | 3.2s |
| rosette-scan2 | lowPoly | 0.957 | 0.774 | 25.3 | 5.8s |
| meander-scan | flat | 0.981 | **0.082** | 18.4 | 1.4s |
| meander-scan2 | lowPoly | 0.964 | 0.903 | 28.9 | 4.5s |
| ice_crack-scan | lowPoly | 0.959 | **0.032** | 13.9 | 3.2s |
| ice_crack-scan2 | color | 0.959 | 0.625 | 24.6 | 10.3s |

**对照组**（只退化位图、不经追踪的 ink F1）：cloud 0.82 / rosette 0.77 / meander 0.89 / ice_crack 0.72。即输入退化本身只丢 11–28% 墨量，而整条管线只留下 0–8%——**损失主体在追踪/二值化环节**。

## 3. 发现

1. **RMSE fidelity 在白底/稀疏纹样上饱和**。云纹底上大段白色使 RMSE 天然极低，fid 恒为 0.94–1.00，即使结构塌缩（CLI 参数子集下的 wave 塌成 1 个 459B 形状、fid 仍 0.92）或笔画全丢（JPEG 组 fid 0.96、F1 0.0）。lineArt 族已用墨迹 F1 避免此坑，其余族没有。
2. **跨族得分不可比 + 族路由对退化输入失效**。512px 上 complexity ≤0.04，λ 项形同虚设，族胜负几乎由饱和的 fid 决定，photo/lowPoly 常以 RMSE 微弱优势胜出并产出 2–3 倍形状数（cloud 99 shapes/22.6KB；rosette photo 21.8KB）。更严重的是族候选只由 `bimodality ≥ 0.45 且 colorfulness < 20` 门控引入二值化候选；JPEG 彩噪使 colorfulness 超阈值后，**lineArt/bw（含唯一的 Bradley–Roth 自适应二值化候选）根本不进候选集** → scan 组全线崩。
3. **细线是系统性弱点**：1.5px 细线干净输入下 ink F1 也只有 0.37–0.86；纯几何图案（回纹/希腊回纹）近无损。弯曲细线在样条拟合处收缩/断裂是主因。
4. **fork 已支持、smart 从未尝试的 1.0 能力**：watershed 聚类、`hierarchical cutout`（无缝拼镶）、palette/max-colors 量化、simplify 容差、cutout+mode 组合。`TraceParams` 均已透传，只缺候选。
5. **smart 延迟**（debug 构建，为上限）：64–256px 样本 24–333ms；512px 干净纹样 0.6–1.2s；512px 高噪图（photo 族多候选试追踪）最高 10.3s。release 构建数字待测。
6. **工具一致性坑**：CLI `trace` 只接受参数子集，与 smart 完整参数行为不同（wave：CLI 1 shape vs 完整参数 30 shapes）。验证 smart 推荐必须走 server 完整参数。
7. **样本代表性缺口**：门禁样本是 64/96px 合成小图；本报告的 8 种 512px 纹样 + 退化组补上了主要盲区，但仍缺真实文物/织绣照片样本。

## 4. 改进方法调研（按预期收益/实现成本排序）

| # | 方法 | 预期收益 | 成本 | 依据 |
| --- | --- | --- | --- | --- |
| 1 | **笔画召回进打分与门禁**：非 lineArt 族也计算容差化墨迹 F1（或 chamfer 距离）参与 smart 打分；把 512px 纹样往返 + JPEG 变体纳入 `quality_baseline` 门禁（本报告数字可作初版基线） | 高：fid 不再能"说谎"，细线损失可回归检测 | 低 | 本报告 §2.3/2.4；StarVector 论文同样指出纯像素 MSE 不适用于 SVG 评测（arXiv:2312.11556） |
| 2 | **修复退化线稿路由 + 自动预处理**：低彩度+高纹理时强制加入 lineArt/bw 候选；对 smart 判为彩色的样本也尝试 `binary_adaptive`（Bradley–Roth，fork 已支持）；noise/软边统计超阈值时自动调用已有 `/api/v1/preprocess`（Real-ESRGAN） | 高：scan 组 F1 0.0→0.6–0.9（scan2 对照即为证据） | 低-中 | VTracer 1.0 README（adaptive/binarization 选项）；本报告对照组 |
| 3 | **尝试 fork 已有开关**：候选集中加入 watershed、cutout、palette/max-colors、simplify 容差族 | 中：彩色纹样文件减半、无缝拼镶利于分层编辑 | 中 | fork=1.0.0-alpha.4；VTracer 1.0 README |
| 4 | **对称感知纹样追踪**：FFT 旋转对称检测（Brubaker et al., CVPR 2010 实时旋转对称检测）+ 镜像/平移对称检测，对团花/回纹只追踪 1/n 区域再镜像成图 | 高（中式纹样专属：文件更小、对称严格、可编辑性好）| 中 | 领域知识 + 本报告证实几何图案 greek_key 已近无损、说明镜像合成可行 |
| 5 | **二值化升级**：全局 Otsu → Sauvola 局部窗（Θ(HW) 快速实现 arXiv:1905.13038；学习型 SauvolaNet arXiv:2105.05521，仅 40K 参数）；与 Bradley–Roth 横评 | 中：进一步压扫描/光照不均场景 | 低-中 | arXiv:1905.13038、2105.05521 |
| 6 | **分层语义化**：SAM 零样本分割（arXiv:2304.02643）预切区域再逐区分色追踪；训练语料参考 LVIS（CVPR 2023 大规模矢量图数据集） | 中-高：纹样分层从"按色分层"进化到"按部件分层" | 高 | SAM、LVIS；VTracer 1.0 README（cutout/watershed 是低配版替代） |
| 7 | **学习型矢量化的选择性引入**：SVGDreamer++（arXiv:2411.17832，HIVE 语义对象级矢量化）、StarVector（arXiv:2312.11556）、DeepSVG（arXiv:2007.11301）、Im2Vec（arXiv:2102.02798）。综述结论（arXiv:2306.06441）：现有 ML 方法慢且不够准，需要人机协同——与产品"smart 推荐 + 人工微调"定位一致，建议作远期实验而非当期依赖 | 高（上限）/ 低（近期） | 高 | 上述文献 + 综述 |
| 8 | **工程项**：用 release 构建复测 smart/trace 延迟；smart 试追踪结果按参数键缓存；quality job 已限定 ubuntu + PR 评审基线（OPTIMIZATION-PLAN.md），可补 512px 样本集 | — | 低 | 本报告 §3.5 |

## 5. 建议落地顺序

1. 门禁扩展（#1）→ 建立诚实基线；
2. 路由/自适应二值化修复（#2）+ fork 开关候选（#3）→ 一轮 A/B 看 F1/PSNR/大小三项指标；
3. 对称检测（#4）作为差异化特性立项；
4. #5 与 #2 合并横评；#6/#7 列入远期。

## 附录：复现步骤

```sh
# 门禁
cargo test -p patternstudio-cli --test quality_baseline
# smart（loopback server）
PORT=8123 target/debug/patternstudio-server &
curl -s -X POST 127.0.0.1:8123/api/v1/smart -H 'content-type: application/json' \
  -d '{"file_path":"<abs path>"}'
# 纹样样本
target/debug/patternstudio pattern -k rosette --size 512 --seed 7 -o rosette.svg
rsvg-convert -w 512 -h 512 -b white rosette.svg -o rosette.png
# 端到端往返：/api/v1/trace 传 smart 返回的完整 params；
# 两侧 rsvg-convert 光栅化后 magick compare -metric PSNR；
# ink F1：magick -colorspace Gray -threshold 50% -compress none 转 PGM 后逐像素计算
```

数据文件：`/tmp/ps-eval/`（smart-*.json、pat/*.svg|png、往返 SVG）。
