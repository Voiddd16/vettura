//! Weather and its effect on grip, rolling resistance and aero.
//!
//! The model is deliberately physical-but-cheap:
//!
//! - track **wetness** interpolates each surface between its dry and wet grip;
//! - **standing water** reduces grip further as speed rises (aquaplaning);
//! - **track temperature** moves grip along a simple tyre working-window curve;
//! - **wind** enters the aerodynamic force as relative air velocity;
//! - **air density** scales drag and downforce.

use glam::Vec3;

use crate::surface::SurfaceProperties;

/// Atmospheric and track conditions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weather {
    /// Track wetness, `0..=1`. `0` is fully dry, `1` is soaked.
    pub wetness: f32,
    /// Rain intensity, `0..=1`. Cosmetic for now; `wetness` is what physics uses.
    pub rain_intensity: f32,
    /// Air temperature, °C.
    pub air_temperature_c: f32,
    /// Track-surface temperature, °C. The world evolves this toward the sun
    /// equilibrium while it runs; use [`Self::with_sun`] with `0 W/m²` when a
    /// fixed condition is wanted.
    pub track_temperature_c: f32,
    /// Sun intensity, W/m². `0` is shade or a night session, and the road then
    /// sits at the air temperature.
    pub sun_w_m2: f32,
    /// Time constant of the road's heat capacity, seconds.
    pub track_time_constant_s: f32,
    /// Wind velocity in the physics frame, m/s. Use `+X` for a wind blowing to
    /// the right.
    pub wind: Vec3,
    /// Global multiplier on tyre grip, for assists, rule sets or debugging.
    pub grip_scale: f32,
    /// Air density, kg/m³.
    pub air_density_kg_m3: f32,
    /// Fog density hint, `0..=1`. Not used by the physics solver.
    pub fog_density: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Self::dry()
    }
}

impl Weather {
    pub fn dry() -> Self {
        Self {
            wetness: 0.0,
            rain_intensity: 0.0,
            // 32 °C for both air and track: the default world is warm, which
            // matches `BrakeSpec::ambient_c` so a car that never gets its own
            // weather still starts where the brake/tyre model expects.
            air_temperature_c: 32.0,
            track_temperature_c: 32.0,
            // Bright but not tropical: a sunny afternoon. The road is slow to
            // follow it, which is the whole point of the time constant.
            sun_w_m2: 800.0,
            track_time_constant_s: 1_800.0,
            wind: Vec3::ZERO,
            grip_scale: 1.0,
            air_density_kg_m3: 1.225,
            fog_density: 0.0,
        }
    }

    /// A wet session. `rain` in `0..=1` controls both rain intensity and the
    /// resulting track wetness (a soaked track after sustained rain).
    pub fn wet(rain: f32) -> Self {
        let rain = finite_clamp(rain, 0.0, 1.0, 0.0);
        Self {
            wetness: 0.55 + 0.45 * rain,
            rain_intensity: rain,
            air_temperature_c: 16.0,
            track_temperature_c: 18.0,
            grip_scale: 1.0,
            ..Self::dry()
        }
    }

    pub fn with_wetness(mut self, wetness: f32) -> Self {
        self.wetness = finite_clamp(wetness, 0.0, 1.0, 0.0);
        self
    }

    pub fn with_rain(mut self, rain: f32) -> Self {
        self.rain_intensity = finite_clamp(rain, 0.0, 1.0, 0.0);
        self
    }

    pub fn with_temperatures(mut self, air_c: f32, track_c: f32) -> Self {
        self.air_temperature_c = if air_c.is_finite() { air_c } else { 22.0 };
        self.track_temperature_c = if track_c.is_finite() { track_c } else { 32.0 };
        self
    }

    /// Sun intensity and the road's heat response, W/m² and seconds.
    pub fn with_sun(mut self, sun_w_m2: f32, time_constant_s: f32) -> Self {
        self.sun_w_m2 = if sun_w_m2.is_finite() {
            sun_w_m2.clamp(0.0, 1_400.0)
        } else {
            0.0
        };
        self.track_time_constant_s = if time_constant_s.is_finite() {
            time_constant_s.clamp(1.0, 3_600.0)
        } else {
            1_800.0
        };
        self
    }

