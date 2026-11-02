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
├── project.godot              # Godot 工程配置（应用名、渲染、物理）
├── addons/
│   └── godot_mcp/             # Godot MCP 编辑器插件，供 AI/CLI 操作场景与脚本
└── test/                      # 本地测试资源（不纳入版本库，见下文）
```

`.godot/` 为编辑器导入缓存，`test/` 为本地大体积资源，二者均被 `.gitignore` 排除。

## 运行

1. 用 Godot 4.7 打开本目录下的 `project.godot`。
2. 首次打开时编辑器会重新生成 `.godot/` 导入缓存。
3. 按 `F5` 运行主场景。

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

- [`vettura-native`](https://github.com/Voiddd16/vettura-native)：Rust 侧引擎实现，负责 KN5/ACD 解析、Chrono::Vehicle 物理后端以及与 Godot 的桥接。
