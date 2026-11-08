# Assetto Corsa 数据格式笔记

## 车目录

典型 mod 车目录：

```
cim_2008_ferrari/
├── data.acd          # 加密/打包的物理数据（优先）
├── data/             # 解包后的同名文件（回退）
│   ├── car.ini
│   ├── engine.ini
│   ├── suspensions.ini
│   ├── tyres.ini
│   ├── drivetrain.ini
│   ├── aero.ini
│   ├── power.lut
│   └── ...
├── cim_2008_ferrari.kn5
├── collider.kn5
├── skins/
└── sfx/
```

`vettura-assets::CarFolder` 的行为：

1. 如果 `data.acd` 存在，读取并解密整个容器到内存；
2. 否则递归读取 `data/`；
3. 文件名统一为相对路径（`/` 分隔），大小写按原样保存，查询时文件系统
   语义由调用者决定。

主模型 `.kn5`（渲染用，不含 `collider.kn5`）的选择规则与 Content Manager 的
`AcPaths.GetMainCarFilename` 一致：

1. `lods.ini` 的 `[LOD_HR] FILE`（或 `[LOD_0] FILE`）优先；
2. 否则取车目录下体积最大的 `*.kn5`——`collider.kn5` 通常远小于完整车模。

`CarFolder::main_model_path()` 返回该路径；AcManager 的 Workshop 上传流程用的是
同一个约定。

## `data.acd` 容器

解密后的容器是连续记录：

```
i32 name_len
u8  name[name_len]
u32 plain_len
u8  encoded[plain_len * 4]
```

- `name_len` 为负数时，紧跟一个 `u32` 作为真实长度（少数写入器使用）；
- 每个明文 byte 占 4 字节；
- 4 字节组内值为 0 的字节被丢弃；
- 其余字节：`plain = encoded.wrapping_sub(key[i % key.len()])`。

### Key 推导

Key 是 8 个十进制数拼成的字符串，例如：

```
172-5-6-58-214-253-22-106
```

真正的 cipher key 是这个字符串的 ASCII 字节，循环使用。

8 个部分来自**车文件夹名**（`data.acd` 的父目录名）的字符运算。实现见
`vettura-acdata/src/acd.rs` 的 `derive_key`。已知测试向量：

```rust
assert_eq!(derive_key("cim_2008_ferrari"), "172-5-6-58-214-253-22-106");
```

因此 `data.acd` 不能脱离原文件夹单独解密。`AcdArchive::from_path` 会自动
取父目录名；命令行工具也支持 `--key` 覆盖。

## INI

AC INI 的特点：

- `[SECTION]` 名称通常大写；
- key 大小写不敏感；
- `;`、`#`、`//` 开头为注释；
- 行尾 `;` 注释**不剥离**（值里可能包含分号）；
- 同一个 key 可能出现多次，当前保留全部并按首次匹配查询。

`Ini` 保留 section 和 entry 的原始顺序，方便以后做无损编辑。

常用字段：

| 文件 | Section | Key | 含义 |
| --- | --- | --- | --- |
| `car.ini` | `BASIC` | `TOTALMASS` | 整车质量 kg |
| `car.ini` | `INFO` | `SCREEN_NAME` | 显示名 |
| `suspensions.ini` | `BASIC` | `WHEELBASE` | 轴距 m |
| `suspensions.ini` | `BASIC` | `CG_LOCATION` | 质心纵向比例 |
| `suspensions.ini` | `FRONT/REAR` | `TRACK` | 轮距 m |
| `suspensions.ini` | `FRONT/REAR` | `SPRING_RATE` | 轮端弹簧 N/m |
| `suspensions.ini` | `FRONT/REAR` | `DAMP_BUMP/REBOUND` | 阻尼 N·s/m |
| `suspensions.ini` | `FRONT/REAR` | `BASEY` | 轮心到质心高度分量 |
| `suspensions.ini` | `ARB` | `FRONT/REAR` | 防倾杆 |
| `tyres.ini` | `FRONT/REAR` | `RADIUS` | 轮胎半径 m |
| `tyres.ini` | `FRONT/REAR` | `WIDTH` | 轮胎宽度 m |
| `tyres.ini` | `FRONT/REAR` | `DY0/DX0` | 峰值侧向/纵向摩擦系数 |
| `engine.ini` | `ENGINE_DATA` | `MINIMUM` | 怠速 rpm |
| `engine.ini` | `ENGINE_DATA` | `LIMITER` | 断油转速 rpm |
| `drivetrain.ini` | `TRACTION` | `TYPE` | `RWD/FWD/AWD` |
| `drivetrain.ini` | `GEARS` | `COUNT/GEAR_n/FINAL` | 齿比 |
| `aero.ini` | `WING_0` | `CHORD/SPAN/CD_GAIN` | 简化气动参数 |

## LUT

文本格式，常见两列：

```
0|0
1000|120
2000|260
```

也接受逗号或空白分隔。解析时：

- 跳过空行和注释；
- 非法行直接忽略；
- 按 x 排序；
- `sample(x)` 做线性插值，超出范围钳制到首/尾值。

`power.lut` 是 `rpm|torque`，当前映射取最大 torque 作为
`EngineSpec::peak_torque_nm` 的简化值。后续应改为完整曲线。

## 待办

- `curves` / `.rto`（齿比文件）支持；
- `setup.ini` 的可调参数；
- 轮胎温度/磨损/压力曲线；
- 无损 INI 编辑与回写；
- `data.acd` 重新加密（目前只读）。