    /// Temperature a sunny road settles at, °C.
    ///
    /// A wet road tracks the air almost exactly; a dry one absorbs the sun and
    /// sits well above it, which is why the racing line is sticky on a hot
    /// afternoon. Negative sun (never in practice) is ignored.
    pub fn sun_equilibrium_temperature_c(&self) -> f32 {
        let sun = if self.sun_w_m2.is_finite() {
            self.sun_w_m2.clamp(0.0, 1_400.0)
        } else {
            0.0
        };
        // ~20 deg of excess at full sun on dry asphalt.
        let dry = (1.0 - self.wetness.clamp(0.0, 1.0)).max(0.0);
        self.air_temperature_c + sun * 0.025 * dry
    }

    /// Temperature a component sitting in the open should relax toward, °C.
    ///
    /// Brakes and the engine exchange with the *air*, while the tyres exchange
    /// with the *road*, which can be far colder on a winter morning. Both can
    /// be below zero; nothing here clamps at the freezing point, because a
    /// frozen tyre is a physical state and not an error.
    pub fn ambient_c(&self, on_the_road: bool) -> f32 {
        if on_the_road {
            self.track_temperature_c
        } else {
            self.air_temperature_c
        }
    }

    pub fn with_wind(mut self, wind: Vec3) -> Self {
        self.wind = if wind.is_finite() { wind } else { Vec3::ZERO };
        self
    }

    pub fn with_grip_scale(mut self, grip_scale: f32) -> Self {
        self.grip_scale = finite_clamp(grip_scale, 0.0, 4.0, 1.0);
        self
    }

    pub fn with_air_density(mut self, density_kg_m3: f32) -> Self {
        self.air_density_kg_m3 = finite_clamp(density_kg_m3, 0.0, 5.0, 1.225);
        self
    }

    /// Clamp every field to a sane range. Used when weather is assigned.
    pub fn clamped(mut self) -> Self {
        self.wetness = finite_clamp(self.wetness, 0.0, 1.0, 0.0);
        self.rain_intensity = finite_clamp(self.rain_intensity, 0.0, 1.0, 0.0);
        self.air_temperature_c = finite_clamp(self.air_temperature_c, -60.0, 70.0, 22.0);
        self.track_temperature_c = finite_clamp(self.track_temperature_c, -60.0, 120.0, 32.0);
        self.sun_w_m2 = finite_clamp(self.sun_w_m2, 0.0, 1_400.0, 800.0);
        self.track_time_constant_s =
            finite_clamp(self.track_time_constant_s, 1.0, 3_600.0, 1_800.0);
        self.grip_scale = finite_clamp(self.grip_scale, 0.0, 4.0, 1.0);
        self.air_density_kg_m3 = finite_clamp(self.air_density_kg_m3, 0.0, 5.0, 1.225);
        self.fog_density = finite_clamp(self.fog_density, 0.0, 1.0, 0.0);
        if !self.wind.is_finite() {
            self.wind = Vec3::ZERO;
        }
        self
    }

    /// Linear blend, used by weather transitions.
    pub fn lerp(from: &Weather, to: &Weather, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let m = |a: f32, b: f32| a + (b - a) * t;
        Self {
            wetness: m(from.wetness, to.wetness),
            rain_intensity: m(from.rain_intensity, to.rain_intensity),
            air_temperature_c: m(from.air_temperature_c, to.air_temperature_c),
            track_temperature_c: m(from.track_temperature_c, to.track_temperature_c),
            sun_w_m2: m(from.sun_w_m2, to.sun_w_m2),
            track_time_constant_s: m(from.track_time_constant_s, to.track_time_constant_s),
            wind: from.wind.lerp(to.wind, t),
            grip_scale: m(from.grip_scale, to.grip_scale),
            air_density_kg_m3: m(from.air_density_kg_m3, to.air_density_kg_m3),
            fog_density: m(from.fog_density, to.fog_density),
        }
    }

