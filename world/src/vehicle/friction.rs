//! 每轮动态摩擦圆：纵向/横向共享一个抓地预算。
//!
//! 这是车辆抓地的最后一个乘数来源：轮胎自己的曲线形状（Jolt/`tyre_model`）
//! 决定峰值在哪、掉得多快，这里决定**两个方向怎么分一个预算**、以及完全打滑
//! 时还剩多少抓地。

use vettura_track::SurfaceKind;
use vettura_track::SurfaceProperties;

/// 全滑移时一个方向还能保住的抓地份额下限。
///
/// 摩擦椭圆不会把任何一边清空：完全打滑的轮子仍然保有一部分抓地，
/// 否则在 bank 上起步会瞬间失去全部侧向力而打转。
pub(crate) const FRICTION_CIRCLE_FLOOR_MIN: f32 = 0.5;

/// 全滑移时一个方向还能保住的抓地份额上限。
pub(crate) const FRICTION_CIRCLE_FLOOR_MAX: f32 = 0.7;

/// 一个轮子的动态摩擦圆。
///
/// 返回 `(纵向抓地系数, 横向抓地系数)`，两者**共享同一个预算**：横向用掉
/// `lateral_demand` 时纵向只能拿剩下的，反之亦然。这就是「刹车入弯会推头」
/// 的物理来源。
///
/// 三处动态输入：
/// * `floor` —— 全滑移时还剩多少，随路面与轮胎衰减特性变（见
///   [`slip_floor`]），不是常数；
/// * `*_demand` —— **这个轮子本步自己的**滑移率/滑移角归一化值（0..1），
///   所以四个轮子各自拥有一个独立的圆；
/// * `relax_*` —— 松弛长度：力还没建立出来的那一部分，直接乘在预算上。
///
/// 关键点是**需求平面的合力归一**：`√(demand_long² + demand_lat²) ≤ 1`，
/// 一个方向要得越多另一个要得越少。归一放在需求上而不是预算上——预算是
/// 「势」，两个方向都不打滑时它们是各自的最大值，拿势算合力会把纯纵向/纯
/// 横向的情况也一起砍掉。
pub(crate) fn combined_slip_circle(
    longitudinal_demand: f32,
    lateral_demand: f32,
    floor: f32,
    relaxation_long: f32,
    relaxation_lateral: f32,
) -> (f32, f32) {
    let longitudinal = longitudinal_demand.clamp(0.0, 4.0);
    let lateral = lateral_demand.clamp(0.0, 4.0);
    let floor = floor.clamp(FRICTION_CIRCLE_FLOOR_MIN, FRICTION_CIRCLE_FLOOR_MAX);

    // 椭圆：一个方向用掉 demand²，另一个方向只剩 √(1 − d²)。
    //
    // 这里**不做**需求平面的合力归一：弯中一旦打滑，横向抓地本就已经不够，
    // 再把 `lateral` 需求一起压小只会让车失速后无法恢复（实测整圈卡死）。
    // 椭圆本身就是摩擦圆的形状，量纲也自洽。
    let budget_long = floor + (1.0 - floor) * (1.0 - lateral * lateral).max(0.0).sqrt();
    let budget_lateral =
        floor + (1.0 - floor) * (1.0 - longitudinal * longitudinal).max(0.0).sqrt();
    (
        (budget_long * relaxation_long).clamp(0.0, 1.0),
        (budget_lateral * relaxation_lateral).clamp(0.0, 1.0),
    )
}

/// 全滑移时的保底抓地份额：由路面与轮胎自己的衰减特性决定。
///
/// * 轮胎的 `falloff_level`（AC 的 `FALLOFF_LEVEL`，3 倍峰值滑移时还剩多少）
///   越接近 1，说明这条曲线在打滑区掉得越慢、滑起来还有抓地；
/// * 路面抓地越低（冰/雪/湿草），打滑时掉得越干净，地板越低；
/// * 磨损不直接进这里——它已经通过 `grip_multiplier` 缩放了整条曲线。
pub(crate) fn slip_floor(tyres: &vettura_physics::TyreSpec, surface: &SurfaceProperties) -> f32 {
    let falloff = tyres.falloff_level.clamp(0.0, 1.0);
    // 路面系数用干燥抓地相对「标准干燥沥青」的比例。
    let surface_share =
        (surface.dry_grip / SurfaceKind::Asphalt.properties().dry_grip).clamp(0.05, 1.0);
    // 0.35..0.7 之间映射：曲线越平 → 地板越高；路面越烂 → 越低。
    let raw = FRICTION_CIRCLE_FLOOR_MIN
        + (FRICTION_CIRCLE_FLOOR_MAX - FRICTION_CIRCLE_FLOOR_MIN) * falloff * surface_share;
    raw.clamp(FRICTION_CIRCLE_FLOOR_MIN, FRICTION_CIRCLE_FLOOR_MAX)
}

/// 冷却液向节温器设定点松弛的速率，1/s（时间常数约 4 s）。真车的节温器就
/// 是这样：任何负荷下都把工作温度守住，而不是让平衡点随散热量漂移。
#[cfg(test)]
mod circle_tests {
    use super::*;
    use vettura_physics::TyreSpec;

    const FLOOR: f32 = 0.5;
    const RELAX: f32 = 1.0;

    /// 合力 = √(long² + lat²)，必须 ≤ 1.0（单方向峰值）。
    fn resultant(long: f32, lat: f32) -> f32 {
        (long * long + lat * lat).sqrt()
    }

