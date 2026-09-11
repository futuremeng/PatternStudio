# patternstudio-py

PatternStudio 的 Python 绑定（基于 [PyO3](https://pyo3.rs)）。

## 构建

```bash
# 开发：构建后把 .so 拷到 Python 可导入的位置
cargo build -p patternstudio-py
cp target/debug/libpatternstudio.so ~/.local/lib/  # 或任意 sys.path 目录

# 推荐：用 maturin 打包成 wheel
pip install maturin
maturin build -m crates/patternstudio-py/Cargo.toml
pip install target/wheels/patternstudio_py-*.whl
```

## 使用

```python
import patternstudio

# 图片 -> 分层 SVG（自动按颜色分 3 层）
svg = patternstudio.trace_file("photo.png", "out.svg", layers=3)

# 直接从字节流转换
with open("photo.png", "rb") as f:
    svg = patternstudio.trace_bytes(f.read(), preset="poster", layers=2)

# 程序化纹样生成
svg = patternstudio.generate_pattern("rosette", n_units=8, complexity=50, size=400)

# 查看可用纹样
for kind in patternstudio.list_pattern_kinds():
    print(kind)
```

## API

| 函数 | 说明 |
|---|---|
| `trace_file(path, output=None, preset="poster", resolution="medium", simplify="low", layers=0)` | 图片文件 → 分层 SVG 字符串；`output` 非空时同时写盘 |
| `trace_bytes(data, preset="poster", resolution="medium", simplify="low", layers=0)` | 图片字节 → 分层 SVG 字符串 |
| `generate_pattern(kind, n_units=8, complexity=50, size=400)` | 生成程序化纹样 SVG |
| `list_pattern_kinds()` | 列出可用纹样（"kind - 描述"） |

参数取值：
- `preset`: `bw` / `poster` / `photo`
- `resolution`: `coarse` / `medium` / `fine`
- `simplify`: `minimal` / `low` / `medium` / `high` / `maximum`
- `layers`: 自动按颜色分层的最大层数（0 = 单层）
