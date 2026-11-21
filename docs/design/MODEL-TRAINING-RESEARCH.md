# 纹样提取模型与纹样生成模型训练调研

> 状态：调研报告（纯调研，不改代码）。承接 `QUALITY-IMPROVEMENT-PLAN.md` §6 延后项表的两条触发线："学习型矢量化"与"SAM 语义分层"。引用规范：**只引本次调研经 arXiv API/检索页核实过的条目**，以 arXiv id 为准；早期笔记中未经核实的 id（M2SVG、SVG-1、Vector-FLAN、LLaVA-SVG 等）在本报告 §6 明示剔除。

## 0. 结论先行

1. **提取侧不建议训练端到端大模型替换 vtracer 主链路**。性价比最高的路线是"三段式"：开源小 VLM（3B 级，StarVector/OmniSVG 式 SVG token 化）→ 用 `patternstudio-patterns` 生成器合成纹样做 LoRA SFT → 用**现有 ink F1 门禁当可验证奖励**做 RL 后训练。落地形态是 opt-in 候选臂（对齐 QWP4 第 5 条 `extended` 的先例），0 胜即降级，不进主链路。
2. **语义分层已有比"onnxruntime 预切"更成熟的方案**：学习型分层（Illustrator's Depth / AmodalSVG / SemLayer）把"哪些像素属于哪一层"直接当预测任务，且都有矢量下游验证。§6 的 SAM 预切方案可升级为"分层先验 + 逐区矢量化"。
3. **生成侧差异化卖点 = 程序装配 × 模型内容**：模型只生成基本扇区（纹样的 1/n），对称性由 QWP7 装配器（`plan_for`/`sector_image`/`write_symmetric`）程序保证、`fold_f1≥0.97` 折叠验证门直接复用为生成验证门。这是文献中没有、项目独有的结构。
4. **数据是项目最大优势**：patterns 参数化生成器可无限产 raster/vector 配对 + 对称标注，无需依赖 MMSVG-2M/SVG-Stack 级别的大数据即可启动 SFT（ReDraw 经验：1.2M 合成 + 320 张真实即够域适配）。
5. 现有 ink F1 度量在模型时代价值反而变大：综述与评测工作实证 CLIPScore 类指标对矢量的颜色/数量/空间错误**几乎不敏感**（2609.03806），自渲染 + 墨迹 F1 是天然低 hacking 面的域内奖励。

## 1. 背景与问题定义

现状：经典管线（vtracer + smart 选型）经 QWP1–QWP7 已达——干净 8 样本 + scan 组 ink F1 0.617–1.000，smart 延迟 release 56–288ms。模型训练只在两类场景有意义（对齐 §6 触发条件）：

- **提取侧**：某类真实输入（照片底、水彩渐变、密排曲线）经典候选 F1 仍 <0.5 且量大；或需要**语义分层**（cutout/watershed 满意度 <70%）。
- **生成侧**：参数化生成器只覆盖几何族（团花/回纹/云纹…），语义纹样（饕餮、Scroll 类）与无限变体需要模型。

## 2. 提取侧调研（raster→vector / 分层）

### 2.1 五条路线

**A. 自回归 VLM 直接产 SVG 代码**
- StarVector（2312.11556）：PaliGemma-3B 基座，SVG-Stack 2M 训练，SVG-Bench 10 数据集 3 任务（含 Image-to-SVG）。证明 3B 级模型在图→矢量上可用。
- OmniSVG（2504.06263）：VLM 端到端多模态 SVG 生成，**把 SVG 命令与坐标参数化为离散 token**，结构逻辑与低层几何解耦；随发布 MMSVG-2M（200 万带标注 SVG 资产）与统一评测协议。当前该路线公开最强配方。
- 综述（2503.04983）：generation/editing/understanding 分类；结论——**reasoning 增强模型优于标准 LLM**；数据集多样性普遍不足。

**B. 扩散/逐样本优化**
- SVGDreamer（2312.16476，CVPR 2024）：文本引导，SIVE 语义驱动矢量化 + VPSD 粒子化得分蒸馏 + reward model 重加权。
- SVGDreamer++（2411.17832）：HIVE **分层**矢量化（借图像分割先验做语义对象级）+ 自适应 primitive 控制。
- 特点：质量高但**每图迭代优化、推理慢**，与 512px ≤2s 延迟线冲突，只适合作离线质量上限参照，不适合作产品臂。

**C. 区域/分层先验 + 局部矢量化**（对 §6"SAM 语义分层"的直接升级）
- SAM（2304.02643）/ SAM2（2408.00714）：零样本文本/点提示分割，作区域预切的经典方案（§6 原案）。
- Illustrator's Depth（2511.17454）：给像素预测**层序索引**，在分层矢量数据集上训练网络直接从 raster 预测分层；作者实证可显著改进矢量化 SOTA，并支持 text-to-vector-graphics。
- AmodalSVG（2604.10940）：两阶段——VLM 引导 Semantic Layer Peeling（逐层剥离 + hybrid inpainting 补全遮挡）→ Adaptive Layered Vectorization（**误差预算驱动 primitive 配额**）。其"误差预算→基元预算"与 QWP3"候选预算"思想同构。
- SemLayer（2603.24039，CVPR 2026）：把扁平化图标恢复为**带遮挡关系的分层矢量**——语义着色生成→语义补全→矢量装配。与"从扫描的扁平纹样恢复可编辑分层"需求几乎同题。

