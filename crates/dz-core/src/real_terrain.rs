//! Maps of real places: a square of the Earth with its satellite picture, its heights, its
//! OpenStreetMap shapes and a surface mask read from the picture.
//!
//! Everything comes from free web map services that need no key:
//!
//! - Satellite pictures are "slippy map" tiles (256 × 256 pixels in the Web Mercator
//!   projection, `{z}/{x}/{y}`): Esri World Imagery, EOX Sentinel-2 cloudless, the open
//!   orthophotos of Poland (Geoportal WMTS in EPSG:3857, whose grid is the same), Czechia,
//!   Austria and the USA, or any server the user gives.
//! - Heights are the Terrarium tiles of the AWS Terrain Tiles (Mapzen) set: each pixel holds
//!   `red * 256 + green + blue / 256 - 32768` metres. They are mostly SRTM (about 30 m) on land
//!   and better in some countries; the sea floor is under 0.
//! - Shapes come from the Overpass API of OpenStreetMap (`out geom` gives the points of every
//!   way and of the members of every relation).
//! - Place names are looked up with Nominatim.
//!
//! The map is a flat square, so the place is projected with Web Mercator scaled by the cosine
//! of the latitude of its centre: metres stay metres there and north stays up. Over 20 km the
//! scale is off by a few parts in a thousand at the north and south edges.
//!
//! The pictures and data have their own licences (Esri's imagery may not be published in a
//! map, Sentinel-2 cloudless 2016, ČÚZK and basemap.at are CC BY 4.0, the Polish orthophoto
//! is open data, USGS is public domain, OpenStreetMap is ODbL), which the editor shows where
//! the source is chosen.
use crate::satgen::SatClassify;
use crate::shapes::{MapShape, ShapeKind, ShapeLayer};
use crate::terrain::{Heightmap, RgbImage};
use crate::world::{World, WorldMeta};
use crate::Job;
use anyhow::{bail, Context, Result};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Radius of the Web Mercator sphere.
const R: f64 = 6_378_137.0;
/// Pixels per side of a map tile.
const TILE: i64 = 256;
/// Downloads at the same time; the servers are free, so this stays polite.
const PARALLEL: usize = 8;
const USER_AGENT: &str = concat!("DZWorldEditor/", env!("CARGO_PKG_VERSION"), " (+https://github.com/MrKamil404/DZWorldEditor)");
/// Lowest height kept: deep sea floor is of no use to the game and only wastes height range.
pub const LOWEST: f32 = -100.0;

/// A square of the Earth: its centre and the length of its side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GeoArea {
    pub lat: f64,
    pub lon: f64,
    pub size_m: f64,
}

fn mercator(lon: f64, lat: f64) -> [f64; 2] {
    let lat = lat.clamp(-85.05, 85.05);
    [R * lon.to_radians(), R * (PI / 4.0 + lat.to_radians() / 2.0).tan().ln()]
}

fn unmercator(m: [f64; 2]) -> (f64, f64) {
    ((m[0] / R).to_degrees(), (2.0 * (m[1] / R).exp().atan() - PI / 2.0).to_degrees())
}

/// Pixel of the whole world image at `zoom` for a Web Mercator point.
fn world_pixel(m: [f64; 2], zoom: u8) -> [f64; 2] {
    let n = (TILE << zoom) as f64;
    [(m[0] + PI * R) / (2.0 * PI * R) * n, (PI * R - m[1]) / (2.0 * PI * R) * n]
}

/// Metres on the ground per tile pixel at a latitude.
pub fn ground_resolution(lat: f64, zoom: u8) -> f64 {
    2.0 * PI * R * lat.to_radians().cos() / (TILE << zoom) as f64
}

/// The lowest zoom whose pixels are about as small as `metres_per_px`, at most `max_zoom`.
pub fn zoom_for(lat: f64, metres_per_px: f64, max_zoom: u8) -> u8 {
    (0..=max_zoom).find(|z| ground_resolution(lat, *z) <= metres_per_px * 1.2).unwrap_or(max_zoom)
}

impl GeoArea {
    fn scale(&self) -> f64 {
        1.0 / self.lat.to_radians().cos().max(0.01)
    }
    /// Longitude and latitude of a map point (x east, z north, from the south-west corner).
    pub fn to_lonlat(&self, x: f64, z: f64) -> (f64, f64) {
        let c = mercator(self.lon, self.lat);
        let k = self.scale();
        unmercator([c[0] + (x - self.size_m / 2.0) * k, c[1] + (z - self.size_m / 2.0) * k])
    }
    /// The map point of a longitude and latitude.
    pub fn to_map(&self, lon: f64, lat: f64) -> [f64; 2] {
        let c = mercator(self.lon, self.lat);
        let m = mercator(lon, lat);
        let k = self.scale();
        [(m[0] - c[0]) / k + self.size_m / 2.0, (m[1] - c[1]) / k + self.size_m / 2.0]
    }
    /// South, west, north and east edges in degrees.
    pub fn bbox(&self) -> [f64; 4] {
        let (w, s) = self.to_lonlat(0.0, 0.0);
        let (e, n) = self.to_lonlat(self.size_m, self.size_m);
        [s, w, n, e]
    }
    /// Tile pixel at `zoom` of a map point.
    fn pixel(&self, x: f64, z: f64, zoom: u8) -> [f64; 2] {
        let c = mercator(self.lon, self.lat);
        let k = self.scale();
        world_pixel([c[0] + (x - self.size_m / 2.0) * k, c[1] + (z - self.size_m / 2.0) * k], zoom)
    }
}

/// A server of map tiles; `url` has `{z}`, `{x}` and `{y}` in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TileSource {
    pub name: String,
    pub url: String,
    pub max_zoom: u8,
    /// Who must be named where the picture is shown.
    pub attribution: String,
}

/// The satellite picture servers to choose from.
pub fn satellite_sources() -> Vec<TileSource> {
    vec![
        TileSource {
            name: "Esri World Imagery".into(),
            url: "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}".into(),
            max_zoom: 18,
            attribution: "Esri, Maxar, Earthstar Geographics, and the GIS User Community".into(),
        },
        TileSource {
            name: "Sentinel-2 cloudless 2016 (EOX)".into(),
            url: "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless_3857/default/g/{z}/{y}/{x}.jpg".into(),
            max_zoom: 14,
            attribution: "Sentinel-2 cloudless - https://s2maps.eu by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)".into(),
        },
        // National orthophotos: sharp and open data, but each covers only its own country.
        TileSource {
            name: "Poland: Geoportal orthophoto (GUGiK)".into(),
            url: "https://mapy.geoportal.gov.pl/wss/service/PZGIK/ORTO/WMTS/StandardResolution?SERVICE=WMTS&REQUEST=GetTile&VERSION=1.0.0&LAYER=ORTOFOTOMAPA&STYLE=default&FORMAT=image/jpeg&TILEMATRIXSET=EPSG:3857&TILEMATRIX=EPSG:3857:{z}&TILEROW={y}&TILECOL={x}".into(),
            max_zoom: 19,
            attribution: "Główny Urząd Geodezji i Kartografii (geoportal.gov.pl)".into(),
        },
        TileSource {
            name: "Czechia: ČÚZK orthophoto".into(),
            url: "https://ags.cuzk.cz/arcgis1/rest/services/ORTOFOTO_WM/MapServer/tile/{z}/{y}/{x}".into(),
            max_zoom: 19,
            attribution: "ČÚZK (cuzk.gov.cz)".into(),
        },
        TileSource {
            name: "Austria: basemap.at orthophoto".into(),
            url: "https://mapsneu.wien.gv.at/basemap/bmaporthofoto30cm/normal/google3857/{z}/{y}/{x}.jpeg".into(),
            max_zoom: 19,
            attribution: "basemap.at".into(),
        },
        TileSource {
            name: "USA: USGS imagery".into(),
            url: "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}".into(),
            max_zoom: 16,
            attribution: "USGS The National Map".into(),
        },
    ]
}

