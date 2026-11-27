//! Resolved `CfgWorlds` preview settings and DayZ ocean material (`.emat`) parsing.
//!
//! DayZ maps inherit every unset world property from their base class (`Enoch`, `ChernarusPlus`,
//! or a custom world class) and ultimately `CAWorld`. This module provides a single source of
//! truth for those defaults, date/time parsing, top-view grid steps, and `MatOceanOld` `.emat`
//! parsing so both the editor viewport and the Map config window agree on every value.
//!
//! Vanilla `MatOceanOld` `.emat` files (such as `P:\dz\worlds\sakhal\data\sakhal_sea.emat` and
//! `sakhal_sea_overcast.emat`) store plain whitespace-separated key/value lines inside
//! `MatOceanOld { ... }`:
//! - `Color r g b a` — deep water body colour (default `0.0235 0.0588 0.0745`)
//! - `DeepMapAlphaColor r g b a` — distant deep-sea tint (default `0.47 0.53 0.59`)
//! - `WaterExtinction f` — clarity distance in metres (default `15`)
//! - `SunPower f` — specular exponent of sun glints (default `256`)
//! - `SunInfluence f` — specular intensity multiplier (default `4`)
//! - `WetFloor f` — wet shore darkening factor (default `0.5`)
//! - `GreyModifier f` — desaturation/darkening in overcast materials (default `1.0`)
//! - `FoamPower f` — shore foam strength (default `0.64`)
//! - `FoamColor r g b a` — shore foam tint (default `0.2667 0.2667 0.251`)
//! - `NormalPower f` — surface ripple normal strength (default `0.1`)
use crate::world::{BaseWorld, WorldMeta};
use std::path::Path;

/// Default string representation of a `CfgWorlds` property inherited from `base`.
pub fn base_value(base: &BaseWorld, key: &str) -> &'static str {
    let chernarus = matches!(base, BaseWorld::ChernarusPlus);
    match key {
        "cutscenes" => if chernarus { "ChernarusPlusIntro" } else { "EnochIntro" },
        "startTime" => "9:00",
        "startDate" => if chernarus { "5/10/2015" } else { "25/8/2015" },
        "ceFiles" => if chernarus { r"DZ\worlds\chernarusplus\ce" } else { r"DZ\worlds\enoch\ce" },
        "mapDisplayNameKey" => "#STR_CfgVehicles_ChernarusMap0",
        "mapDescriptionKey" => "#STR_CfgVehicles_ChernarusMap1",
        "mapTextureClosed" => if chernarus { r"dz\gear\navigation\data\map_chernarus_co.paa" } else { r"dz\gear\navigation\data\map_enoch_co.paa" },
        "mapTextureOpened" => if chernarus { r"dz\structures\signs\tourist\data\karta_co.paa" } else { r"dz\structures_bliss\signs\tourist\data\karta_enoch_co.paa" },
        "mapTextureLegend" => if chernarus { r"dz\structures\signs\tourist\data\karta_side_co.paa" } else { r"dz\structures_bliss\signs\tourist\data\karta_enoch_side_co.paa" },
        "userMapPath" => if chernarus { r"dz\gear\navigation\data\usermap" } else { r"dz\worlds\enoch\data\usermap" },
        "satellite" => if chernarus { r"DZ\worlds\chernarusplus\data\outside_sat_co.paa" } else { r"DZ\worlds\enoch\data\enoch_satout_co.paa" },
        "texture" => if chernarus { r"DZ\surfaces\data\terrain\cp_grass_ca.paa" } else { r"DZ\surfaces_bliss\data\terrain\en_grass1_ca.paa" },
        "nopx" => if chernarus { r"DZ\surfaces\data\terrain\cp_grass_nopx.paa" } else { r"DZ\surfaces_bliss\data\terrain\en_grass1_nopx.paa" },
        "midDetailTexture" => if chernarus { r"DZ\worlds\chernarusplus\data\middle_sat_mco.paa" } else { r"DZ\worlds\enoch\data\enoch_middle_mco.paa" },
        "terrainNormalTexture" => if chernarus { r"DZ\worlds\chernarusplus\data\global_nohq.paa" } else { r"DZ\worlds\enoch\data\enoch_global_nohq.paa" },
        "oceanMaterial" => if chernarus { "" } else { r"dz\worlds\enoch\data\enoch_riversea.emat" },
        "noDetailDist" => if chernarus { "40" } else { "65" },
        "fullDetailDist" => if chernarus { "15" } else { "10" },
        "clutterDist" => "125",
        "clutterGrid" => "1",
        "minTrees" => "10",
        "minRocks" => "5",
        "envTexture" => r"DZ\Data\data\env_co.paa",
        "atmosphereTexture" => r"DZ\worlds\chernarusplus\data\Sky_Stage01_Clear_sky.paa",
        "maxTide" => "1.5",
        "maxWave" => "0.25",
        "waveAmplitude" => "0.081",
        "waveFrequency" => "9",
        "waveOctaves" => "6",
        "wavePersistence" => "0.745",
        "starsIntensity" => "0.181",
        "hazeDistCoef" => "0",
        "hazeFogCoef" => "1",
        "volFogOffset" => "0",
        "satNormStart" => "100",
        "satNormEnd" => "500",
        "terrainNormalPower" => "3.21",
        "soundMapAttenCoef" => "0.01",
        "tileWidth" => "50",
        "maxSlope" => "60",
        "maxStep" => "0.45",
        _ => "",
    }
}

