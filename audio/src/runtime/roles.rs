//! AC bank 采样的统一解析器。
//!
//! 这里的规则只依赖 AC 通用命名、采样时长和 `ext/int` 视角，不包含任何具体
//! 车型分支。音色永远是 mod 自己的采样；本模块只回答“这个采样是什么角色”。
#![allow(dead_code)]

/// 循环层至少这么长；更短的按一次性音效处理。
pub(super) const LOOP_MIN_MS: u32 = 500;
/// 换挡一次性音效的时长上限。`upsh*` / `dwsh*` 这类几十秒长循环会被排除。
pub(super) const ONE_SHOT_MAX_MS: u32 = 1000;
/// 回火采样可以稍长（部分 mod 的 backfire 接近 2 s），但不能是长循环。
pub(super) const BACKFIRE_MAX_MS: u32 = 3000;

/// 采样属于车外 / 车内 / 两者皆可。AC 的 FSB 命名里 `_ext_` / `_int_` 是稳定的
/// 区分方式；没有前缀的（例如 Porsche 的 `idle` / `onthrottle`）两边共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SampleView {
    Both,
    Ext,
    Int,
}

impl SampleView {
    pub(super) fn matches(self, in_cockpit: bool) -> bool {
        match self {
            Self::Both => true,
            Self::Ext => !in_cockpit,
            Self::Int => in_cockpit,
        }
    }
}

/// 采样录制位置。`back` / `rear` 是车尾（排气侧），`front` 是车头。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SamplePosition {
    Rear,
    Front,
    Unspecified,
}

/// 统一解析器给每个 FSB 采样分配的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SampleRole {
    Idle,
    EngineOn,
    EngineOff,
    Wind,
    Tyre,
    Skid,
    Turbo,
    GearUp,
    GearDown,
    Backfire,
    Traction,
    Transmission(i32),
    Ignored,
}

/// 名字里可能出现的角色关键字。先匹配到哪个 token，就按哪个角色的时长规则
/// 处理；`Ignored` 用来表达“这类词出现时不要再走通用 on/off”。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RoleToken {
    Idle,
    EngineOn,
    EngineOff,
    Wind,
    Tyre,
    Skid,
    Turbo,
    Traction,
    Transmission,
    GearUp,
    GearDown,
    Backfire,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Contains,
    Prefix,
}

/// 规则表按顺序匹配，等价于一个 switch：
///
/// - 强角色（transmission/traction/backfire/gearup…）放在通用 on/off 前面，
///   避免 `"...sion_4th"` 里的 `on_` 把变速箱采样误判成油门声；
/// - `draft` / `button` / `control` 先落到 `Ignored`，保留原先的排除语义。
const ROLE_RULES: &[(&str, MatchKind, RoleToken)] = &[
    ("draft", MatchKind::Contains, RoleToken::Ignored),
    ("gear_up", MatchKind::Contains, RoleToken::GearUp),
    ("gearup", MatchKind::Contains, RoleToken::GearUp),
    ("gear_down", MatchKind::Contains, RoleToken::GearDown),
    ("geardn", MatchKind::Contains, RoleToken::GearDown),
    ("downshift", MatchKind::Contains, RoleToken::GearDown),
    ("backfire", MatchKind::Contains, RoleToken::Backfire),
    ("transmission", MatchKind::Contains, RoleToken::Transmission),
    ("traction", MatchKind::Contains, RoleToken::Traction),
    ("button", MatchKind::Contains, RoleToken::Ignored),
    ("control", MatchKind::Contains, RoleToken::Ignored),
    ("gearbox", MatchKind::Contains, RoleToken::Ignored),
    ("idle", MatchKind::Contains, RoleToken::Idle),
    ("onthrottle", MatchKind::Contains, RoleToken::EngineOn),
    ("decel", MatchKind::Contains, RoleToken::EngineOff),
    ("wind", MatchKind::Contains, RoleToken::Wind),
    ("tyre_rolling", MatchKind::Contains, RoleToken::Tyre),
    ("tire_rolling", MatchKind::Contains, RoleToken::Tyre),
    ("skid", MatchKind::Contains, RoleToken::Skid),
    ("turbo", MatchKind::Contains, RoleToken::Turbo),
    ("_on_", MatchKind::Contains, RoleToken::EngineOn),
    ("off_", MatchKind::Contains, RoleToken::EngineOff),
    ("acc", MatchKind::Prefix, RoleToken::EngineOn),
    ("on", MatchKind::Prefix, RoleToken::EngineOn),
    ("off", MatchKind::Prefix, RoleToken::EngineOff),
];

