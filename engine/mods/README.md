# mods/ — 引擎扩展 manifest

`DisplayPipeline` 在启动时扫描：

- `res://engine/mods/*/manifest.json`（工程内随版本分发）
- `user://engine/mods/*/manifest.json`（本机安装）

只读取 manifest，不加载代码。manifest 只能**声明**一个已经由引擎注册的
singleton；真正的 native 代码必须在引擎/扩展侧编译好。这样脚本侧不会执行
任意路径的 `.dll/.so/.dylib`。

## manifest 格式（API v1）

```json
{
  "api": 1,
  "id": "fsr31",
  "name": "AMD FSR 3.1",
  "version": "1.0.0",
  "enabled": true,
  "provides": ["upscaler.fsr3_1"],
  "engine": {
    "min": "4.7",
    "patches": ["0001-fsr31.patch"]
  },
  "bridge": {
    "singleton": "VetturaEngineMod",
    "methods": {
      "available": "display_available",
      "apply": "display_apply",
      "clear": "display_clear",
      "status": "display_status"
    }
  }
}
```

字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `api` | 是 | 当前固定 `1`；不匹配直接忽略 |
| `id` | 否 | 默认取目录名 |
| `enabled` | 是 | `false` 的模板不会被加载 |
| `provides` | 是 | 能力字符串数组，目前认 `upscaler.fsr3_1` |
| `bridge.singleton` | 否 | 引擎已注册的 singleton 名；没有则只能靠 Viewport 枚举 |
| `bridge.methods` | 否 | 方法名映射；缺省值见上 |

## 桥接方法契约

```cpp
// 引擎 C++ 侧（示意）
bool display_available(const String &id);          // id = "fsr3_1"
bool display_apply(const Dictionary &request);     // 成功返回 true
void display_clear();                              // 切走时恢复引擎默认
Dictionary display_status();                       // 调试信息（可选）
```

`display_apply` 的 `request`：

```json
{
  "id": "fsr3_1",
  "scale": 0.78,
  "target_fps": 60.0,
  "viewport_rid": "<RID>"
}
```

`DisplayPipeline` 在以下时机调用：

- 用户在设置里选择 `fsr3_1`，或 `auto` 策略选中它；
- 动态分辨率改变 `scale`（引擎应读取最新 scale，不必每次都重建资源）；
- 切换到其他上采样器时调用 `display_clear()`。

如果 `display_available` 返回 `false`、方法不存在或 `display_apply` 返回
`false`，运行时会**自动回退**到 FSR2/MetalFX/FSR1/原生，不阻断画面。

> 走桥接时 `DisplayPipeline` **不会**替你设置 `Viewport.scaling_3d_mode`：
> 引擎侧 `display_apply` 接管整条 upscale 链，必须自己决定内置 scaling
> （通常设成原生/BILINEAR，避免和自定义 FSR3.1 pass 双重放大）。
> 只有走 `SCALING_3D_MODE_FSR3_1` 枚举时，才由运行时像普通模式一样设置。

## 安全边界

- manifest 不是插件加载器；它不执行路径里的代码；
- 只扫描固定根目录，不递归；
- 引擎 singleton 必须由可信的引擎/扩展构建注册；
- 发行版不要把测试用 manifest 留在 `engine/mods/`，用户自装放
  `user://engine/mods/`。