/// The height tiles.
pub fn elevation_source() -> TileSource {
    TileSource {
        name: "Terrain Tiles (AWS, Terrarium)".into(),
        url: "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png".into(),
        max_zoom: 15,
        attribution: "Mapzen Terrain Tiles: SRTM, GMTED, ETOPO1, NRCan, EU-DEM and others".into(),
    }
}

/// Downloads with a cache on disk, so the same tiles are fetched only once.
#[derive(Clone)]
pub struct Fetcher {
    agent: ureq::Agent,
    cache: Option<PathBuf>,
}

impl Fetcher {
    pub fn new(cache: Option<PathBuf>) -> Result<Self> {
        let tls = native_tls::TlsConnector::new().context("Could not set up the HTTPS connection")?;
        let agent = ureq::AgentBuilder::new()
            .tls_connector(Arc::new(tls))
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(600))
            .user_agent(USER_AGENT)
            .max_idle_connections_per_host(PARALLEL)
            .build();
        Ok(Self { agent, cache })
    }

    fn read(resp: ureq::Response) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut out)?;
        Ok(out)
    }

    /// One tile; `None` when the server has none there.
    pub fn tile(&self, src: &TileSource, z: u8, x: i64, y: i64) -> Result<Option<Vec<u8>>> {
        let url = src.url.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string());
        let file = self.cache.as_ref().map(|c| {
            let key = sha1_smol::Sha1::from(&src.url).digest().to_string();
            c.join(&key[..12]).join(z.to_string()).join(x.to_string()).join(y.to_string())
        });
        if let Some(f) = &file {
            if let Ok(b) = std::fs::read(f) {
                return Ok((!b.is_empty()).then_some(b));
            }
        }
        let mut last = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 * attempt));
            }
            match self.agent.get(&url).call() {
                Ok(r) => {
                    let b = Self::read(r)?;
                    if let Some(f) = &file {
                        if let Some(d) = f.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        let _ = std::fs::write(f, &b);
                    }
                    return Ok((!b.is_empty()).then_some(b));
                }
                Err(ureq::Error::Status(404 | 204, _)) => {
                    if let Some(f) = &file {
                        if let Some(d) = f.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        let _ = std::fs::write(f, []);
                    }
                    return Ok(None);
                }
                Err(e) => last = Some(e),
            }
        }
        Err(anyhow::anyhow!("{}", last.map(|e| e.to_string()).unwrap_or_default())).with_context(|| format!("Could not download {url}"))
    }
}

/// A pixel of a tile that can be mixed with others.
trait Px: Copy + Default + Send + Sync + 'static {
    fn decode(bytes: &[u8]) -> Result<Vec<Self>>;
    fn channels(self) -> [f32; 3];
    fn from_channels(c: [f32; 3]) -> Self;
    /// A tile the server sent with nothing in it.
    fn is_void(_tile: &[Self]) -> bool {
        false
    }
}