**D. RL 后训练（可验证奖励）**——与项目门禁天然对接
- RLRF（2505.20793）：自回归 VLM 生成 SVG→渲染→与原图比较得奖励；实证 RL 显著优于 SFT，修正常见失败模式。
- IR4RL（2609.34587）：**中间渲染进度奖励**——代码前缀已可渲染出半成品，把渲染间差异做成 token 级过程监督；超 SFT 与标准 GRPO，开源模型达 SOTA。
- Symbolic Graphics Programming（2509.05208，TMLR）：**格式有效性门 + SigLIP/DINO 跨模态可验证奖励**，7B 模型经 RL 达前沿水平；并观察到 RL 诱导更细的基元分解。
- CTRL-S（2603.16189）：SVG-Sophia 145K（代码精炼/Text-to-SVG/Image-to-SVG），GRPO 多奖励（DINO、图文相似、格式、代码效率）。
- Reason-SVG（2505.24499，CVPR 2026）：Drawing-with-Thought——同时产 SVG 与设计理由，SFT+GRPO 混合奖励。
- Visual-ERM（2603.13224）：在**渲染视觉空间**里做生成式奖励模型，SVG 解析任务稳定增益。
- 评测侧佐证：SVG-Score（2609.03806）实证 CLIPScore 对矢量的颜色/数量/空间关系错误几乎不响应；RULER（2609.25270）用 rubric 化 VLM 判分替代标量指标以避免 reward hacking。→ **项目自渲染 ink F1 作为域内可验证奖励，比借用自然图像指标可靠**。

**E. 域适配小样本配方**
- ReDraw（2611.24084）：汉简残损汉字→两阶段 VLM（忠实矢量化 + skeleton CoT 引导的补全重绘），**ink/材质/退化三层解耦合成 1.2M 训练样本，真实标注 320 字派生 ≤6.4K 对做适配**，结构感知奖励 RL。这是所有文献里与"文物纹样扫描提取"最贴近的配方：**大规模合成解耦退化 + 少量真实样本对齐**。

### 2.2 训练方案建议（提取侧）

| 阶段 | 做法 | 数据 | 算力 |
|---|---|---|---|
| S0 复用 | 选开源 VLM 基座（StarVector/OmniSVG 类，≤3B），tokenizer 用命令/坐标离散 token 方案 | — | 0 |
| S1 合成 SFT | patterns 生成器按族 × 参数网格 × 风格采样；raster 侧套 ReDraw 式退化层（JPEG 噪声/纸底/墨损——scan/scan2 脚本已有雏形）；标注含对称族信息 | 5×10⁴–10⁵ 对起步 | 单卡 A100/4090 级 LoRA |
| S2 真实适配 | 授权文物图扫 + 人工修正矢量，数百张派生数千对 | ≤10³ 真实 | 同上 |
| S3 RL（可选） | GRPO + 混合奖励：SVG 格式门 + ink F1（膨胀 r=2，现门禁脚本）+ 字节数惩罚 | rollout 渲染现成 | 渲染绑定，GPU 富余则并行 |

- **集成方式**：`/smart` 增加 opt-in 臂（形态 = QWP4 的 `extended`），同图同 ink F1 A/B；**0 胜即移出默认集**，不回调阈值凑验收（QWP3/QWP4 先例）。
- **延迟**：3B VLM 512px 单次前向约 0.5–2s（GPU），不进主链路；必要时蒸馏为轻量后处理（§6 原案措辞依然成立）。
- **分层支线**：若要语义分层，优先试 Illustrator's Depth 式层序预测或 SemLayer 式分层重建 + 逐区矢量装配，而非 SAM 裸预切。

## 3. 生成侧调研（新纹样生成）

### 3.1 四条路线

**A. SVG 自回归 LM**：StarVector/OmniSVG 同款基座可 Text-to-SVG；SVGThinker（2509.24299）给出低成本数据配方——**按序渲染 primitive 再让多模态模型反向标注图+代码**，CoT 暴露中间推理；VectorGym（2603.29852，EMNLP 2026）提供 4 任务基准（含 Sketch2SVG、VG-Edit）与 GRPO 课程训练配方（8B 达开源 SOTA）；RULER（2609.25270）在**无配对 ground truth**时用 rubric 奖励——适合纹样这种风格化内容。
**B. SVG 扩散优化**：SVGDreamer/SVGDreamer++，免训练但每图慢；适合离线出高质感参照图，不适合产品路径。
**C. raster 基础模型 + 级联矢量化**：server 已有 Qwen-Image edit 路由（redraw）——**文/图→raster→本项目提取管线**的级联今天就能拼出来，作为对照臂零训练成本；再往下是纹样风格 LoRA。
**D. 程序装配 × 模型内容（推荐主打）**：模型只生成**基本扇区**（内容/母题），由 QWP7 装配器程序化生成 D_n/C_n/C1v 全图——对称精确性由代码保证而非模型保证，`fold_f1` 验证门天然成为生成质量门禁。文献均为"模型直出整图"，无此结构先验约束，是差异化卖点；也天然继承参数化生成器"每份输出可编辑分层 SVG"的性质。