    /// Combined tyre-grip factor for one surface, speed and standing-water depth.
    pub fn grip(&self, surface: &SurfaceProperties, speed_mps: f32, water_depth_m: f32) -> f32 {
        self.grip_for_wheel(surface, speed_mps, water_depth_m, 0.0)
    }

    /// Grip for one wheel, including the water film it is carrying.
    ///
    /// The standing water under the tyre is only half the story: a tyre with a
    /// film on it has to displace that too, and the film is what actually
    /// lifts the contact patch off the road. `water_film_mm` is the thickness
    /// the wheel picked up from previous contact.
    pub fn grip_for_wheel(
        &self,
        surface: &SurfaceProperties,
        speed_mps: f32,
        water_depth_m: f32,
        water_film_mm: f32,
    ) -> f32 {
        let wetness = self.wetness.clamp(0.0, 1.0);
        let base = surface.dry_grip + (surface.wet_grip - surface.dry_grip) * wetness;

        let temperature = track_temperature_grip(self.track_temperature_c);
        // The film is the water the tyre itself is holding, which is the part
        // that can seal the contact patch and lift it.
        let effective_water_m = (water_depth_m.max(0.0) + water_film_mm.max(0.0) * 0.001).max(0.0);
        let aquaplaning = aquaplaning_factor(speed_mps, effective_water_m);

        (base * temperature * self.grip_scale * aquaplaning).clamp(0.01, 2.5)
    }

    /// Dynamic water film thickness a tyre reaches in the steady state, mm.
    ///
    /// The tread only sees water for the short time it is in the contact
    /// patch, and that dwell time shrinks with speed, so a faster tyre picks up
    /// less; load and a cold, soft tyre both hold on to more. The steady state
    /// is the balance of the two, which is what a rolling tyre settles at.
    pub fn water_film_mm(
        &self,
        speed_mps: f32,
        water_depth_m: f32,
        load_n: f32,
        tyre_c: f32,
    ) -> f32 {
        let water_mm = (water_depth_m.max(0.0) * 1000.0).clamp(0.0, 200.0);
        if water_mm <= 0.0 {
            return 0.0;
        }
        let speed = speed_mps.abs().max(0.5);
        let load = (load_n / 4_000.0).clamp(0.0, 4.0);
        // Dwell time in the contact patch falls with speed.
        let dwell = 1.0 / speed.sqrt().clamp(0.5, 12.0);
        // A loaded, cold tyre grips the film; a hot one throws it off.
        let hold = (0.3 + 0.7 * load).clamp(0.2, 1.5) * (1.0 + 0.004 * (60.0 - tyre_c).max(0.0));
        (water_mm * dwell * hold).clamp(0.0, water_mm)
    }

    /// Rolling-resistance coefficient after rain and standing water.
    pub fn rolling_resistance(&self, surface: &SurfaceProperties, water_depth_m: f32) -> f32 {
        let wetness = self.wetness.clamp(0.0, 1.0);
        let water = water_depth_m.max(0.0);
        (surface.rolling_resistance * (1.0 + 0.6 * wetness + 40.0 * water)).clamp(0.0, 1.5)
    }
}

/// Tyre working-window curve. Dry racing tyres are happiest around 30–40 °C
/// track temperature; cold and overheated surfaces lose grip.
fn track_temperature_grip(track_c: f32) -> f32 {
    let normalized = (track_c - 35.0) / 45.0;
    let window = 0.78 + 0.22 * (-normalized * normalized).exp();
    // Below freezing the rubber goes glassy and keeps losing grip, down to a
    // winter floor at -30 °C. Zero degrees is not a wall: a cold tyre simply
    // grips badly, exactly like a very hot one.
    let cold = if track_c < 0.0 {
        (1.0 + track_c / 30.0).clamp(0.0, 1.0)
    } else {
        1.0
    };
    (window * cold).clamp(0.35, 1.05)
}

