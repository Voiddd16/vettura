# patches/ — Godot 补丁序列

补丁按引擎版本分目录，文件名 `NNNN-short-name.patch`（`git format-patch` 默认
格式即可）。`series` 里每行一个文件名，按顺序应用；空行和 `#` 注释忽略。

```
patches/4.7/
├── series
└── 0001-...patch
```

## 当前

`4.7/series` 为空：FSR3.1 补丁尚未落地。运行时会把 `fsr3_1` 标为不可用并
回退到 stock 引擎的 FSR2/FSR1/MetalFX/原生。FSR3.1 要碰的引擎层与验收清单
见 [`4.7/FSR31_DESIGN.md`](4.7/FSR31_DESIGN.md)。

## 验收

对每个补丁：

1. 干净 checkout 上 `git apply --check` 通过；
2. `scons` 构建目标平台成功；
3. `godot --headless --path <project> res://game/display_smoke.tscn` 通过；
4. 桌面窗口跑一次设置页，切换新上采样器无报错；
5. 记录在 `CHANGELOG.md`（版本、Godot 提交、补丁顺序、验证机器）。

不要提交 Godot 源码目录或构建产物；本目录只放 patch 文本。