fn role_token(name: &str) -> Option<RoleToken> {
    ROLE_RULES.iter().find_map(|(pattern, kind, token)| {
        let matched = match kind {
            MatchKind::Contains => name.contains(pattern),
            MatchKind::Prefix => name.starts_with(pattern),
        };
        matched.then_some(*token)
    })
}

/// 统一的采样分类入口。switch 先按名字取 token，再按时长过滤。
pub(super) fn classify(name: &str, duration_ms: u32) -> SampleRole {
    let name = name.to_ascii_lowercase();
    match role_token(&name) {
        Some(RoleToken::Idle) if duration_ms >= LOOP_MIN_MS => SampleRole::Idle,
        Some(RoleToken::EngineOn) if duration_ms >= LOOP_MIN_MS => SampleRole::EngineOn,
        Some(RoleToken::EngineOff) if duration_ms >= LOOP_MIN_MS => SampleRole::EngineOff,
        Some(RoleToken::Wind) if duration_ms >= LOOP_MIN_MS => SampleRole::Wind,
        Some(RoleToken::Tyre) if duration_ms >= LOOP_MIN_MS => SampleRole::Tyre,
        Some(RoleToken::Skid) if duration_ms >= LOOP_MIN_MS => SampleRole::Skid,
        Some(RoleToken::Turbo) if duration_ms >= LOOP_MIN_MS => SampleRole::Turbo,
        Some(RoleToken::Traction) if duration_ms >= LOOP_MIN_MS => SampleRole::Traction,
        Some(RoleToken::Transmission) if duration_ms >= LOOP_MIN_MS => {
            transmission_gear(&name).map_or(SampleRole::Ignored, SampleRole::Transmission)
        }
        Some(RoleToken::GearUp) if duration_ms <= ONE_SHOT_MAX_MS => SampleRole::GearUp,
        Some(RoleToken::GearDown) if duration_ms <= ONE_SHOT_MAX_MS => SampleRole::GearDown,
        Some(RoleToken::Backfire) if duration_ms <= BACKFIRE_MAX_MS => SampleRole::Backfire,
        _ => SampleRole::Ignored,
    }
}

pub(super) fn sample_view(name: &str) -> SampleView {
    let name = name.to_ascii_lowercase();
    let ext = name.contains("ext_") || name.ends_with("ext");
    let int = name.contains("int_") || name.ends_with("int");
    match (ext, int) {
        (true, false) => SampleView::Ext,
        (false, true) => SampleView::Int,
        _ => SampleView::Both,
    }
}

pub(super) fn sample_position(name: &str) -> SamplePosition {
    let name = name.to_ascii_lowercase();
    match (
        name.contains("back") || name.contains("rear"),
        name.contains("front"),
    ) {
        (true, _) => SamplePosition::Rear,
        (false, true) => SamplePosition::Front,
        (false, false) => SamplePosition::Unspecified,
    }
}