impl Px for [u8; 3] {
    fn decode(bytes: &[u8]) -> Result<Vec<Self>> {
        let img = image::load_from_memory(bytes)?.into_rgb8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let raw = img.into_raw();
        Ok(resize_tile(&raw.chunks(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<_>>(), w, h))
    }
    fn channels(self) -> [f32; 3] {
        [self[0] as f32, self[1] as f32, self[2] as f32]
    }
    fn from_channels(c: [f32; 3]) -> Self {
        [c[0].round().clamp(0.0, 255.0) as u8, c[1].round().clamp(0.0, 255.0) as u8, c[2].round().clamp(0.0, 255.0) as u8]
    }
}

/// A Terrarium height.
#[derive(Clone, Copy, Default)]
struct Height(f32);

impl Px for Height {
    fn decode(bytes: &[u8]) -> Result<Vec<Self>> {
        let img = image::load_from_memory(bytes)?.into_rgb8();
        let (w, h) = (img.width() as usize, img.height() as usize);
        let v: Vec<Height> = img.into_raw().chunks(3).map(|c| Height(c[0] as f32 * 256.0 + c[1] as f32 + c[2] as f32 / 256.0 - 32768.0)).collect();
        Ok(resize_tile(&v, w, h))
    }
    fn channels(self) -> [f32; 3] {
        [self.0, 0.0, 0.0]
    }
    fn from_channels(c: [f32; 3]) -> Self {
        Height(c[0])
    }
    /// The Terrarium set has holes at some zooms: black tiles, -32768 m everywhere.
    fn is_void(tile: &[Self]) -> bool {
        tile.iter().all(|h| h.0 <= -32767.0)
    }
}

/// A tile, or when the server has none (or an empty one) there, the quarter of the tile one
/// zoom out that covers it, made bigger; up to four zooms out.
fn load_tile<P: Px>(f: &Fetcher, src: &TileSource, z: u8, x: i64, y: i64) -> Result<Option<Vec<P>>> {
    let own = f.tile(src, z, x, y)?.and_then(|b| P::decode(&b).ok()).filter(|t| !P::is_void(t));
    if own.is_some() {
        return Ok(own);
    }
    let mut up = 1u8;
    while up <= 4 && up <= z {
        let k = 1i64 << up;
        if let Some(parent) = f.tile(src, z - up, x / k, y / k)?.and_then(|b| P::decode(&b).ok()).filter(|t| !P::is_void(t)) {
            let t = TILE as f64;
            let (ox, oy) = (((x % k) as f64) * t / k as f64, ((y % k) as f64) * t / k as f64);
            let mut map: TileMap<P> = HashMap::new();
            map.insert((0, 0), Some(Arc::new(parent)));
            let out = (0..TILE * TILE)
                .map(|i| {
                    let (px, py) = ((i % TILE) as f64 + 0.5, (i / TILE) as f64 + 0.5);
                    // Edge pixels repeat: the parent is the only tile there is.
                    let u = (ox + px / k as f64).clamp(0.5, t - 0.5);
                    let v = (oy + py / k as f64).clamp(0.5, t - 0.5);
                    P::from_channels(sample(&map, TILE, u, v, false))
                })
                .collect();
            return Ok(Some(out));
        }
        up += 1;
    }
    Ok(None)
}

/// A tile at 256 × 256 pixels, whatever size the server sent (some send 512).
fn resize_tile<T: Copy + Default>(v: &[T], w: usize, h: usize) -> Vec<T> {
    let t = TILE as usize;
    if w == t && h == t {
        return v.to_vec();
    }
    let mut out = vec![T::default(); t * t];
    if w == 0 || h == 0 {
        return out;
    }
    for y in 0..t {
        for x in 0..t {
            out[y * t + x] = v[(y * h / t) * w + x * w / t];
        }
    }
    out
}

/// Where the output grid lies on the map: point `(col, row)` is at `x0 + col * dx`,
/// `z0 + row * dz`; `dz` is below 0, so row 0 is the northernmost.
struct Grid {
    cols: usize,
    rows: usize,
    x0: f64,
    dx: f64,
    z0: f64,
    dz: f64,
    /// Samples per side averaged for each point, for pictures with smaller pixels than asked.
    supersample: usize,
    cubic: bool,
}

type TileMap<P> = HashMap<(i64, i64), Option<Arc<Vec<P>>>>;

fn pixel_of<P: Px>(tiles: &TileMap<P>, world: i64, ix: i64, iy: i64) -> P {
    let ix = ix.rem_euclid(world);
    let iy = iy.clamp(0, world - 1);
    match tiles.get(&(ix / TILE, iy / TILE)) {
        Some(Some(t)) => t[((iy % TILE) * TILE + ix % TILE) as usize],
        _ => P::default(),
    }
}

fn cubic_weights(t: f64) -> [f64; 4] {
    // Catmull-Rom.
    let (t2, t3) = (t * t, t * t * t);
    [(-t3 + 2.0 * t2 - t) / 2.0, (3.0 * t3 - 5.0 * t2 + 2.0) / 2.0, (-3.0 * t3 + 4.0 * t2 + t) / 2.0, (t3 - t2) / 2.0]
}

fn sample<P: Px>(tiles: &TileMap<P>, world: i64, u: f64, v: f64, cubic: bool) -> [f32; 3] {
    // Pixel centres sit at half pixels.
    let (u, v) = (u - 0.5, v - 0.5);
    let (i, j) = (u.floor(), v.floor());
    let (a, b) = (u - i, v - j);
    let (i, j) = (i as i64, j as i64);
    let mut sum = [0f64; 3];
    if cubic {
        let (wx, wy) = (cubic_weights(a), cubic_weights(b));
        for (dy, wy) in wy.iter().enumerate() {
            for (dx, wx) in wx.iter().enumerate() {
                let c = pixel_of(tiles, world, i + dx as i64 - 1, j + dy as i64 - 1).channels();
                for k in 0..3 {
                    sum[k] += c[k] as f64 * wx * wy;
                }
            }
        }
    } else {
        for (dy, wy) in [1.0 - b, b].iter().enumerate() {
            for (dx, wx) in [1.0 - a, a].iter().enumerate() {
                let c = pixel_of(tiles, world, i + dx as i64, j + dy as i64).channels();
                for k in 0..3 {
                    sum[k] += c[k] as f64 * wx * wy;
                }
            }
        }
    }
    [sum[0] as f32, sum[1] as f32, sum[2] as f32]
}

/// Downloads the tiles under `grid` and samples them, a band of tile rows at a time so only
/// a few rows of tiles are in memory. `progress` gets the share of tiles done.
fn resample<P: Px>(f: &Fetcher, src: &TileSource, area: &GeoArea, zoom: u8, grid: &Grid, job: &Job, progress: &(dyn Fn(f32) + Sync)) -> Result<Vec<P>> {
    let world = TILE << zoom;
    let s = grid.supersample.max(1);
    let sub = |i: usize| (i as f64 + 0.5) / s as f64 - 0.5;
    // Tile pixel of output point (col, row) with sub-sample offsets; x depends only on the
    // column and y only on the row.
    let u_of = |col: f64| area.pixel(grid.x0 + col * grid.dx, area.size_m / 2.0, zoom)[0];
    let v_of = |row: f64| area.pixel(area.size_m / 2.0, grid.z0 + row * grid.dz, zoom)[1];
    let us: Vec<Vec<f64>> = (0..grid.cols).map(|c| (0..s).map(|k| u_of(c as f64 + sub(k))).collect()).collect();
    let margin = if grid.cubic { 3.0 } else { 2.0 };
    let (umin, umax) = (us.first().map_or(0.0, |u| u[0]) - margin, us.last().map_or(0.0, |u| u[s - 1]) + margin);
    let (tx0, tx1) = ((umin / TILE as f64).floor() as i64, (umax / TILE as f64).floor() as i64);
    let (vmin, vmax) = (v_of(sub(0)) - margin, v_of(grid.rows as f64 - 1.0 + sub(s - 1)) + margin);
    let (ty0, ty1) = ((vmin / TILE as f64).floor().max(0.0) as i64, ((vmax / TILE as f64).floor() as i64).min(world / TILE - 1));
    let total = ((tx1 - tx0 + 1) * (ty1 - ty0 + 1)).max(1) as usize;
    let done = AtomicUsize::new(0);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(PARALLEL).build()?;

    let mut tiles: TileMap<P> = HashMap::new();
    let mut out = vec![P::default(); grid.cols * grid.rows];
    let mut row = 0usize;
    while row < grid.rows {
        job.check()?;
        // The output rows whose middle lies in the same row of tiles.
        let band = (v_of(row as f64) / TILE as f64).floor() as i64;
        let mut end = row + 1;
        while end < grid.rows && (v_of(end as f64) / TILE as f64).floor() as i64 == band {
            end += 1;
        }
        let need0 = (((v_of(row as f64 + sub(0)) - margin) / TILE as f64).floor() as i64).max(ty0);
        let need1 = (((v_of(end as f64 - 1.0 + sub(s - 1)) + margin) / TILE as f64).floor() as i64).min(ty1);
        tiles.retain(|k, _| k.1 >= need0);
        let missing: Vec<(i64, i64)> = (need0..=need1).flat_map(|ty| (tx0..=tx1).map(move |tx| (tx.rem_euclid(world / TILE), ty))).filter(|k| !tiles.contains_key(k)).collect();
        let got: Vec<Result<((i64, i64), Option<Arc<Vec<P>>>)>> = pool.install(|| {
            missing
                .par_iter()
                .map(|&(tx, ty)| {
                    job.check()?;
                    let px = load_tile::<P>(f, src, zoom, tx, ty)?.map(Arc::new);
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    progress(n as f32 / total as f32);
                    Ok(((tx, ty), px))
                })
                .collect()
        });
        for g in got {
            let (k, v) = g?;
            tiles.insert(k, v);
        }
        let tiles = &tiles;
        out[row * grid.cols..end * grid.cols].par_chunks_mut(grid.cols).enumerate().for_each(|(i, line)| {
            let r = (row + i) as f64;
            let vs: Vec<f64> = (0..s).map(|k| v_of(r + sub(k))).collect();
            for (c, o) in line.iter_mut().enumerate() {
                let mut acc = [0f32; 3];
                for v in &vs {
                    for u in &us[c] {
                        let p = sample(tiles, world, *u, *v, grid.cubic);
                        for k in 0..3 {
                            acc[k] += p[k];
                        }
                    }
                }
                let n = (s * s) as f32;
                *o = P::from_channels([acc[0] / n, acc[1] / n, acc[2] / n]);
            }
        });
        row = end;
    }
    Ok(out)
}

/// The satellite picture of the area, `px` pixels per side, row 0 on the north edge.
pub fn satellite(f: &Fetcher, src: &TileSource, area: &GeoArea, px: usize, job: &Job, progress: &(dyn Fn(f32) + Sync)) -> Result<RgbImage> {
    let mpp = area.size_m / px as f64;
    let zoom = zoom_for(area.lat, mpp, src.max_zoom);
    let supersample = (mpp / ground_resolution(area.lat, zoom)).round().clamp(1.0, 4.0) as usize;
    let grid = Grid { cols: px, rows: px, x0: mpp / 2.0, dx: mpp, z0: area.size_m - mpp / 2.0, dz: -mpp, supersample, cubic: false };
    let data: Vec<[u8; 3]> = resample(f, src, area, zoom, &grid, job, progress)?;
    // The pixels as bytes without a second copy: a big picture is gigabytes.
    let mut data = std::mem::ManuallyDrop::new(data);
    let (ptr, len, cap) = (data.as_mut_ptr(), data.len(), data.capacity());
    // SAFETY: `[u8; 3]` is three bytes with the alignment of `u8`, so the buffer holds
    // `len * 3` bytes and was allocated for `cap * 3` of them with the same alignment.
    let data = unsafe { Vec::from_raw_parts(ptr as *mut u8, len * 3, cap * 3) };
    Ok(RgbImage { width: px, height: px, data })
}

/// The zoom of the height tiles for a terrain cell: the data is seldom finer than 10 m.
fn height_zoom(area: &GeoArea, cell: f64) -> u8 {
    zoom_for(area.lat, cell.max(10.0), elevation_source().max_zoom)
}

/// Heights of the area for a terrain of `n` × `n` vertices `cell` metres apart, with
/// `j = 0` on the south edge like [`Heightmap`].
pub fn heights(f: &Fetcher, area: &GeoArea, n: usize, cell: f32, job: &Job, progress: &(dyn Fn(f32) + Sync)) -> Result<Heightmap> {
    let src = elevation_source();
    let zoom = height_zoom(area, cell as f64);
    let c = cell as f64;
    let grid = Grid { cols: n, rows: n, x0: 0.0, dx: c, z0: (n as f64 - 1.0) * c, dz: -c, supersample: 1, cubic: true };
    let data: Vec<Height> = resample(f, &src, area, zoom, &grid, job, progress)?;
    let mut map = Heightmap::new(n, n, cell, 0.0);
    // The rows came north first.
    for (r, line) in data.chunks(n).enumerate() {
        let j = n - 1 - r;
        for (i, h) in line.iter().enumerate() {
            map.set(i, j, h.0.max(LOWEST));
        }
    }
    Ok(map)
}

/// How many tiles the satellite picture and the heights of an area need.
pub fn tile_counts(area: &GeoArea, sat: Option<(&TileSource, usize)>, cell: f32) -> (usize, usize) {
    let count = |zoom: u8| {
        let a = area.pixel(0.0, area.size_m, zoom);
        let b = area.pixel(area.size_m, 0.0, zoom);
        let n = |a: f64, b: f64| ((b / TILE as f64).floor() - (a / TILE as f64).floor() + 1.0).max(1.0) as usize;
        n(a[0], b[0]) * n(a[1], b[1])
    };
    let s = sat.map_or(0, |(src, px)| count(zoom_for(area.lat, area.size_m / px as f64, src.max_zoom)));
    (s, count(height_zoom(area, cell as f64)))
}

/// A picture of the land round a place, `px` pixels per side and `metres_per_px` each, to
/// choose the area on.
pub fn preview(f: &Fetcher, src: &TileSource, lat: f64, lon: f64, metres_per_px: f64, px: usize, job: &Job) -> Result<RgbImage> {
    let area = GeoArea { lat, lon, size_m: metres_per_px * px as f64 };
    satellite(f, src, &area, px, job, &|_| {})
}

// ---- OpenStreetMap

/// Which OpenStreetMap shapes are downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OsmParts {
    pub roads: bool,
    pub railways: bool,
    pub waterways: bool,
    pub water: bool,
    pub forests: bool,
    pub land: bool,
    pub buildings: bool,
}