### 3.2 训练方案建议（生成侧）

- 数据：**复用 §2.2 S1 同一合成管线**（patterns 生成器 pair 数据 + 族/风格文本标签即条件生成的 prompt 空间）；对称族样本额外存扇区标注。10⁴–10⁵ 即可验证单族。
- 模型：0.5–3B 自回归 LM（纹样级 SVG 序列短）；训练 = SFT + GRPO（格式门 + 渲染 ink F1 + 多样性奖励防塌缩）。
- 评估三层：①生成 SVG 往返一致性（矢量→栅格→现有管线提取→ink F1，生成侧同样吃门禁红利）；②多样性（pairwise 嵌入距离/去重率，防 mode collapse）；③VLM-as-judge rubric 抽样（借鉴 VectorGym/SVG-Score 协议），release 数字才作报告口径。
- 无缝平铺：检索未发现必须训练专用 seamless 模型的证据；patterns 生成器本身产无缝 tile，模型只需管 tile 内容，程序负责平铺。

## 4. 成本与风险

| 路线 | 训练成本 | 推理成本 | 主要风险 |
|---|---|---|---|
| 提取 A+RL（M1/M3） | LoRA 单卡天级 | 0.5–2s/图（GPU） | 语法有效率；真实文物泛化差 |
| 分层 C | 层序预测需分层矢量数据（可用合成造） | 中 | 分层粒度与产品语义不符 |
| 生成 D（扇区） | 最低（数据现成） | 低 | 扇区内容风格漂移（折叠门只保证对称不保证美） |
| 生成 C（级联） | 0（拼装） | raster 扩散秒级 | 级联误差叠加：redraw 结果过提取臂可能比直出更差，先测 |
| 扩散 B | 0 | 每图分钟级 | 只作参照 |

通用风险：reward hacking（ink F1 膨胀容差可能被"画粗墨线"投机——可加字节数/线宽惩罚，GeoSVG-RL 2605.25447 的多维可执行奖励是参考）；数据许可（用 MMSVG-2M/SVG-Stack 前先查许可，本项目优先自合成）；真实样本缺位使 real 区只能监控不能门禁（§6 第 3 条不变）。

## 5. 建议里程碑

- **M0（数天，零训练）**：把现有门禁脚本封装成可被 RL harness 调用的奖励函数；固定 16 样本作模型臂 A/B 基线；**先测路线 C 级联**（Qwen-Image redraw → /trace）作为最廉价对照臂。
- **M1（1–2 周）**：提取侧 S1 试点——3B 基座 LoRA + 5×10⁴ 合成纹样；判据：**经典臂 F1<0.5 的样本类上，模型臂中位提升 ≥0.05 才立为 opt-in 候选**（同 QWP5 判据口径），否则只留实验记录。
- **M2（1 周）**：生成侧路线 D 试点——扇区生成 + QWP7 装配，验收 = 装配臂 fold_f1 ≥0.97 且人工评审风格合格。
- **M3（条件启动）**：仅当 M1/M2 SFT 有效但触顶，上 GRPO + 混合奖励。
- 与 §6 对齐：**§6 触发条件未成立前，M1 起不动工**，本档仅存档；触发时按 M0→M1 顺序重启。

## 6. 引用清单与勘误

**已核实（本次或上次经 arXiv 检索取回元数据/摘要原文）**：
2312.11556 StarVector · 2312.16476 SVGDreamer · 2411.17832 SVGDreamer++ · 2503.04983 LLM-for-SVG 综述 · 2504.06263 OmniSVG · 2509.24299 SVGThinker · 2505.20793 RLRF · 2505.24499 Reason-SVG · 2509.05208 Symbolic Graphics Programming · 2609.34587 IR4RL · 2609.25270 RULER · 2609.03806 SVG-Score · 2603.29852 VectorGym · 2603.16189 CTRL-S · 2603.13224 Visual-ERM · 2605.25447 GeoSVG-RL · 2607.28073 GVR-Coder · 2605.01517 VAnim · 2611.24084 ReDraw · 2511.17454 Illustrator's Depth · 2604.10940 AmodalSVG · 2603.24039 SemLayer · 2304.02643 SAM · 2408.00714 SAM2

**勘误与剔除**：早期笔记缓存 id 有误——2412.16175/2504.14633/2506.09945/2503.16421 经 id_list 核对分别为金融 RL/金融 NER/视频生成/流体论文，与 SVG 无关；OmniSVG 正确 id 为 **2504.06263**。M2SVG、SVG-1、Vector-FLAN、LLaVA-SVG 在 arXiv API 与检索页均查无，**不予引用**。中文传统纹样专用生成论文经多种措辞检索未命中，生成侧方案未依赖此类工作。
