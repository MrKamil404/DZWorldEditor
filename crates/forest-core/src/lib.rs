//! forest-core — silnik generowania lasów dla DayZ (Terrain Builder).
//!
//! Pipeline: maska PNG + heightmapa ASC + wykluczenia GeoJSON
//! -> rozrzut Poisson-disk z proporcjami gatunków
//! -> plik TXT do importu w Terrain Builder.

pub mod export_tb;
pub mod geojson;
pub mod heightmap;
pub mod layers;
pub mod mask;
pub mod model3d;
pub mod odol;
pub mod paa;
pub mod preset;
pub mod saved;
pub mod scatter;
pub mod shp;
pub mod species;
pub mod user_presets;

pub use export_tb::{write_tb_file, write_tb_txt, export_trees_png, export_trees_png_with_layers, export_zones_png, parse_tb_txt, TbImport};
pub use preset::ForestProject;
pub use scatter::{generate, GenStats, PlacedObject};