/// Day of year (1..=365) from calendar day and month.
pub fn day_of_year(day: i32, month: i32) -> u32 {
    const BEFORE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let m = month.clamp(1, 12) as usize;
    BEFORE[m - 1] + day.clamp(1, 31) as u32
}

/// Parses a `startDate` string like `"25/8/2015"` (day/month/year) into day of year (1..=365).
pub fn parse_day_of_year(date: &str) -> Option<u32> {
    let parts: Vec<i32> = date.split('/').filter_map(|p| p.trim().parse().ok()).collect();
    match parts.as_slice() {
        [d, m, ..] if (1..=12).contains(m) => Some(day_of_year(*d, *m)),
        _ => None,
    }
}

/// Parses a `startTime` string like `"9:00"` or `"6:30"` into hours `0.0..=24.0`.
pub fn parse_start_time(time: &str) -> Option<f32> {
    let mut it = time.trim().split(':');
    let h: f32 = it.next()?.trim().parse().ok()?;
    let m: f32 = it.next().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0);
    if !(0.0..=24.0).contains(&h) || !(0.0..60.0).contains(&m) {
        return None;
    }
    Some((h + m / 60.0).clamp(0.0, 24.0))
}

/// Parsed visual parameters of a DayZ `MatOceanOld` `.emat` file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OceanMaterial {
    /// Water body colour (`Color`).
    pub color: [f32; 3],
    /// Distant deep-sea tint (`DeepMapAlphaColor`).
    pub deep_color: [f32; 3],
    /// Clarity distance in metres (`WaterExtinction`).
    pub extinction: f32,
    /// Specular exponent (`SunPower`).
    pub sun_power: f32,
    /// Specular intensity multiplier (`SunInfluence`).
    pub sun_influence: f32,
    /// Wet shore darkening strength (`WetFloor`, 0..1).
    pub wet_floor: f32,
    /// Overcast greying / darkening multiplier (`GreyModifier`, 0..1).
    pub grey_modifier: f32,
    /// Shore foam strength (`FoamPower`).
    pub foam_power: f32,
    /// Shore foam colour (`FoamColor`, raw emat RGB).
    pub foam_color: [f32; 3],
    /// Ripple normal strength (`NormalPower`, vanilla default 0.1).
    pub normal_power: f32,
}

impl Default for OceanMaterial {
    fn default() -> Self {
        Self {
            color: [0.0235, 0.0588, 0.0745],
            deep_color: [0.47, 0.53, 0.59],
            extinction: 15.0,
            sun_power: 256.0,
            sun_influence: 4.0,
            wet_floor: 0.5,
            grey_modifier: 1.0,
            foam_power: 0.64,
            foam_color: [0.2667, 0.2667, 0.251],
            normal_power: 0.1,
        }
    }
}

