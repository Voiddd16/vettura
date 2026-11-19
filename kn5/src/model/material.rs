//! 材质数据模型：混合模式、深度模式、shader 属性与贴图槽。

/// 材质混合模式，在文件中占 `u8`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Opaque,
    AlphaBlend,
    AlphaToCoverage,
    /// 未知值不会导致解析失败，而是原样保留，行为与 AcTools 一致。
    Unknown(u8),
}

impl BlendMode {
    /// 从原始字节构造，未知值进入 [`BlendMode::Unknown`]。
    pub fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Opaque,
            1 => Self::AlphaBlend,
            2 => Self::AlphaToCoverage,
            other => Self::Unknown(other),
        }
    }

    /// 返回写回文件时应使用的原始字节。
    pub fn as_raw(self) -> u8 {
        match self {
            Self::Opaque => 0,
            Self::AlphaBlend => 1,
            Self::AlphaToCoverage => 2,
            Self::Unknown(value) => value,
        }
    }
}

/// 材质深度模式，在文件中占 `i32`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DepthMode {
    /// 正常深度测试并写入深度。
    Normal,
    /// 保留深度测试，但关闭深度写入（只读）。
    NoWrite,
    /// 关闭深度测试。
    ///
    /// 这也是 AcTools 遇到未知值时的回退值。
    #[default]
    Off,
    /// 未知值不会导致解析失败，而是原样保留，行为与 AcTools 一致。
    Unknown(i32),
}

impl DepthMode {
    /// 从原始整数构造，未知值进入 [`DepthMode::Unknown`]。
    pub fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::NoWrite,
            2 => Self::Off,
            other => Self::Unknown(other),
        }
    }

    /// 返回写回文件时应使用的原始整数。
    pub fn as_raw(self) -> i32 {
        match self {
            Self::Normal => 0,
            Self::NoWrite => 1,
            Self::Off => 2,
            Self::Unknown(value) => value,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Material {
    pub name: String,
    /// shader 名称，例如 `ksPerPixelMultiMap`。
    pub shader: String,
    pub blend_mode: BlendMode,
    pub alpha_tested: bool,
    pub depth_mode: DepthMode,
    pub properties: Vec<MaterialProperty>,
    /// 贴图槽绑定。KN5 中称为 texture mapping。
    pub samples: Vec<TextureSample>,
}

impl Material {
    /// 返回指定 shader 属性的第一个标量分量。
    ///
    /// KN5 属性总是按四个分量存储，但大多数属性只使用 `ValueA`
    /// （例如 `ksAmbient`）。其余分量有意义时请使用
    /// [`Material::property_values`]。
    pub fn property(&self, name: &str) -> Option<f32> {
        self.property_values(name).map(|p| p.value_a)
    }

    /// 按名称查找完整属性值，忽略 ASCII 大小写。
    pub fn property_values(&self, name: &str) -> Option<&MaterialProperty> {
        self.properties
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    /// 返回某个贴图槽（例如 `txDiffuse`）绑定的贴图名。
    pub fn texture(&self, slot_name: &str) -> Option<&str> {
        self.texture_sample(slot_name).map(|s| s.texture.as_str())
    }

    /// 按名称查找贴图槽，忽略 ASCII 大小写。
    pub fn texture_sample(&self, slot_name: &str) -> Option<&TextureSample> {
        self.samples
            .iter()
            .find(|s| s.slot_name.eq_ignore_ascii_case(slot_name))
    }
}

/// 一个 shader 属性。KN5 固定写入 `f32`、`vec2`、`vec3`、`vec4` 四组值；
/// 多数材质属性只使用 `value_a`。
#[derive(Debug, Clone)]
pub struct MaterialProperty {
    pub name: String,
    pub value_a: f32,
    pub value_b: [f32; 2],
    pub value_c: [f32; 3],
    pub value_d: [f32; 4],
}

impl MaterialProperty {
    /// 返回常见 `ks*` 属性使用的标量值。
    pub fn value(&self) -> f32 {
        self.value_a
    }
}

#[derive(Debug, Clone)]
pub struct TextureSample {
    /// 槽名，例如 `txDiffuse`。
    pub slot_name: String,
    /// 与槽名一起存储的数字槽索引。
    pub slot: u32,
    /// 该槽引用的贴图名。
    pub texture: String,
}
