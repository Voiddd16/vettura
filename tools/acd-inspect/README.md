# acd-inspect

`data.acd` 解密/解包 CLI：给 ACD 格式逆向与参数排查用，
基于 [`vettura-acdata`](../../acdata/)。

## 用法

```bash
cargo run -p acd-inspect -- "mods/<车目录>/assettocorsa/content/cars/<车>/data.acd"
cargo run -p acd-inspect -- data.acd --extract /tmp/acd_out     # 解包全部文件
cargo run -p acd-inspect -- data.acd --print car.ini            # 单文件打到 stdout
cargo run -p acd-inspect -- data.acd --key <key> --quiet        # 手动指定 key
```

| 参数 | 作用 |
| --- | --- |
| `path` | `data.acd` 路径；**key 从其父目录（车目录）名推导** |
| `--key <KEY>` | 覆盖自动推导的 key |
| `--print <NAME>` | 打印某个解密后的文件 |
| `--extract <DIR>` | 解包全部文件到目录 |
| `--quiet` | 隐藏逐文件列表 |

**注意**：ACD key 由车目录名推导——车目录**改名后 key 就错了**，
解不开时先确认目录名与 AC 原版一致。

格式细节见 [`docs/ac-data.md`](../../docs/ac-data.md)。