impl OceanMaterial {
    /// Parses a DayZ `.emat` text file into `OceanMaterial`, keeping vanilla defaults for unset keys.
    pub fn parse_emat(text: &str) -> Option<Self> {
        let mut mat = Self::default();
        let mut saw_any = false;
        for raw in text.lines() {
            let line = raw.split("//").next().unwrap_or("").trim();
            if line.is_empty() || line == "{" || line == "}" {
                continue;
            }
            let mut parts = line.split_whitespace();
            let Some(key) = parts.next() else { continue };
            let nums: Vec<f32> = parts.filter_map(|p| p.parse::<f32>().ok()).collect();
            match key {
                "Color" if nums.len() >= 3 => {
                    mat.color = [nums[0].clamp(0.0, 1.0), nums[1].clamp(0.0, 1.0), nums[2].clamp(0.0, 1.0)];
                    saw_any = true;
                }
                "DeepMapAlphaColor" if nums.len() >= 3 => {
                    mat.deep_color = [nums[0].clamp(0.0, 1.0), nums[1].clamp(0.0, 1.0), nums[2].clamp(0.0, 1.0)];
                    saw_any = true;
                }
                "WaterExtinction" if !nums.is_empty() => {
                    mat.extinction = nums[0].clamp(0.2, 200.0);
                    saw_any = true;
                }
                "SunPower" if !nums.is_empty() => {
                    mat.sun_power = nums[0].clamp(4.0, 2048.0);
                    saw_any = true;
                }
                "SunInfluence" if !nums.is_empty() => {
                    mat.sun_influence = nums[0].clamp(0.0, 20.0);
                    saw_any = true;
                }
                "WetFloor" if !nums.is_empty() => {
                    mat.wet_floor = nums[0].clamp(0.0, 1.0);
                    saw_any = true;
                }
                "GreyModifier" if !nums.is_empty() => {
                    mat.grey_modifier = nums[0].clamp(0.0, 1.5);
                    saw_any = true;
                }
                "FoamPower" if !nums.is_empty() => {
                    mat.foam_power = nums[0].clamp(0.0, 2.0);
                    saw_any = true;
                }
                "FoamColor" if nums.len() >= 3 => {
                    mat.foam_color = [nums[0].clamp(0.0, 1.0), nums[1].clamp(0.0, 1.0), nums[2].clamp(0.0, 1.0)];
                    saw_any = true;
                }
                "NormalPower" if !nums.is_empty() => {
                    mat.normal_power = nums[0].clamp(0.0, 2.0);
                    saw_any = true;
                }
                _ => {}
            }
        }
        saw_any.then_some(mat)
    }

    /// Loads an `.emat` file from `work_drive`, returning `None` if missing or unreadable.
    pub fn load(work_drive: &Path, rel_path: &str) -> Option<Self> {
        let trimmed = rel_path.trim().trim_start_matches(['\\', '/']);
        if trimmed.is_empty() {
            return None;
        }
        let full = work_drive.join(trimmed.replace('\\', "/"));
        let text = std::fs::read_to_string(full).ok()?;
        Self::parse_emat(&text)
    }
}

/// Effective values of `MapConfig` + `WorldMeta` with every unset field resolved from `BaseWorld`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedMapConfig {
    pub base_name: String,
    pub has_ocean: bool,
    pub center: [f32; 2],
    pub center_custom: bool,
    pub latitude: f32,
    pub longitude: f32,
    pub start_time_str: String,
    pub start_hour: f32,
    pub start_date_str: String,
    pub start_day: u32,
    pub grid_steps: [u32; 3],
    pub clutter_dist: f32,
    pub clutter_grid: f32,
    pub full_detail_dist: f32,
    pub no_detail_dist: f32,
    pub height_blending: bool,
    pub bicubic: bool,
    pub mid_detail_texture: String,
    pub terrain_normal_texture: String,
    pub sat_normal_blend_start: f32,
    pub sat_normal_blend_end: f32,
    pub terrain_normal_power: f32,
    pub outside_satellite: String,
    pub outside_synth: bool,
    pub outside_texture: String,
    pub outside_nopx: String,
    pub ocean_material: String,
    pub ocean_nice_material: String,
    pub ocean_storm_material: String,
    pub max_tide: f32,
    pub max_wave: f32,
    pub wave_amplitude: f32,
    pub wave_frequency: f32,
    pub wave_octaves: u32,
    pub wave_persistence: f32,
    pub env_texture: String,
    pub atmosphere_texture: String,
    pub stars_intensity: f32,
    pub haze_dist_coef: f32,
    pub haze_fog_coef: f32,
    pub vol_fog_offset: f32,
}

