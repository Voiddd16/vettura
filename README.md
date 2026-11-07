# Vettura

Vettura 是一款以 **Godot 4** 为运行时的赛车游戏主工程，目标是导入 Assetto Corsa 的车模与物理数据（KN5 / ACD），并在移动端风格的渲染管线下运行。

## 环境要求

| 项目 | 版本 |
| --- | --- |
| Godot | 4.7（`mobile` 渲染后端） |
| 物理引擎 | Jolt Physics（`project.godot` 中已启用） |
| 目标平台 | 移动端为主，桌面端可运行 |

## 目录结构

```
vettura/
├── project.godot              # Godot 工程配置（应用名、渲染、物理、主场景）
├── test_visual/               # 测试性开发页面：默认车模浮空预览 + 触摸 + 多视角
├── addons/
│   └── godot_mcp/             # Godot MCP 编辑器插件，供 AI/CLI 操作场景与脚本
└── test/                      # 本地测试资源（不纳入版本库，见下文）
```

`.godot/` 为编辑器导入缓存，`test/` 为本地大体积资源，二者均被 `.gitignore` 排除。
`vettura-native` 通过软链接暴露为 `res://vettura-native/`；如果它不在本目录下，
先在 `vettura-native` 仓库里执行 `./scripts/link_into_godot.sh`。

## 运行

1. 用 Godot 4.7 打开本目录下的 `project.godot`。
2. 首次打开时编辑器会重新生成 `.godot/` 导入缓存。
3. 按 `F5` 运行主场景：`res://test_visual/main.tscn`。
4. 也可以用无头模式验证交互链路：

```bash
godot --headless --path . res://test_visual/smoke.tscn
# 期望最后一行 [visual-smoke] PASS，退出码 0
```

`test_visual/main.tscn` 这一版做的是：

- 默认 F1 2008 Ferrari 车模浮空预览（不接赛道物理）；
- 左半屏触摸 = 刹车，右半屏触摸 = 油门；
- 右上角按钮循环切换视角：车后 / 驾驶位 / 绕车 / 前机盖；
- HUD 显示当前油门/刹车与车轮角速度。

## godot_mcp 插件

`addons/godot_mcp` 提供 MCP 服务，让外部工具（AI 助手、CLI）可以直接查询节点树、读写脚本、修改场景。插件默认在编辑器启动时加载，配置项位于 编辑器 → 项目设置 → `godot_mcp`。使用说明见 `addons/godot_mcp/README.zh.md`。

## 本地测试资源

`test/` 目录用于存放导入验证用的 Assetto Corsa 内容，体积达数 GB（含多个 100 MB+ 的 `.kn5` / `.psd`），超出 GitHub 的仓库与 LFS 配额，因此 **不提交到版本库**。需要复现测试时，请在本地按下列结构放置：

```
test/
├── F1 2008 by Cimmerian_Iter/   # 测试用车模（KN5 + 纹理 + data.acd）
└── actools/                     # KN5/ACD 解析参考实现（第三方仓库克隆）
```

资源仅用于本地开发验证，请勿将其提交或再分发。

## 相关仓库

- [`vettura-native`](https://github.com/Voiddd16/vettura-native)：Rust 侧引擎实现，负责 KN5/ACD 解析、Chrono::Vehicle 物理后端以及与 Godot 的桥接。声音也是同一条链：`FmodSystem` 节点直接调用 FMOD runtime 播放 AC 车模 `sfx/*.bank`（native 侧需 `--features fmod` 构建）。
