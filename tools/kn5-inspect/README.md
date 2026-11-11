# kn5-inspect

`.kn5` 模型检查 CLI：给 kn5 格式逆向与加载问题排查用，
基于 [`vettura-kn5`](../../kn5/)。

## 用法

```bash
cargo run -p kn5-inspect -- car.kn5 --nodes --materials --textures
cargo run -p kn5-inspect -- car.kn5 --depth 5              # 节点树更深
cargo run -p kn5-inspect -- car.kn5 --extract-dds /tmp/dds # 导出内嵌贴图
cargo run -p kn5-inspect -- --synth                        # 生成合成车模（无需真车资产）
```

| 参数 | 作用 |
| --- | --- |
| `path` | `.kn5` 文件路径（`--synth` 生成模式可省略） |
| `--textures` | 列出内嵌贴图 |
| `--materials` | 列出材质与贴图槽位 |
| `--nodes` | 打印节点层级 |
| `--depth N` | 节点树打印深度（默认 3） |
| `--extract-dds <dir>` | 把内嵌 DDS 写入目录 |

典型排查流程：车模加载不对 → `--nodes` 看 `WHEEL_*` 挂点位置与层级 →
`--materials` 看贴图槽位/ shader → `--extract-dds` 抽贴图单独验证。

格式细节见 [`docs/kn5-format.md`](../../docs/kn5-format.md)。