impl ResolvedMapConfig {
    pub fn of(meta: &WorldMeta) -> Self {
        let cfg = &meta.config;
        let base = &cfg.base;
        let side = meta.size_m();
        let text = |v: &str, key: &str| {
            let t = v.trim();
            if t.is_empty() { base_value(base, key).to_string() } else { t.to_string() }
        };
        let num = |v: Option<f32>, key: &str, fallback: f32| {
            v.unwrap_or_else(|| base_value(base, key).parse().unwrap_or(fallback))
        };
        let start_time_str = text(&cfg.start_time, "startTime");
        let start_hour = parse_start_time(&start_time_str).unwrap_or(9.0);
        let start_date_str = text(&cfg.start_date, "startDate");
        let base_day = parse_day_of_year(base_value(base, "startDate")).unwrap_or(237);
        let start_day = parse_day_of_year(&start_date_str).unwrap_or(base_day);

        let ocean_material = text(&cfg.ocean_material, "oceanMaterial");
        let ocean_nice_material = if cfg.ocean_nice_material.trim().is_empty() {
            ocean_material.clone()
        } else {
            cfg.ocean_nice_material.trim().to_string()
        };
        let ocean_storm_material = if cfg.ocean_storm_material.trim().is_empty() {
            ocean_material.clone()
        } else {
            cfg.ocean_storm_material.trim().to_string()
        };

        let full_detail_dist = num(cfg.full_detail_dist, "fullDetailDist", 10.0).max(1.0);
        let no_detail_dist = num(cfg.no_detail_dist, "noDetailDist", 65.0).max(full_detail_dist + 1.0);

        Self {
            base_name: base.class().to_string(),
            has_ocean: cfg.has_ocean,
            center: cfg.center.unwrap_or([side * 0.5, side * 0.5]),
            center_custom: cfg.center.is_some(),
            latitude: meta.latitude,
            longitude: meta.longitude,
            start_time_str,
            start_hour,
            start_date_str,
            start_day,
            grid_steps: [cfg.grid_steps[0].max(1), cfg.grid_steps[1].max(1), cfg.grid_steps[2].max(1)],
            clutter_dist: num(cfg.clutter_dist, "clutterDist", 125.0).clamp(15.0, 400.0),
            clutter_grid: num(cfg.clutter_grid, "clutterGrid", 1.0).clamp(0.4, 4.0),
            full_detail_dist,
            no_detail_dist,
            // Vanilla DayZ worlds enable height-based terrain blending and bilinear (non-bicubic) masks by default.
            height_blending: cfg.height_blending_mode.unwrap_or(true),
            bicubic: cfg.bicubic_mode.unwrap_or(false),
            mid_detail_texture: text(&cfg.mid_detail_texture, "midDetailTexture"),
            terrain_normal_texture: text(&cfg.terrain_normal_texture, "terrainNormalTexture"),
            sat_normal_blend_start: num(cfg.satellite_normal_blend_start, "satNormStart", 100.0).max(0.0),
            sat_normal_blend_end: num(cfg.satellite_normal_blend_end, "satNormEnd", 500.0).max(1.0),
            terrain_normal_power: num(cfg.terrain_normal_power, "terrainNormalPower", 3.21).max(0.0),
            outside_satellite: text(&cfg.outside_satellite, "satellite"),
            outside_synth: cfg.outside_synth.unwrap_or(true),
            outside_texture: text(&cfg.outside_texture, "texture"),
            outside_nopx: text(&cfg.outside_nopx, "nopx"),
            ocean_material,
            ocean_nice_material,
            ocean_storm_material,
            max_tide: num(cfg.max_tide, "maxTide", 1.5).clamp(0.0, 10.0),
            max_wave: num(cfg.max_wave, "maxWave", 0.25).clamp(0.0, 5.0),
            wave_amplitude: num(cfg.wave_amplitude, "waveAmplitude", 0.081).clamp(0.0, 2.0),
            wave_frequency: num(cfg.wave_frequency, "waveFrequency", 9.0).clamp(0.1, 50.0),
            wave_octaves: cfg.wave_octaves.unwrap_or(6).clamp(1, 12),
            wave_persistence: num(cfg.wave_persistence, "wavePersistence", 0.745).clamp(0.0, 2.0),
            env_texture: text(&cfg.env_texture, "envTexture"),
            atmosphere_texture: text(&cfg.atmosphere_texture, "atmosphereTexture"),
            stars_intensity: num(cfg.stars_intensity, "starsIntensity", 0.181).clamp(0.0, 2.0),
            haze_dist_coef: num(cfg.haze_dist_coef, "hazeDistCoef", 0.0).clamp(0.0, 5.0),
            haze_fog_coef: num(cfg.haze_fog_coef, "hazeFogCoef", 1.0).clamp(0.0, 5.0),
            vol_fog_offset: num(cfg.vol_fog_offset, "volFogOffset", 0.0).clamp(-500.0, 500.0),
        }
    }