/// Aquaplaning: grip falls with speed and water depth. A 1 cm puddle at
/// 30 m/s keeps roughly 45 % of the dry grip; at 50 m/s it is about 23 %.
fn aquaplaning_factor(speed_mps: f32, water_depth_m: f32) -> f32 {
    let water_mm = water_depth_m.max(0.0) * 1000.0;
    let speed_ratio = (speed_mps.max(0.0) / 30.0).powi(2);
    let severity = water_mm * 0.12 * speed_ratio;
    1.0 / (1.0 + severity)
}

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::SurfaceKind;

    #[test]
    fn dry_grip_beats_wet_grip() {
        let asphalt = SurfaceKind::Asphalt.properties();
        let dry = Weather::dry();
        let wet = Weather::wet(1.0);
        assert!(dry.grip(&asphalt, 30.0, 0.0) > wet.grip(&asphalt, 30.0, 0.0));
    }

    #[test]
    fn asphalt_beats_grass_and_ice() {
        let weather = Weather::dry();
        let asphalt = SurfaceKind::Asphalt.properties();
        let grass = SurfaceKind::Grass.properties();
        let ice = SurfaceKind::Ice.properties();
        assert!(weather.grip(&asphalt, 20.0, 0.0) > weather.grip(&grass, 20.0, 0.0));
        assert!(weather.grip(&grass, 20.0, 0.0) > weather.grip(&ice, 20.0, 0.0));
    }

    #[test]
    fn standing_water_costs_more_grip_at_speed() {
        let asphalt = SurfaceKind::Asphalt.properties();
        let weather = Weather::wet(1.0);
        let slow = weather.grip(&asphalt, 5.0, 0.01);
        let fast = weather.grip(&asphalt, 45.0, 0.01);
        assert!(fast < slow * 0.6, "slow={slow} fast={fast}");
    }

    #[test]
    fn temperature_curve_peaks_near_working_window() {
        let at_35 = track_temperature_grip(35.0);
        let at_0 = track_temperature_grip(0.0);
        let at_90 = track_temperature_grip(90.0);
        assert!(at_35 > at_0);
        assert!(at_35 > at_90);
    }

    #[test]
    fn weather_blend_hits_both_ends() {
        let dry = Weather::dry();
        let wet = Weather::wet(0.8);
        assert_eq!(Weather::lerp(&dry, &wet, 0.0), dry);
        assert_eq!(Weather::lerp(&dry, &wet, 1.0), wet);
        let half = Weather::lerp(&dry, &wet, 0.5);
        assert!((half.wetness - wet.wetness * 0.5).abs() < 1e-6);
    }

    #[test]
    fn wetness_and_water_raise_rolling_resistance() {
        let asphalt = SurfaceKind::Asphalt.properties();
        let dry = Weather::dry().rolling_resistance(&asphalt, 0.0);
        let wet = Weather::wet(1.0).rolling_resistance(&asphalt, 0.0);
        let flooded = Weather::wet(1.0).rolling_resistance(&asphalt, 0.01);
        assert!(wet > dry);
        assert!(flooded > wet);
    }

    #[test]
    fn cold_track_temperatures_lose_grip_without_a_wall_at_zero() {
        let warm = track_temperature_grip(30.0);
        let freezing = track_temperature_grip(0.0);
        let cold = track_temperature_grip(-15.0);
        let arctic = track_temperature_grip(-30.0);
        assert!(warm > freezing, "{warm} vs {freezing}");
        assert!(freezing > cold, "{freezing} vs {cold}");
        assert!(cold > arctic, "{cold} vs {arctic}");
        // A floor, not zero: winter tyres still grip something.
        assert!(arctic > 0.0);
    }

    #[test]
    fn ambient_c_picks_air_for_open_parts_and_road_for_tyres() {
        let winter = Weather::dry().with_temperatures(-12.0, -20.0).clamped();
        assert!((winter.ambient_c(false) - (-12.0)).abs() < 1e-6);
        assert!((winter.ambient_c(true) - (-20.0)).abs() < 1e-6);
        // Nonsense falls back to the default world, not to zero.
        let broken = Weather {
            air_temperature_c: f32::NAN,
            track_temperature_c: f32::NAN,
            ..Weather::dry()
        }
        .clamped();
        assert!(broken.air_temperature_c > 0.0);
        assert!(broken.track_temperature_c > 0.0);
    }
}