impl Default for OsmParts {
    fn default() -> Self {
        Self { roads: true, railways: true, waterways: true, water: true, forests: true, land: true, buildings: false }
    }
}

impl OsmParts {
    pub fn any(&self) -> bool {
        self.roads || self.railways || self.waterways || self.water || self.forests || self.land || self.buildings
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum OsmKind {
    Roads,
    Railways,
    Waterways,
    Water,
    Forests,
    Land,
    Buildings,
}

impl OsmKind {
    fn layer(self) -> &'static str {
        match self {
            OsmKind::Roads => "OSM roads",
            OsmKind::Railways => "OSM railways",
            OsmKind::Waterways => "OSM waterways",
            OsmKind::Water => "OSM water",
            OsmKind::Forests => "OSM forests",
            OsmKind::Land => "OSM land use",
            OsmKind::Buildings => "OSM buildings",
        }
    }
    fn wanted(self, p: &OsmParts) -> bool {
        match self {
            OsmKind::Roads => p.roads,
            OsmKind::Railways => p.railways,
            OsmKind::Waterways => p.waterways,
            OsmKind::Water => p.water,
            OsmKind::Forests => p.forests,
            OsmKind::Land => p.land,
            OsmKind::Buildings => p.buildings,
        }
    }
    fn is_area(self) -> bool {
        matches!(self, OsmKind::Water | OsmKind::Forests | OsmKind::Land | OsmKind::Buildings)
    }
}

const RAILWAYS: &str = "rail|light_rail|narrow_gauge|tram|subway|disused|abandoned|preserved";
const WATERWAYS: &str = "river|stream|canal|drain|ditch";
const LANDUSE: &str = "farmland|meadow|grass|residential|industrial|commercial|retail|orchard|vineyard|farmyard|quarry|allotments|cemetery|military|brownfield|landfill";
const NATURAL: &str = "scrub|heath|grassland|wetland|sand|beach|bare_rock|scree|shingle";

fn is_one_of(v: &str, list: &str) -> bool {
    list.split('|').any(|x| x == v)
}

/// What a way or relation with these tags is.
fn classify(tags: &HashMap<String, String>) -> Option<OsmKind> {
    let get = |k: &str| tags.get(k).map(String::as_str);
    if get("building").is_some_and(|v| v != "no") {
        return Some(OsmKind::Buildings);
    }
    if get("natural") == Some("water") || get("waterway") == Some("riverbank") || get("landuse").is_some_and(|v| v == "reservoir" || v == "basin") {
        return Some(OsmKind::Water);
    }
    if get("landuse") == Some("forest") || get("natural") == Some("wood") {
        return Some(OsmKind::Forests);
    }
    if get("landuse").is_some_and(|v| is_one_of(v, LANDUSE)) || get("natural").is_some_and(|v| is_one_of(v, NATURAL)) {
        return Some(OsmKind::Land);
    }
    if get("highway").is_some() && get("area") != Some("yes") {
        return Some(OsmKind::Roads);
    }
    if get("railway").is_some_and(|v| is_one_of(v, RAILWAYS)) {
        return Some(OsmKind::Railways);
    }
    if get("waterway").is_some_and(|v| is_one_of(v, WATERWAYS)) {
        return Some(OsmKind::Waterways);
    }
    None
}

/// The Overpass query for the chosen shapes of the area.
pub fn osm_query(area: &GeoArea, p: &OsmParts) -> String {
    let [s, w, n, e] = area.bbox();
    let b = format!("({s:.7},{w:.7},{n:.7},{e:.7})");
    let mut q = String::from("[out:json][timeout:300];\n(\n");
    let mut add = |filter: &str, relations: bool| {
        q += &format!("  way{filter}{b};\n");
        if relations {
            q += &format!("  relation[\"type\"=\"multipolygon\"]{filter}{b};\n");
        }
    };
    if p.roads {
        add("[\"highway\"]", false);
    }
    if p.railways {
        add(&format!("[\"railway\"~\"^({RAILWAYS})$\"]"), false);
    }
    if p.waterways {
        add(&format!("[\"waterway\"~\"^({WATERWAYS})$\"]"), false);
    }
    if p.water {
        add("[\"natural\"=\"water\"]", true);
        add("[\"waterway\"=\"riverbank\"]", true);
        add("[\"landuse\"~\"^(reservoir|basin)$\"]", true);
    }
    if p.forests {
        add("[\"landuse\"=\"forest\"]", true);
        add("[\"natural\"=\"wood\"]", true);
    }
    if p.land {
        add(&format!("[\"landuse\"~\"^({LANDUSE})$\"]"), true);
        add(&format!("[\"natural\"~\"^({NATURAL})$\"]"), true);
    }
    if p.buildings {
        add("[\"building\"]", true);
    }
    q += ");\nout geom;\n";
    q
}

const OVERPASS: [&str; 2] = ["https://overpass-api.de/api/interpreter", "https://overpass.kumi.systems/api/interpreter"];

/// The Overpass answer (JSON) for the chosen shapes of the area.
pub fn download_osm(f: &Fetcher, area: &GeoArea, p: &OsmParts, job: &Job) -> Result<String> {
    let q = osm_query(area, p);
    let mut errors = Vec::new();
    // The free servers are often busy (429, 502, 503, 504): each is tried again after a pause.
    for round in 0..3u64 {
        for url in OVERPASS {
            job.check()?;
            match f.agent.post(url).send_form(&[("data", &q)]) {
                Ok(r) => return String::from_utf8(Fetcher::read(r)?).context("OpenStreetMap sent an invalid reply"),
                Err(ureq::Error::Status(code @ (429 | 502 | 503 | 504), _)) => errors.push(format!("{url}: {code}")),
                Err(e) => {
                    errors.push(format!("{url}: {e}"));
                }
            }
        }
        job.log(format!("OpenStreetMap is busy, trying again ({})", round + 2));
        for _ in 0..(10 * (round + 1)) {
            job.check()?;
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    bail!("Could not download the OpenStreetMap shapes: {}", errors.join("; "))
}

/// Joins the pieces of the outer edge of a multipolygon into closed rings.
fn join_rings(mut pieces: Vec<Vec<[f64; 2]>>) -> Vec<Vec<[f64; 2]>> {
    let same = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    let mut rings = Vec::new();
    pieces.retain(|p| p.len() >= 2);
    while let Some(mut ring) = pieces.pop() {
        loop {
            if ring.len() >= 4 && same(ring[0], *ring.last().unwrap()) {
                break;
            }
            let end = *ring.last().unwrap();
            let Some(i) = pieces.iter().position(|p| same(p[0], end) || same(*p.last().unwrap(), end)) else { break };
            let mut next = pieces.swap_remove(i);
            if !same(next[0], end) {
                next.reverse();
            }
            ring.extend_from_slice(&next[1..]);
        }
        if ring.len() >= 4 && same(ring[0], *ring.last().unwrap()) {
            rings.push(ring);
        }
    }
    rings
}

/// The part of a polygon inside the square `0..size` (Sutherland-Hodgman).
fn clip_polygon(ring: &[[f64; 2]], size: f64) -> Vec<[f64; 2]> {
    let mut pts = ring.to_vec();
    // Each edge: which axis, the limit, and whether inside is below it.
    for (axis, limit, below) in [(0, 0.0, false), (0, size, true), (1, 0.0, false), (1, size, true)] {
        if pts.is_empty() {
            break;
        }
        let inside = |p: &[f64; 2]| if below { p[axis] <= limit } else { p[axis] >= limit };
        let cut = |a: [f64; 2], b: [f64; 2]| {
            let t = (limit - a[axis]) / (b[axis] - a[axis]);
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
        };
        let mut out = Vec::with_capacity(pts.len() + 4);
        for i in 0..pts.len() {
            let (a, b) = (pts[(i + pts.len() - 1) % pts.len()], pts[i]);
            match (inside(&a), inside(&b)) {
                (true, true) => out.push(b),
                (true, false) => out.push(cut(a, b)),
                (false, true) => {
                    out.push(cut(a, b));
                    out.push(b);
                }
                (false, false) => {}
            }
        }
        pts = out;
    }
    pts
}

/// The parts of a line inside the square `0..size` (Liang-Barsky on each segment).
fn clip_polyline(line: &[[f64; 2]], size: f64) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut keep = true;
        for (p, q) in [(-d[0], a[0]), (d[0], size - a[0]), (-d[1], a[1]), (d[1], size - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    keep = false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if !keep || t0 > t1 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        let p0 = [a[0] + d[0] * t0, a[1] + d[1] * t0];
        let p1 = [a[0] + d[0] * t1, a[1] + d[1] * t1];
        if cur.last().map_or(true, |l| (l[0] - p0[0]).abs() > 1e-6 || (l[1] - p0[1]).abs() > 1e-6) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur = vec![p0];
        }
        cur.push(p1);
        if t1 < 1.0 {
            out.push(std::mem::take(&mut cur));
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out
}

fn geometry(v: &serde_json::Value, area: &GeoArea) -> Vec<[f64; 2]> {
    v.as_array()
        .map(|a| a.iter().filter_map(|p| Some(area.to_map(p.get("lon")?.as_f64()?, p.get("lat")?.as_f64()?))).collect())
        .unwrap_or_default()
}

/// Shape layers from an Overpass answer, in map metres and cut to the map.
pub fn osm_layers(json: &str, area: &GeoArea, p: &OsmParts) -> Result<Vec<ShapeLayer>> {
    let v: serde_json::Value = serde_json::from_str(json).context("OpenStreetMap sent an invalid reply")?;
    if let Some(remark) = v.get("remark").and_then(|r| r.as_str()) {
        if remark.contains("runtime error") {
            bail!("OpenStreetMap: {remark}");
        }
    }
    let size = area.size_m;
    let mut layers: std::collections::BTreeMap<OsmKind, Vec<MapShape>> = Default::default();
    for e in v.get("elements").and_then(|e| e.as_array()).into_iter().flatten() {
        let tags: HashMap<String, String> = e.get("tags").and_then(|t| t.as_object()).map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
        let Some(kind) = classify(&tags) else { continue };
        if !kind.wanted(p) {
            continue;
        }
        let mut attributes: Vec<(String, String)> = tags.into_iter().collect();
        attributes.sort();
        if let Some(id) = e.get("id").and_then(|i| i.as_i64()) {
            attributes.insert(0, ("osm_id".into(), id.to_string()));
        }
        let to_f32 = |pts: Vec<[f64; 2]>| pts.into_iter().map(|p| [p[0] as f32, p[1] as f32]).collect::<Vec<_>>();
        let rings: Vec<Vec<[f64; 2]>> = match e.get("type").and_then(|t| t.as_str()) {
            Some("way") => vec![geometry(&e["geometry"], area)],
            Some("relation") if kind.is_area() => {
                let outer = e.get("members").and_then(|m| m.as_array()).into_iter().flatten().filter(|m| m.get("type").and_then(|t| t.as_str()) == Some("way") && m.get("role").and_then(|r| r.as_str()).map_or(true, |r| r == "outer" || r.is_empty())).map(|m| geometry(&m["geometry"], area)).collect();
                join_rings(outer)
            }
            _ => continue,
        };
        let out = layers.entry(kind).or_default();
        for ring in rings {
            if kind.is_area() {
                let closed = ring.len() >= 4 && ring.first() == ring.last();
                if !closed {
                    continue;
                }
                let mut clipped = clip_polygon(&ring[..ring.len() - 1], size);
                clipped.dedup();
                if clipped.len() >= 3 {
                    out.push(MapShape { kind: ShapeKind::Polygon, points: to_f32(clipped), attributes: attributes.clone() });
                }
            } else {
                for part in clip_polyline(&ring, size) {
                    out.push(MapShape { kind: ShapeKind::Polyline, points: to_f32(part), attributes: attributes.clone() });
                }
            }
        }
    }
    Ok(layers.into_iter().filter(|(_, s)| !s.is_empty()).map(|(k, shapes)| ShapeLayer { name: k.layer().into(), visible: true, shapes }).collect())
}

// ---- Places

/// A place found by name.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Latitude and longitude typed as two numbers, such as `49.2969, 19.9505`.
pub fn parse_coordinates(text: &str) -> Option<(f64, f64)> {
    let parts: Vec<f64> = text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| s.parse().ok()).collect::<Option<_>>()?;
    match parts[..] {
        [lat, lon] if lat.abs() <= 85.0 && lon.abs() <= 180.0 => Some((lat, lon)),
        _ => None,
    }
}

/// Places that match a name (Nominatim), or the coordinates typed.
pub fn search_places(f: &Fetcher, query: &str) -> Result<Vec<Place>> {
    if let Some((lat, lon)) = parse_coordinates(query) {
        return Ok(vec![Place { name: format!("{lat:.5}, {lon:.5}"), lat, lon }]);
    }
    let r = f.agent.get("https://nominatim.openstreetmap.org/search").query("q", query).query("format", "json").query("limit", "8").call().context("Could not search for the place")?;
    let v: serde_json::Value = r.into_json().context("The place search sent an invalid reply")?;
    Ok(v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            Some(Place {
                name: p.get("display_name")?.as_str()?.to_string(),
                lat: p.get("lat")?.as_str()?.parse().ok()?,
                lon: p.get("lon")?.as_str()?.parse().ok()?,
            })
        })
        .collect())
}

// ---- The mask from the shapes

/// A kind of OpenStreetMap shape that can become a surface of the mask. Lines are painted
/// `width` metres wide; areas have a width of 0.
pub struct ShapeClass {
    pub name: &'static str,
    pub width: f32,
}

/// The kinds of shapes, in the order they are painted: land first, then forests and water
/// over it, then the roads and railways on top.
pub const SHAPE_CLASSES: [ShapeClass; 14] = [
    ShapeClass { name: "Fields", width: 0.0 },
    ShapeClass { name: "Meadows and grass", width: 0.0 },
    ShapeClass { name: "Scrub, heath and wetland", width: 0.0 },
    ShapeClass { name: "Built-up land", width: 0.0 },
    ShapeClass { name: "Sand and beaches", width: 0.0 },
    ShapeClass { name: "Rock, scree and quarries", width: 0.0 },
    ShapeClass { name: "Leafy and mixed forests", width: 0.0 },
    ShapeClass { name: "Conifer forests", width: 0.0 },
    ShapeClass { name: "Water", width: 0.0 },
    ShapeClass { name: "Main roads", width: 8.0 },
    ShapeClass { name: "Minor roads", width: 5.0 },
    ShapeClass { name: "Tracks", width: 3.0 },
    ShapeClass { name: "Paths", width: 1.5 },
    ShapeClass { name: "Railways", width: 4.0 },
];

/// Which of [`SHAPE_CLASSES`] a shape with these OpenStreetMap tags is.
pub fn shape_class(tags: &[(String, String)]) -> Option<usize> {
    let get = |k: &str| tags.iter().find(|t| t.0 == k).map(|t| t.1.as_str());
    let one_of = |v: Option<&str>, list: &str| v.is_some_and(|v| is_one_of(v, list));
    if get("building").is_some_and(|v| v != "no") {
        return None;
    }
    if let Some(h) = get("highway") {
        let h = h.trim_end_matches("_link");
        return if is_one_of(h, "motorway|trunk|primary|secondary|tertiary") {
            Some(9)
        } else if is_one_of(h, "unclassified|residential|service|living_street|road|pedestrian") {
            Some(10)
        } else if h == "track" {
            Some(11)
        } else if is_one_of(h, "path|footway|bridleway|cycleway|steps") {
            Some(12)
        } else {
            None
        };
    }
    if one_of(get("railway"), RAILWAYS) {
        return Some(13);
    }
    if get("natural") == Some("water") || get("waterway") == Some("riverbank") || one_of(get("landuse"), "reservoir|basin") {
        return Some(8);
    }
    if get("landuse") == Some("forest") || get("natural") == Some("wood") {
        return Some(if get("leaf_type") == Some("needleleaved") { 7 } else { 6 });
    }
    let landuse = get("landuse");
    let natural = get("natural");
    if one_of(landuse, "farmland|orchard|vineyard|allotments") {
        Some(0)
    } else if one_of(landuse, "meadow|grass") || natural == Some("grassland") || get("leisure") == Some("park") {
        Some(1)
    } else if one_of(natural, "scrub|heath|wetland") {
        Some(2)
    } else if one_of(landuse, "residential|industrial|commercial|retail|farmyard|military|brownfield|cemetery|landfill") {
        Some(3)
    } else if one_of(natural, "sand|beach|shingle") {
        Some(4)
    } else if one_of(natural, "bare_rock|scree") || landuse == Some("quarry") {
        Some(5)
    } else {
        None
    }
}

/// The surface each kind of shape ([`SHAPE_CLASSES`]) becomes, and how wide lines are painted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeMask {
    pub surfaces: Vec<Option<usize>>,
    pub widths: Vec<f32>,
}