    #[test]
    fn single_direction_keeps_the_whole_budget() {
        // 只有纵向需求：横向一分不让，纵向全给。
        let (long, lat) = combined_slip_circle(0.0, 0.0, FLOOR, RELAX, RELAX);
        assert!((long - 1.0).abs() < 1.0e-6, "{long}");
        assert!((lat - 1.0).abs() < 1.0e-6, "{lat}");
        let (long, _lat) = combined_slip_circle(1.0, 0.0, FLOOR, RELAX, RELAX);
        assert!((long - 1.0).abs() < 1.0e-6, "{long}");
    }

    #[test]
    fn demands_fit_inside_the_circle_and_the_ellipse_is_monotone() {
        // 扫整个需求平面，断言两条真正成立的不变量：
        //   1) 固定一侧需求，另一侧需求越大，本侧预算越低（椭圆在让路）；
        //   2) 需求合力超圆时两边一起被压（一个方向要得越多，另一个越少）。
        for step_l in 0..=20 {
            for step_r in 0..=20 {
                let lon = step_l as f32 / 20.0;
                let lat = step_r as f32 / 20.0;
                let (long, latf) = combined_slip_circle(lon, lat, FLOOR, RELAX, RELAX);
                // 1) 纵向预算随横向需求单调不增。
                let lon_with_more_lat =
                    combined_slip_circle(lon, (lat + 0.05).min(1.0), FLOOR, RELAX, RELAX).0;
                assert!(
                    long >= lon_with_more_lat - 1.0e-6,
                    "横向需求增大反而给了更多纵向：lon={lon:.2} lat={lat:.2} {long} vs {lon_with_more_lat}"
                );
                // 1') 横向预算随纵向需求单调不增。
                let lat_with_more_lon =
                    combined_slip_circle((lon + 0.05).min(1.0), lat, FLOOR, RELAX, RELAX).1;
                assert!(
                    latf >= lat_with_more_lon - 1.0e-6,
                    "纵向需求增大反而给了更多横向：lon={lon:.2} lat={lat:.2} {latf} vs {lat_with_more_lon}"
                );
                // 2) 需求都很大时，两边都远小于单方向时的满预算。
                if lon > 0.8 && lat > 0.8 {
                    assert!(
                        long < 0.95 && latf < 0.95,
                        "双重满需求没有互相挤压：{long} / {latf}"
                    );
                }
                // 边界：永远不会给出负值或超过 1。
                assert!((0.0..=1.0).contains(&long) && (0.0..=1.0).contains(&latf));
            }
        }
    }

    #[test]
    fn one_direction_steals_from_the_other() {
        // 横向满需求时纵向被压到地板附近——这就是刹车入弯推头。
        let (long, _lat) = combined_slip_circle(0.9, 0.9, FLOOR, RELAX, RELAX);
        let (lon_alone, _) = combined_slip_circle(0.9, 0.0, FLOOR, RELAX, RELAX);
        assert!(
            long < lon_alone * 0.95,
            "横向需求没有吃掉纵向预算：{long} vs {lon_alone}"
        );
    }

    #[test]
    fn the_floor_is_dynamic_with_surface_and_tyre() {
        let asphalt = SurfaceKind::Asphalt.properties();
        let ice = SurfaceKind::Ice.properties();

        // 曲线更平（falloff 高）→ 打滑时剩下的更多。
        let flat = TyreSpec {
            falloff_level: 0.9,
            ..TyreSpec::default()
        };
        let steep = TyreSpec {
            falloff_level: 0.2,
            ..TyreSpec::default()
        };
        let flat_floor = slip_floor(&flat, &asphalt);
        let steep_floor = slip_floor(&steep, &asphalt);
        assert!(flat_floor > steep_floor, "{flat_floor} vs {steep_floor}");

        // 同一条胎，冰面上打滑掉得更干净。
        let ice_floor = slip_floor(&flat, &ice);
        assert!(ice_floor < flat_floor, "{ice_floor} vs {flat_floor}");

        // 边界：永远不会低于下限（否则 bank 起步会瞬间失去全部侧向力）。
        assert!(ice_floor >= FRICTION_CIRCLE_FLOOR_MIN);
        assert!(flat_floor <= FRICTION_CIRCLE_FLOOR_MAX);

        // 地板越高 → 打滑时曲线越平 → 同样 demand 下另一边被让得越少。
        let (lon_asp, _) = combined_slip_circle(0.0, 0.9, flat_floor, RELAX, RELAX);
        let (lon_ice, _) = combined_slip_circle(0.0, 0.9, ice_floor, RELAX, RELAX);
        assert!(
            lon_asp < 1.0 && lon_ice < 1.0,
            "另一侧满 demand 时必须让出预算：{lon_asp} / {lon_ice}"
        );
        assert!(
            lon_asp >= lon_ice - 1.0e-6,
            "高地板应该至少不更低：{lon_asp} vs {lon_ice}"
        );
        let _ = resultant(lon_asp, lon_ice);
    }

    #[test]
    fn relaxation_scales_the_budget_down() {
        let (full_long, _) = combined_slip_circle(0.5, 0.5, FLOOR, 1.0, 1.0);
        let (relaxed_long, _) = combined_slip_circle(0.5, 0.5, FLOOR, 0.5, 1.0);
        assert!(
            relaxed_long < full_long * 0.6,
            "{relaxed_long} vs {full_long}"
        );
    }

    #[test]
    fn absurd_demand_does_not_blow_up() {
        for lon in [0.0, 1.0, 4.0, 40.0, f32::NAN, f32::INFINITY] {
            for lat in [0.0, 1.0, 4.0, 40.0, f32::NAN, f32::INFINITY] {
                let (long, latf) = combined_slip_circle(lon, lat, FLOOR, RELAX, RELAX);
                assert!(long.is_finite() && (0.0..=1.0).contains(&long), "{long}");
                assert!(latf.is_finite() && (0.0..=1.0).contains(&latf), "{latf}");
                let _ = resultant(long, latf);
            }
        }
    }
}
