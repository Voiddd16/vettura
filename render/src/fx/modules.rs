//! 内置 FX 模块。
//!
//! v1 的模块只维护状态与预算（不直接画东西）：它们是渲染桥的“控制面”。
//! 真正的 Godot pass / 粒子 / 灯光在 `gdext/src/fx` 里按这些状态实现；
//! 这样可以先验证启用/依赖/预算/报告，再逐个接入渲染。

use std::collections::BTreeMap;

use super::config::{FxConfig, FxValue, bool_at, f32_at, str_at};
use super::context::{FxContext, FxCost, FxQuality};
use super::definition::{FxCapability, FxDefinition, FxError, FxInstance, FxMeta, FxScope};

fn value_f32(value: f32) -> FxValue {
    FxValue::Float(value as f64)
}

fn value_bool(value: bool) -> FxValue {
    FxValue::Bool(value)
}

fn config(entries: &[(&str, FxValue)]) -> FxConfig {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

struct GenericDefinition {
    meta: FxMeta,
    defaults: FxConfig,
}

impl FxDefinition for GenericDefinition {
    fn meta(&self) -> FxMeta {
        self.meta.clone()
    }

    fn default_config(&self) -> FxConfig {
        self.defaults.clone()
    }

    fn create(&self, config: &FxConfig) -> Result<Box<dyn FxInstance>, FxError> {
        let mut merged = self.defaults.clone();
        merged.extend(config.clone());
        Ok(Box::new(ConfigInstance {
            meta: self.meta.clone(),
            config: merged,
            state: BTreeMap::new(),
        }))
    }
}

struct ConfigInstance {
    meta: FxMeta,
    config: FxConfig,
    state: BTreeMap<String, FxValue>,
}

impl FxInstance for ConfigInstance {
    fn update(&mut self, ctx: &mut FxContext) {
        self.state.insert(
            "quality".to_owned(),
            FxValue::String(ctx.quality.label().to_owned()),
        );

        match self.meta.id.as_str() {
            "vettura.fx.weather" => {
                let scale = f32_at(&self.config, "time_scale", 1.0).max(0.0);
                let start = f32_at(&self.config, "time_of_day", ctx.weather.time_of_day);
                let tod = (start + ctx.dt * scale / 3600.0).rem_euclid(24.0);
                let rain = f32_at(&self.config, "rain", ctx.weather.rain).clamp(0.0, 1.0);
                let wetness = f32_at(&self.config, "wetness", ctx.weather.wetness).clamp(0.0, 1.0);
                let wind = f32_at(&self.config, "wind_speed", ctx.weather.wind_speed).max(0.0);
                let direction = f32_at(
                    &self.config,
                    "wind_direction_deg",
                    ctx.weather.wind_direction_deg,
                );
                let fog = f32_at(&self.config, "fog", ctx.weather.fog).clamp(0.0, 1.0);

                ctx.weather.time_of_day = tod;
                ctx.weather.rain = rain;
                ctx.weather.wetness = wetness;
                ctx.weather.wind_speed = wind;
                ctx.weather.wind_direction_deg = direction;
                ctx.weather.fog = fog;

                self.state.insert("time_of_day".to_owned(), value_f32(tod));
                self.state.insert("rain".to_owned(), value_f32(rain));
                self.state.insert("wetness".to_owned(), value_f32(wetness));
                self.state.insert("wind_speed".to_owned(), value_f32(wind));
                self.state.insert("fog".to_owned(), value_f32(fog));
            }
            "vettura.fx.rain" => {
                let target = f32_at(&self.config, "intensity", 1.0).clamp(0.0, 1.0);
                let intensity = (target * ctx.weather.rain).clamp(0.0, 1.0);
                let windshield = bool_at(&self.config, "windshield", true);
                let spray = bool_at(&self.config, "spray", true) && ctx.weather.wetness > 0.05;
                let puddles = bool_at(&self.config, "puddles", true) && ctx.weather.wetness > 0.2;

                self.state
                    .insert("intensity".to_owned(), value_f32(intensity));
                self.state
                    .insert("windshield".to_owned(), value_bool(windshield));
                self.state.insert("spray".to_owned(), value_bool(spray));
                self.state.insert("puddles".to_owned(), value_bool(puddles));
            }
            "vettura.fx.grass" => {
                let density = f32_at(&self.config, "density", 1.0).clamp(0.0, 4.0);
                let distance = f32_at(&self.config, "draw_distance", 120.0).max(0.0);
                self.state.insert("density".to_owned(), value_f32(density));
                self.state
                    .insert("draw_distance".to_owned(), value_f32(distance));
                self.state.insert(
                    "wind".to_owned(),
                    value_bool(bool_at(&self.config, "wind", true)),
                );
            }
            "vettura.fx.lighting" => {
                let max_lights = f32_at(&self.config, "max_lights", 32.0).max(0.0) as u32;
                let car_lights = f32_at(&self.config, "car_lights", 4.0).max(0.0) as u32;
                let track_lights = f32_at(&self.config, "track_lights", 0.0).max(0.0) as u32;
                let requested = car_lights.saturating_mul(ctx.cars.len() as u32) + track_lights;
                let active = requested.min(max_lights);
                self.state
                    .insert("active_lights".to_owned(), FxValue::Integer(active as i64));
                self.state.insert(
                    "requested_lights".to_owned(),
                    FxValue::Integer(requested as i64),
                );
            }
            "vettura.fx.particles" => {
                let max_particles = f32_at(&self.config, "max_particles", 8_000.0).max(0.0) as u32;
                let wet_boost = if ctx.weather.wetness > 0.2 { 1.5 } else { 1.0 };
                let active = (max_particles as f32 * wet_boost) as u32;
                self.state.insert(
                    "particle_budget".to_owned(),
                    FxValue::Integer(active as i64),
                );
                self.state.insert(
                    "smoke".to_owned(),
                    value_bool(bool_at(&self.config, "smoke", true)),
                );
                self.state.insert(
                    "spray".to_owned(),
                    value_bool(bool_at(&self.config, "spray", true)),
                );
            }
            "vettura.fx.tyres" => {
                let threshold = f32_at(&self.config, "slip_threshold", 0.15).max(0.0);
                let slip = ctx
                    .cars
                    .iter()
                    .map(|car| car.tyre_slip)
                    .fold(0.0_f32, f32::max);
                self.state.insert("slip".to_owned(), value_f32(slip));
                self.state
                    .insert("active".to_owned(), value_bool(slip > threshold));
                self.state.insert(
                    "tread_texture".to_owned(),
                    FxValue::String(str_at(&self.config, "tread_texture", "").to_owned()),
                );
            }
            "vettura.fx.extra" | "vettura.fx.generic" => {
                self.state.insert(
                    "enabled".to_owned(),
                    value_bool(bool_at(&self.config, "enabled", true)),
                );
            }
            "vettura.fx.brake_disc" => {
                let threshold = f32_at(&self.config, "glow_threshold", 450.0).max(0.0);
                let max_temp = ctx
                    .cars
                    .iter()
                    .map(|car| car.brake_temp_c)
                    .fold(0.0_f32, f32::max);
                let glow = ((max_temp - threshold) / 350.0).clamp(0.0, 1.0);
                self.state
                    .insert("max_temp_c".to_owned(), value_f32(max_temp));
                self.state.insert("glow".to_owned(), value_f32(glow));
                self.state
                    .insert("threshold".to_owned(), value_f32(threshold));
            }
            id if id.starts_with("vettura.fx.extra.") => {
                let enabled = bool_at(&self.config, "enabled", true);
                self.state.insert("enabled".to_owned(), value_bool(enabled));
            }
            _ => {}
        }
    }

    fn snapshot(&self) -> BTreeMap<String, FxValue> {
        self.state.clone()
    }
}

fn definition(
    id: &str,
    scope: FxScope,
    capabilities: &[FxCapability],
    min_quality: FxQuality,
    cost: FxCost,
    requires: &[&str],
    defaults: FxConfig,
) -> Box<dyn FxDefinition> {
    let mut meta = FxMeta::new(id, scope, capabilities, min_quality).cost(cost);
    for requirement in requires {
        meta = meta.require(*requirement);
    }
    Box::new(GenericDefinition { meta, defaults })
}

/// 所有内置模块的注册表。顺序即默认更新顺序；依赖会在 `FxManager` 里再做拓扑排序。
pub fn builtin_definitions() -> Vec<Box<dyn FxDefinition>> {
    vec![
        definition(
            "vettura.fx.weather",
            FxScope::Scene,
            &[FxCapability::Environment],
            FxQuality::Low,
            FxCost::default(),
            &[],
            config(&[
                ("time_of_day", value_f32(10.0)),
                ("time_scale", value_f32(1.0)),
                ("rain", value_f32(0.0)),
                ("wetness", value_f32(0.0)),
                ("wind_speed", value_f32(0.0)),
                ("wind_direction_deg", value_f32(0.0)),
                ("fog", value_f32(0.0)),
            ]),
        ),
        definition(
            "vettura.fx.particles",
            FxScope::Scene,
            &[FxCapability::Particle],
            FxQuality::Low,
            FxCost {
                particles: 8_000,
                ..FxCost::default()
            },
            &[],
            config(&[
                ("max_particles", FxValue::Integer(8_000)),
                ("smoke", value_bool(true)),
                ("dust", value_bool(true)),
                ("spray", value_bool(true)),
                ("sparks", value_bool(true)),
            ]),
        ),
        definition(
            "vettura.fx.rain",
            FxScope::Scene,
            &[FxCapability::Render, FxCapability::Particle],
            FxQuality::Low,
            FxCost {
                draw_calls: 12,
                particles: 4_000,
                ..FxCost::default()
            },
            &["vettura.fx.weather", "vettura.fx.particles"],
            config(&[
                ("intensity", value_f32(1.0)),
                ("windshield", value_bool(true)),
                ("spray", value_bool(true)),
                ("puddles", value_bool(true)),
            ]),
        ),
        definition(
            "vettura.fx.grass",
            FxScope::Track,
            &[FxCapability::Render, FxCapability::Material],
            FxQuality::Medium,
            FxCost {
                draw_calls: 64,
                ..FxCost::default()
            },
            &[],
            config(&[
                ("density", value_f32(1.0)),
                ("draw_distance", value_f32(120.0)),
                ("wind", value_bool(true)),
            ]),
        ),
        definition(
            "vettura.fx.lighting",
            FxScope::Scene,
            &[FxCapability::Light],
            FxQuality::Low,
            FxCost {
                lights: 32,
                ..FxCost::default()
            },
            &[],
            config(&[
                ("max_lights", FxValue::Integer(32)),
                ("car_lights", FxValue::Integer(4)),
                ("track_lights", FxValue::Integer(0)),
                ("shadows", value_bool(true)),
            ]),
        ),
        definition(
            "vettura.fx.tyres",
            FxScope::Car,
            &[FxCapability::Render, FxCapability::Material],
            FxQuality::Low,
            FxCost {
                draw_calls: 8,
                ..FxCost::default()
            },
            &[],
            config(&[
                ("slip_threshold", value_f32(0.15)),
                ("tread_texture", FxValue::String(String::new())),
                ("side_texture", FxValue::String(String::new())),
                ("blur_texture", FxValue::String(String::new())),
            ]),
        ),
        definition(
            "vettura.fx.extra",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::Medium,
            FxCost {
                post_passes: 0,
                milliseconds: 0.0,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.brake_disc",
            FxScope::Car,
            &[FxCapability::Material, FxCapability::Render],
            FxQuality::Low,
            FxCost {
                draw_calls: 8,
                ..FxCost::default()
            },
            &[],
            config(&[
                ("glow_threshold", value_f32(450.0)),
                ("max_emissive", value_f32(1.0)),
            ]),
        ),
        definition(
            "vettura.fx.generic",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::Low,
            FxCost::default(),
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.bloom",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::Medium,
            FxCost {
                post_passes: 1,
                milliseconds: 0.4,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.ssao",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::High,
            FxCost {
                post_passes: 1,
                milliseconds: 1.2,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.ssr",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::High,
            FxCost {
                post_passes: 1,
                milliseconds: 1.6,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.dof",
            FxScope::Camera,
            &[FxCapability::Render],
            FxQuality::High,
            FxCost {
                post_passes: 1,
                milliseconds: 1.0,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.motion_blur",
            FxScope::Camera,
            &[FxCapability::Render],
            FxQuality::High,
            FxCost {
                post_passes: 1,
                milliseconds: 0.8,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
        definition(
            "vettura.fx.extra.sharpen",
            FxScope::Scene,
            &[FxCapability::Render],
            FxQuality::Medium,
            FxCost {
                post_passes: 1,
                milliseconds: 0.2,
                ..FxCost::default()
            },
            &[],
            config(&[("enabled", value_bool(true))]),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_definitions_have_unique_ids() {
        let definitions = builtin_definitions();
        let mut ids = std::collections::BTreeSet::new();
        for definition in &definitions {
            assert!(ids.insert(definition.meta().id));
        }
        assert!(ids.contains("vettura.fx.weather"));
        assert!(ids.contains("vettura.fx.rain"));
        assert!(ids.contains("vettura.fx.grass"));
        assert!(ids.contains("vettura.fx.lighting"));
        assert!(ids.contains("vettura.fx.particles"));
        assert!(ids.contains("vettura.fx.brake_disc"));
        assert!(ids.contains("vettura.fx.tyres"));
        assert!(ids.contains("vettura.fx.extra"));
        assert!(ids.contains("vettura.fx.generic"));
        assert!(ids.contains("vettura.fx.extra.ssao"));
    }
}