impl ShapeMask {
    /// Each kind given to the surface whose name fits it (see [`crate::satgen::natural_surfaces`]).
    pub fn for_layers(layers: &[crate::world::SurfaceLayer]) -> Self {
        use crate::satgen::natural::*;
        let kinds = crate::satgen::natural_surfaces(layers);
        let k = |kind: u8| kinds[kind as usize];
        let surfaces = vec![k(FIELD), k(GRASS), k(PALE_GRASS), None, k(SOIL), k(ROCK), k(FOREST), k(DENSE_FOREST), None, k(PAVED), k(PAVED), k(SOIL), None, k(ROCK)];
        Self { surfaces, widths: SHAPE_CLASSES.iter().map(|c| c.width).collect() }
    }
}

/// Calls `row(y, x0, x1)` for the pixels of each row of an image `w` × `h` inside the ring
/// (in pixels), by the even-odd rule.
fn fill_ring(ring: &[[f32; 2]], w: usize, h: usize, mut row: impl FnMut(usize, usize, usize)) {
    if ring.len() < 3 || w == 0 || h == 0 {
        return;
    }
    let (lo, hi) = ring.iter().fold((f32::MAX, f32::MIN), |m, p| (m.0.min(p[1]), m.1.max(p[1])));
    let (y0, y1) = ((lo - 0.5).ceil().max(0.0) as usize, ((hi - 0.5).floor().max(-1.0) as i64).min(h as i64 - 1));
    let mut xs = Vec::new();
    for y in y0 as i64..=y1 {
        let py = y as f32 + 0.5;
        xs.clear();
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            if (a[1] <= py) != (b[1] <= py) {
                xs.push(a[0] + (py - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            let from = (pair[0] - 0.5).ceil().max(0.0) as i64;
            let to = ((pair[1] - 0.5).floor() as i64).min(w as i64 - 1);
            if from <= to {
                row(y as usize, from as usize, to as usize);
            }
        }
    }
}

/// Calls `px(x, y)` for the pixels within `half` pixels of the line (in pixels).
fn stroke_line(line: &[[f32; 2]], half: f32, w: usize, h: usize, mut px: impl FnMut(usize, usize)) {
    for s in line.windows(2) {
        let (a, b) = (s[0], s[1]);
        let x0 = (a[0].min(b[0]) - half).floor().max(0.0) as usize;
        let y0 = (a[1].min(b[1]) - half).floor().max(0.0) as usize;
        let x1 = ((a[0].max(b[0]) + half).ceil() as i64).min(w as i64 - 1);
        let y1 = ((a[1].max(b[1]) + half).ceil() as i64).min(h as i64 - 1);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let len2 = (ab[0] * ab[0] + ab[1] * ab[1]).max(1e-6);
        for y in y0 as i64..=y1 {
            for x in x0 as i64..=x1 {
                let p = [x as f32 + 0.5 - a[0], y as f32 + 0.5 - a[1]];
                let t = ((p[0] * ab[0] + p[1] * ab[1]) / len2).clamp(0.0, 1.0);
                if (p[0] - ab[0] * t).hypot(p[1] - ab[1] * t) <= half {
                    px(x as usize, y as usize);
                }
            }
        }
    }
}

/// Paints the shapes of `layers` into `mask` (the world's mask size) with the surfaces of `m`,
/// kind by kind in the order of [`SHAPE_CLASSES`]. Kinds without a surface leave the mask.
pub fn mask_from_shapes(w: &World, layers: &[ShapeLayer], m: &ShapeMask, mask: &mut [u8]) {
    let (mw, mh) = (w.mask.width, w.mask.height);
    let size = w.meta.size_m();
    let (sx, sy) = (mw as f32 / size, mh as f32 / size);
    let to_px = |p: &[f32; 2]| [p[0] * sx, (size - p[1]) * sy];
    let count = w.layers.len();
    let mut by_class: Vec<Vec<&MapShape>> = vec![Vec::new(); SHAPE_CLASSES.len()];
    for s in layers.iter().flat_map(|l| l.shapes.iter()) {
        if let Some(c) = shape_class(&s.attributes) {
            by_class[c].push(s);
        }
    }
    for (c, shapes) in by_class.iter().enumerate() {
        let Some(surface) = m.surfaces.get(c).copied().flatten().filter(|s| *s < count) else { continue };
        let v = surface as u8;
        let width = m.widths.get(c).copied().unwrap_or(SHAPE_CLASSES[c].width);
        for s in shapes {
            let pts: Vec<[f32; 2]> = s.points.iter().map(to_px).collect();
            match s.kind {
                ShapeKind::Polygon => fill_ring(&pts, mw, mh, |y, x0, x1| mask[y * mw + x0..=y * mw + x1].fill(v)),
                ShapeKind::Polyline if width > 0.0 => stroke_line(&pts, (width * 0.5 * sx).max(0.5), mw, mh, |x, y| mask[y * mw + x] = v),
                _ => {}
            }
        }
    }
}

// ---- The whole map

/// The kinds of ground of [`crate::satgen::natural_classes`] given to the surfaces of
/// [`crate::world::default_layers`]: dense forest, leafy forest, grass, pale grass, soil,
/// fields, rock, and roads and roofs.
pub fn default_classify() -> SatClassify {
    SatClassify::natural(&crate::world::default_layers())
}

/// The surface mask read from a real satellite photo.
pub fn mask_from_satellite(w: &World, sat: &RgbImage, c: &SatClassify) -> Vec<u8> {
    let (mw, mh) = (w.mask.width, w.mask.height);
    let cls = c.classes(sat, mw, mh);
    let cls = crate::satgen::smooth_classes(&cls, mw, mh, c.smooth);
    crate::satgen::mask_from_classes(w, &cls, c, None)
}

/// What a map of a real place is made of.
#[derive(Clone, Debug, PartialEq)]
pub struct RealMap {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub terrain_size: u32,
    pub cell: f32,
    /// The picture server and the metres per pixel of the picture.
    pub satellite: Option<(TileSource, f32)>,
    pub heights: bool,
    pub osm: Option<OsmParts>,
    /// The surface mask is read from the picture this way.
    pub mask: Option<SatClassify>,
    /// The OpenStreetMap shapes are painted into the mask over what the picture gave.
    pub shape_mask: Option<ShapeMask>,
}

impl RealMap {
    pub fn area(&self) -> GeoArea {
        GeoArea { lat: self.lat, lon: self.lon, size_m: self.terrain_size as f64 * self.cell as f64 }
    }
    /// Pixels per side of the satellite picture.
    pub fn satellite_px(&self) -> Option<usize> {
        self.satellite.as_ref().map(|(_, mpp)| ((self.area().size_m / *mpp as f64).round() as usize).max(256))
    }
}

/// Downloads everything and makes the world. Shapes that cannot be downloaded are left out
/// with a warning in the job log; the picture and heights are needed.
pub fn build_world(m: &RealMap, f: &Fetcher, job: &Job) -> Result<World> {
    let area = m.area();
    let mut meta = WorldMeta::new(&m.name, m.terrain_size, m.cell);
    meta.latitude = m.lat as f32;
    meta.longitude = m.lon as f32;
    meta.geo = Some(area.clone());
    let mut w = World::new(meta, 0.0);
    // Shares of the progress bar: picture, heights, shapes.
    let parts = [m.satellite.is_some() as u8 as f32 * 6.0, m.heights as u8 as f32 * 2.0, m.osm.is_some_and(|o| o.any()) as u8 as f32];
    let sum = parts.iter().sum::<f32>().max(1.0);
    let start = |i: usize| parts[..i].iter().sum::<f32>() / sum;
    let span = |i: usize| parts[i] / sum;

    if m.heights {
        job.stage("Downloading the heights");
        let (s, k) = (start(1), span(1));
        let h = heights(f, &area, m.terrain_size as usize, m.cell, job, &|p| job.set_progress(s + p * k))?;
        w.base = h;
        w.recompose(None);
    }
    if let Some(o) = m.osm.filter(|o| o.any()) {
        job.stage("Downloading the OpenStreetMap shapes");
        job.set_progress(start(2));
        match download_osm(f, &area, &o, job).and_then(|j| osm_layers(&j, &area, &o)) {
            Ok(layers) => {
                job.log(format!("{} shape layers from OpenStreetMap", layers.len()));
                w.shape_layers = layers;
            }
            Err(e) => {
                job.check()?;
                job.warn(format!("{e:#}"));
            }
        }
    }
    if let (Some((src, _)), Some(px)) = (&m.satellite, m.satellite_px()) {
        job.stage("Downloading the satellite picture");
        let (s, k) = (start(0), span(0));
        let img = satellite(f, src, &area, px, job, &|p| job.set_progress(s + p * k))?;
        if let Some(c) = &m.mask {
            job.stage("Reading the surfaces from the satellite picture");
            w.mask.data = mask_from_satellite(&w, &img, c);
        }
        w.satellite = Some(img);
        w.sat_image_replaced();
    }
    if let Some(sm) = &m.shape_mask {
        if !w.shape_layers.is_empty() {
            job.stage("Painting the surfaces of the shapes");
            let mut data = std::mem::take(&mut w.mask.data);
            mask_from_shapes(&w, &w.shape_layers, sm, &mut data);
            w.mask.data = data;
        }
    }
    job.set_progress(1.0);
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::satmod::Adjust;

    #[test]
    fn map_points_and_coordinates_go_both_ways() {
        let a = GeoArea { lat: 49.3, lon: 19.95, size_m: 10240.0 };
        let (lon, lat) = a.to_lonlat(5120.0, 5120.0);
        assert!((lon - 19.95).abs() < 1e-9 && (lat - 49.3).abs() < 1e-9);
        let p = a.to_map(19.99, 49.33);
        let (lon, lat) = a.to_lonlat(p[0], p[1]);
        assert!((lon - 19.99).abs() < 1e-9 && (lat - 49.33).abs() < 1e-9);
        // A kilometre is a kilometre: one minute of latitude is about 1853 m.
        let p = a.to_map(19.95, 49.3 + 1.0 / 60.0);
        assert!((p[1] - 5120.0 - 1853.0).abs() < 15.0, "{}", p[1]);
        let [s, w, n, e] = a.bbox();
        assert!(s < 49.3 && n > 49.3 && w < 19.95 && e > 19.95);
    }

    #[test]
    fn zoom_follows_the_pixel_size() {
        assert_eq!(zoom_for(0.0, 156543.0, 18), 0);
        assert_eq!(zoom_for(50.0, 1.0, 18), 17);
        assert_eq!(zoom_for(50.0, 0.01, 18), 18);
    }

    #[test]
    fn shapes_are_cut_to_the_map() {
        let sq = clip_polygon(&[[-10.0, -10.0], [50.0, -10.0], [50.0, 50.0], [-10.0, 50.0]], 100.0);
        assert_eq!(sq.len(), 4);
        assert!(sq.iter().all(|p| p[0] >= 0.0 && p[1] >= 0.0));
        let parts = clip_polyline(&[[-10.0, 50.0], [50.0, 50.0], [150.0, 50.0], [150.0, 60.0], [50.0, 60.0]], 100.0);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], vec![[0.0, 50.0], [50.0, 50.0], [100.0, 50.0]]);
        assert_eq!(parts[1], vec![[100.0, 60.0], [50.0, 60.0]]);
        let rings = join_rings(vec![vec![[0.0, 0.0], [1.0, 0.0]], vec![[1.0, 1.0], [1.0, 0.0]], vec![[1.0, 1.0], [0.0, 0.0]]]);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 4);
    }

    #[test]
    fn overpass_answer_becomes_shape_layers() {
        let a = GeoArea { lat: 49.3, lon: 19.95, size_m: 2000.0 };
        let p = |x: f64, z: f64| {
            let (lon, lat) = a.to_lonlat(x, z);
            format!("{{\"lat\":{lat},\"lon\":{lon}}}")
        };
        let json = format!(
            r#"{{"elements":[
            {{"type":"way","id":1,"tags":{{"highway":"track"}},"geometry":[{},{}]}},
            {{"type":"way","id":2,"tags":{{"landuse":"forest"}},"geometry":[{},{},{},{}]}},
            {{"type":"relation","id":3,"tags":{{"type":"multipolygon","natural":"water"}},"members":[
               {{"type":"way","role":"outer","geometry":[{},{}]}},{{"type":"way","role":"outer","geometry":[{},{},{}]}}]}},
            {{"type":"way","id":4,"tags":{{"building":"yes"}},"geometry":[{},{},{},{}]}}
            ]}}"#,
            p(100.0, 100.0), p(500.0, 100.0),
            p(200.0, 200.0), p(400.0, 200.0), p(400.0, 400.0), p(200.0, 200.0),
            p(600.0, 600.0), p(800.0, 600.0), p(800.0, 600.0), p(800.0, 800.0), p(600.0, 600.0),
            p(10.0, 10.0), p(20.0, 10.0), p(20.0, 20.0), p(10.0, 10.0),
        );
        let layers = osm_layers(&json, &a, &OsmParts::default()).unwrap();
        let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["OSM roads", "OSM water", "OSM forests"]);
        let road = &layers[0].shapes[0];
        assert_eq!(road.kind, ShapeKind::Polyline);
        assert!((road.points[1][0] - 500.0).abs() < 0.01 && (road.points[1][1] - 100.0).abs() < 0.01);
        assert!(road.attributes.contains(&("highway".into(), "track".into())));
        assert_eq!(layers[1].shapes[0].points.len(), 3);
        assert_eq!(layers[2].shapes[0].kind, ShapeKind::Polygon);
        assert!(osm_query(&a, &OsmParts::default()).contains("way[\"highway\"]("));
    }

    #[test]
    fn the_place_is_kept_in_the_project_and_older_projects_have_none() {
        let mut meta = WorldMeta::new("Real", 64, 4.0);
        assert!(!serde_json::to_string(&meta).unwrap().contains("\"geo\""));
        meta.geo = Some(GeoArea { lat: 49.3, lon: 19.95, size_m: 256.0 });
        let w = World::new(meta, 0.0);
        let d = tempfile::tempdir().unwrap();
        crate::project::save(&w, d.path(), crate::project::SaveParts::ALL).unwrap();
        let back = crate::project::load(d.path()).unwrap();
        assert_eq!(back.meta.geo, w.meta.geo);
    }

    #[test]
    fn real_colours_become_kinds_of_ground() {
        use crate::satgen::natural::*;
        // Dense forest, a meadow, a ripe field and a pale rock, as an Esri photo shows them.
        let colours = [[25u8, 35, 39], [68, 86, 65], [150, 135, 100], [200, 200, 196]];
        let mut img = RgbImage::new(4, 1, [0, 0, 0]);
        for (i, c) in colours.iter().enumerate() {
            img.data[i * 3..i * 3 + 3].copy_from_slice(c);
        }
        assert_eq!(crate::satgen::natural_classes(&img, &Adjust::default(), 4, 1), vec![DENSE_FOREST, GRASS, FIELD, ROCK]);
        // The kinds go to the surfaces of the same name.
        assert_eq!(default_classify().surfaces, [Some(3), Some(2), Some(0), Some(1), Some(4), Some(5), Some(6), Some(7)]);
        let named = |n: &[&str]| n.iter().map(|n| crate::world::SurfaceLayer { name: n.to_string(), texture: String::new(), material: String::new(), color: [0; 3] }).collect::<Vec<_>>();
        let cp = crate::satgen::natural_surfaces(&named(&["cp_grass", "cp_conifer_common1", "cp_broadleaf_dense1", "cp_dirt", "cp_rock", "cp_grass_tall"]));
        assert_eq!(cp, [Some(1), Some(2), Some(0), Some(5), Some(3), None, Some(4), None]);
    }

    #[test]
    fn shapes_become_surfaces_of_the_mask() {
        let tags = |list: &[(&str, &str)]| list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(shape_class(&tags(&[("landuse", "forest"), ("leaf_type", "needleleaved")])), Some(7));
        assert_eq!(shape_class(&tags(&[("natural", "wood")])), Some(6));
        assert_eq!(shape_class(&tags(&[("highway", "primary_link")])), Some(9));
        assert_eq!(shape_class(&tags(&[("highway", "track")])), Some(11));
        assert_eq!(shape_class(&tags(&[("building", "yes"), ("landuse", "farmland")])), None);
        let mut w = World::new(WorldMeta::new("S", 64, 4.0), 0.0);
        w.mask = crate::terrain::MaskMap::new(256, 256, 0);
        let field = MapShape { kind: ShapeKind::Polygon, points: vec![[0.0, 0.0], [128.0, 0.0], [128.0, 256.0], [0.0, 256.0]], attributes: tags(&[("landuse", "farmland")]) };
        let road = MapShape { kind: ShapeKind::Polyline, points: vec![[0.0, 128.0], [256.0, 128.0]], attributes: tags(&[("highway", "track")]) };
        let layer = ShapeLayer { name: "OSM".into(), visible: true, shapes: vec![field, road] };
        let m = ShapeMask::for_layers(&w.layers);
        let (stubble, soil) = (m.surfaces[0].unwrap() as u8, m.surfaces[11].unwrap() as u8);
        let mut data = w.mask.data.clone();
        mask_from_shapes(&w, &[layer], &m, &mut data);
        // The field covers the west half, and the track crosses it in the middle, 3 m wide.
        assert_eq!(data[10 * 256 + 10], stubble);
        assert_eq!(data[10 * 256 + 200], 0);
        assert_eq!(data[128 * 256 + 200], soil);
        assert_eq!(data[124 * 256 + 200], 0);
    }

    #[test]
    fn coordinates_can_be_typed() {
        assert_eq!(parse_coordinates("49.2969, 19.9505"), Some((49.2969, 19.9505)));
        assert_eq!(parse_coordinates("49.2969 19.9505"), Some((49.2969, 19.9505)));
        assert_eq!(parse_coordinates("Zakopane"), None);
    }

    /// Samples a test pattern through the same code as the tiles: the heights of a slope
    /// that rises to the east come out rising to the east, south row first.
    #[test]
    fn resampling_keeps_the_picture_the_right_way_round() {
        let world = TILE << 1;
        let mut tiles: TileMap<Height> = HashMap::new();
        for ty in 0..2 {
            for tx in 0..2 {
                let v: Vec<Height> = (0..TILE * TILE).map(|i| Height(((tx * TILE + i % TILE) * 1000 + (ty * TILE + i / TILE)) as f32)).collect();
                tiles.insert((tx, ty), Some(Arc::new(v)));
            }
        }
        let a = sample(&tiles, world, 300.5, 10.5, false)[0];
        assert_eq!(a, (300 * 1000 + 10) as f32);
        let b = sample(&tiles, world, 300.5, 10.5, true)[0];
        assert!((b - a).abs() < 0.01);
    }
}
