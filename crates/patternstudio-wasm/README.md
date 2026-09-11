# patternstudio-wasm

PatternStudio 的 WASM 绑定（基于 [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/)），可在浏览器中做图片 → 分层 SVG 转换和纹样生成。

## 构建

```bash
# 需要安装 wasm-pack: cargo install wasm-pack
cd crates/patternstudio-wasm
wasm-pack build --target web --out-dir pkg

# 然后用任意静态服务器打开 demo
python3 -m http.server 8080
# 访问 http://localhost:8080/web/
```

## API

| 函数 | 说明 |
|---|---|
| `traceBytes(data, preset, resolution, simplify, layers)` | 图片字节 → 分层 SVG 字符串（wasm-bindgen 无默认参数，全部必传） |
| `generatePattern(kind, nUnits, complexity, size)` | 生成程序化纹样 SVG |
| `listPatternKinds()` | 列出可用纹样（"kind - 描述"） |

参数取值：
- `preset`: `bw` / `poster` / `photo`
- `resolution`: `coarse` / `medium` / `fine`
- `simplify`: `minimal` / `low` / `medium` / `high` / `maximum`
- `layers`: 自动按颜色分层的最大层数（0 = 单层）

## 使用示例

```js
import init, { traceBytes, generatePattern } from "./pkg/patternstudio_wasm.js";

await init();

// 图片 -> 分层 SVG
const bytes = await (await fetch("photo.png")).arrayBuffer();
const svg = traceBytes(bytes, "poster", "medium", "low", 3);
document.body.innerHTML = svg;

// 纹样生成
const pattern = generatePattern("rosette", 8, 50, 400);
```