/// 从 `056_ext_on_back_12halfK` / `056_ext_on_front_14Kish` / `056_ext_off_front_13k`
/// 这类名字里解析录制转速。`056_` 这种编号后面跟 `_`，不会被误判。
pub(super) fn reference_rpm(name: &str) -> Option<f32> {
    let name = name.to_ascii_lowercase();
    let bytes = name.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let Ok(value) = name[start..index].parse::<f32>() else {
            continue;
        };
        let rest = &name[index..];
        if rest.starts_with("halfk") {
            return Some(value * 1000.0 + 500.0);
        }
        if rest.starts_with("kish") || rest.starts_with('k') {
            return Some(value * 1000.0);
        }
    }
    None
}

/// 没有数字转速时按 `low` / `mid` / `high` 粗分。
pub(super) fn band_rpm(name: &str, idle_ref: f32) -> Option<f32> {
    let name = name.to_ascii_lowercase();
    match name.as_str() {
        n if n.contains("mid") => Some(idle_ref * 2.2),
        n if n.contains("low") => Some(idle_ref * 1.4),
        n if n.contains("high") || n.contains("_hi") => Some(idle_ref * 3.0),
        _ => None,
    }
}

/// `transmission_4th` / `transmission_5th` → 挡位。
pub(super) fn transmission_gear(name: &str) -> Option<i32> {
    let name = name.to_ascii_lowercase();
    if !name.contains("transmission") {
        return None;
    }
    [
        ("1st", 1),
        ("2nd", 2),
        ("3rd", 3),
        ("4th", 4),
        ("5th", 5),
        ("6th", 6),
        ("7th", 7),
        ("8th", 8),
    ]
    .into_iter()
    .find_map(|(suffix, gear)| name.contains(suffix).then_some(gear))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_f1_engine_and_one_shots() {
        assert_eq!(classify("056_ext_idle", 4000), SampleRole::Idle);
        assert_eq!(
            classify("056_ext_on_back_12halfK", 3000),
            SampleRole::EngineOn
        );
        assert_eq!(
            classify("056_ext_off_front_13k", 3000),
            SampleRole::EngineOff
        );
        assert_eq!(
            classify("wire_traction_control", 9000),
            SampleRole::Traction
        );
        assert_eq!(classify("tyre_rolling", 10000), SampleRole::Tyre);
    }

    #[test]
    fn long_upsh_is_not_a_gear_one_shot() {
        assert_eq!(classify("upsh1", 53_955), SampleRole::Ignored);
        assert_eq!(classify("dwsh_rear", 44_780), SampleRole::Ignored);
        assert_eq!(classify("gearupEXT", 193), SampleRole::GearUp);
        assert_eq!(classify("geardnEXT", 193), SampleRole::GearDown);
    }

    #[test]
    fn classifies_porsche_loops_and_transmission() {
        assert_eq!(classify("onthrottle", 76_000), SampleRole::EngineOn);
        assert_eq!(classify("decel_rear", 2191), SampleRole::EngineOff);
        assert_eq!(
            classify("transmission_4th", 47_845),
            SampleRole::Transmission(4)
        );
        assert_eq!(
            classify("transmission_6th", 89_767),
            SampleRole::Transmission(6)
        );
    }

    #[test]
    fn parses_recorded_rpm_and_position() {
        assert_eq!(reference_rpm("056_ext_on_back_6halfK"), Some(6500.0));
        assert_eq!(reference_rpm("056_ext_on_back_12halfK"), Some(12500.0));
        assert_eq!(reference_rpm("056_ext_on_front_14Kish"), Some(14000.0));
        assert_eq!(sample_position("acc_rear_far3"), SamplePosition::Rear);
        assert_eq!(sample_position("acc_front"), SamplePosition::Front);
    }

    #[test]
    fn parses_view_suffixes() {
        assert_eq!(sample_view("056_ext_idle"), SampleView::Ext);
        assert_eq!(sample_view("f2008_int_on_high"), SampleView::Int);
        assert_eq!(sample_view("gearup"), SampleView::Both);
        assert_eq!(sample_view("geardnEXT"), SampleView::Ext);
    }
}