    /// Active ocean `.emat` path for the current overcast level (0 clear .. 1 stormy).
    pub fn ocean_material_for(&self, overcast: f32) -> &str {
        if overcast >= 0.5 {
            &self.ocean_storm_material
        } else {
            &self.ocean_nice_material
        }
    }

    /// Picks the grid step from `grid_steps` (Zoom1/Zoom2/Zoom3) that keeps lines at least
    /// ~45 px apart on screen at the given orthographic `mpp` (metres per point).
    pub fn grid_step_for_mpp(&self, mpp: f32) -> f32 {
        let mut steps = self.grid_steps;
        steps.sort_unstable();
        let min_world = (mpp.max(0.01)) * 45.0;
        for s in steps {
            if s as f32 >= min_world {
                return s as f32;
            }
        }
        steps[2] as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_base_world_defaults_and_overrides() {
        let mut meta = WorldMeta::new("Test", 1024, 4.0);
        meta.config.base = BaseWorld::ChernarusPlus;
        let c = ResolvedMapConfig::of(&meta);
        assert_eq!(c.full_detail_dist, 15.0);
        assert_eq!(c.no_detail_dist, 40.0);
        assert_eq!(c.start_day, day_of_year(5, 10));
        assert_eq!(c.start_hour, 9.0);
        assert_eq!(c.center, [2048.0, 2048.0]);
        assert!(!c.center_custom);

        meta.config.base = BaseWorld::Enoch;
        let e = ResolvedMapConfig::of(&meta);
        assert_eq!(e.full_detail_dist, 10.0);
        assert_eq!(e.no_detail_dist, 65.0);
        assert_eq!(e.start_day, day_of_year(25, 8));

        meta.config.start_time = "6:30".into();
        meta.config.start_date = "1/1/2020".into();
        meta.config.max_tide = Some(2.4);
        meta.config.center = Some([1200.0, 3400.0]);
        meta.config.ocean_material = r"my\sea.emat".into();
        meta.config.ocean_storm_material = r"my\storm.emat".into();
        let o = ResolvedMapConfig::of(&meta);
        assert_eq!(o.start_hour, 6.5);
        assert_eq!(o.start_day, 1);
        assert_eq!(o.max_tide, 2.4);
        assert!(o.center_custom);
        assert_eq!(o.center, [1200.0, 3400.0]);
        assert_eq!(o.ocean_material_for(0.2), r"my\sea.emat");
        assert_eq!(o.ocean_material_for(0.8), r"my\storm.emat");
    }

    #[test]
    fn parses_vanilla_ocean_emat() {
        let sample = r#"
MatOceanOld {
 Color 0.05 0.06 0.07 0
 DeepMapAlphaColor 0.40 0.45 0.50 1
 SunPower 128
 SunInfluence 3.5
 WaterExtinction 9.5
 WetFloor 0.7
 GreyModifier 0.8
 FoamPower 0.5
 FoamColor 0.3 0.3 0.28 1
 NormalPower 0.2
}
"#;
        let m = OceanMaterial::parse_emat(sample).unwrap();
        assert_eq!(m.color, [0.05, 0.06, 0.07]);
        assert_eq!(m.deep_color, [0.40, 0.45, 0.50]);
        assert_eq!(m.sun_power, 128.0);
        assert_eq!(m.sun_influence, 3.5);
        assert_eq!(m.extinction, 9.5);
        assert_eq!(m.wet_floor, 0.7);
        assert_eq!(m.grey_modifier, 0.8);
        assert_eq!(m.foam_power, 0.5);
        assert_eq!(m.foam_color, [0.3, 0.3, 0.28]);
        assert_eq!(m.normal_power, 0.2);
    }

    #[test]
    fn picks_zoom_appropriate_grid_step() {
        let meta = WorldMeta::new("Test", 1024, 4.0);
        let r = ResolvedMapConfig::of(&meta);
        assert_eq!(r.grid_step_for_mpp(1.0), 100.0);
        assert_eq!(r.grid_step_for_mpp(5.0), 1000.0);
        assert_eq!(r.grid_step_for_mpp(30.0), 10000.0);
    }
}
