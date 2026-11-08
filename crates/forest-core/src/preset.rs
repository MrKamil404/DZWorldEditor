//! Konfiguracja projektu (serializowana do JSON) — most między GUI/CLI a silnikiem.

use crate::mask::Rgb8;
use crate::species::SpeciesDef;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElevationMode {
    /// Elevation = 0 -> obiekty "przyklejone" do terenu przy imporcie (relative).
    RelativeZero,
    /// Elevation = wysokość z heightmapy ASC (import jako absolute).
    AbsoluteSampled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZoneDef {
    /// Kolor strefy na masce (RGB).
    pub color: Rgb8,
    pub label: String,
    /// Docelowa gęstość drzew na hektar.
    pub density_per_ha: f32,
    /// Wagi gatunków w strefie: (indeks do `species`, waga > 0).
    pub species_weights: Vec<(usize, f32)>,
    /// Miks presetów: (nazwa presetu, udział 0..1]. Gdy niepuste —
    /// gęstość/wagi są wynikiem zmieszania tych presetów proporcjonalnie
    /// do udziałów (edytowalne w GUI; edycja ręczna wag czyści miks).
    #[serde(default)]
    pub preset_mix: Vec<(String, f32)>,
}

impl Default for ZoneDef {
    fn default() -> Self {
        Self {
            color: Rgb8([255, 0, 0]),
            label: "Strefa".into(),
            density_per_ha: 220.0,
            species_weights: Vec::new(),
            preset_mix: Vec::new(),
        }
    }
}

/// Obszar rysowany ręcznie (poligon) — alternatywa dla stref z maski.
/// Współrzędne świata w metrach (origin SW), obrys zamknięty automatycznie.
/// Strefa wycinania: obiekty w promieniu `margin_m` od pikseli o kolorze
/// `color` są usuwane PO generowaniu (niezależnie od źródła).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CutZone {
    pub color: Rgb8,
    pub margin_m: f32,
}

/// Inteligentne generowanie: filtr kolorów z warstwy satelitarnej.
/// Drzewa powstają tylko tam, gdzie piksel podkładu pasuje do jednej
/// z próbek (Manhattan <= tolerancja). Gdy `invert`, próbki są IGNOROWANE
/// (wykluczane): drzewa powstają wszędzie indziej na obszarze.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColorFilter {
    /// Próbki referencyjne (RGB) pobrane z podkładu.
    pub samples: Vec<Rgb8>,
    /// Tolerancja dopasowania (suma różnic kanałów).
    #[serde(default = "default_cf_tol")]
    pub tolerance: u32,
    /// Ignoruj te kolory (odwróć filtr): generuj poza próbkami.
    #[serde(default)]
    pub invert: bool,
}

fn default_cf_tol() -> u32 {
    60
}

fn default_mix_scale() -> f64 {
    200.0
}

/// Wpis miksu presetów na obszarze. Stare projekty zapisane jako
/// `["Nazwa", 1.0]` nadal się wczytują.
#[derive(Clone, Debug, Serialize)]
pub struct MixEntry {
    pub name: String,
    pub share: f32,
    /// true = bierze udział w mieszaniu przestrzennym (Perlin / grupa kolorów).
    /// false = generuje się standardowo (równomiernie na całym obszarze).
    #[serde(default = "default_true")]
    pub spatial: bool,
    /// Osobna grupa kolorów tego presetu (tylko gdy `spatial` i mieszanie włączone).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_filter: Option<ColorFilter>,
    /// Gęstość tego presetu (kopiowana przy dodaniu / odświeżeniu miksu).
    #[serde(default)]
    pub density_per_ha: f32,
    /// Wagi gatunków tego presetu (indeksy do `ForestProject.species`).
    #[serde(default)]
    pub species_weights: Vec<(usize, f32)>,
}

impl Default for MixEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            share: 1.0,
            spatial: true,
            color_filter: None,
            density_per_ha: 0.0,
            species_weights: Vec::new(),
        }
    }
}

impl<'de> Deserialize<'de> for MixEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Legacy(String, f32),
            Full {
                name: String,
                share: f32,
                #[serde(default = "default_true")]
                spatial: bool,
                #[serde(default)]
                color_filter: Option<ColorFilter>,
                #[serde(default)]
                density_per_ha: f32,
                #[serde(default)]
                species_weights: Vec<(usize, f32)>,
            },
        }
        match Raw::deserialize(deserializer)? {
            Raw::Legacy(name, share) => Ok(MixEntry {
                name,
                share,
                spatial: true,
                color_filter: None,
                density_per_ha: 0.0,
                species_weights: Vec::new(),
            }),
            Raw::Full {
                name,
                share,
                spatial,
                color_filter,
                density_per_ha,
                species_weights,
            } => Ok(MixEntry {
                name,
                share,
                spatial,
                color_filter,
                density_per_ha,
                species_weights,
            }),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AreaDef {
    /// Czy obszar bierze udział w generowaniu.
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub label: String,
    pub density_per_ha: f32,
    /// Wagi gatunków: (indeks do `species`, waga > 0).
    pub species_weights: Vec<(usize, f32)>,
    /// Pierścień poligonu, >= 3 punkty.
    pub polygon: Vec<[f64; 2]>,
    /// Dziury poligonu (wycięcia) — pierścienie >= 3 punkty. Obiekty nie są
    /// generowane wewnątrz dziur (import SHP z wieloczęściowymi poligonami).
    #[serde(default)]
    pub holes: Vec<Vec<[f64; 2]>>,
    /// Miks presetów — lista wpisów (wstecznie kompatybilna z parami nazwa/udział).
    #[serde(default)]
    pub preset_mix: Vec<MixEntry>,
    /// Mieszanie przestrzenne dodanych presetów (łatki Perlin + osobne grupy kolorów).
    /// Wyłączone = dotychczasowy miks wag (jeden zestaw gatunków na cały obszar).
    #[serde(default)]
    pub mix_spatial: bool,
    /// Skala łat Perlin [m] — większa = większe plamy jednego presetu.
    #[serde(default = "default_mix_scale")]
    pub mix_scale_m: f64,
    /// Indywidualna granica lasu dla tego obszaru (None = użyj globalnej).
    #[serde(default)]
    pub edges: Option<EdgeSettings>,
    /// Inteligentne generowanie: tylko na kolorach podkładu z próbek.
    /// None lub puste próbki = brak filtra (normalne generowanie).
    #[serde(default)]
    pub color_filter: Option<ColorFilter>,
    /// Wycinanie: gdy true, obszar nie generuje drzew — tylko wycina
    /// obiekty leżące wewnątrz poligonu (poza dziurami). Inne opcje są wtedy ukryte.
    #[serde(default)]
    pub cutting: bool,
}

impl Default for AreaDef {
    fn default() -> Self {
        Self {
            enabled: true,
            label: "Obszar".into(),
            density_per_ha: 0.0,
            species_weights: Vec::new(),
            polygon: Vec::new(),
            holes: Vec::new(),
            preset_mix: Vec::new(),
            mix_spatial: false,
            mix_scale_m: default_mix_scale(),
            edges: None,
            color_filter: None,
            cutting: false,
        }
    }
}

/// Pas graniczny lasu — krzewy/podrost sadzone wzdłuż krawędzi stref i obszarów.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeSettings {
    pub enabled: bool,
    /// Szerokość pasa po obu stronach granicy [m].
    pub band_width_m: f64,
    pub density_per_ha: f32,
    pub species_weights: Vec<(usize, f32)>,
    /// Miks presetów (jak w obszarach): gdy zawiera gotowe wpisy, pas
    /// generuje zmieszane gatunki z miksu zamiast ręcznej listy powyżej.
    #[serde(default)]
    pub preset_mix: Vec<MixEntry>,
    /// Wtapianie: gęstość pasa zanika wraz z odległością od granicy
    /// (zamiast równomiernego, twardego pasa).
    #[serde(default = "default_true")]
    pub blend: bool,
    /// Poszarpanie granicy [m] — szum przesuwający efektywną linię lasu,
    /// dzięki czemu brzeg nie jest równy jak od linijki. 0 = wyłączone.
    #[serde(default = "default_jagged")]
    pub jagged_m: f64,
    /// Odległość wtapiania w głąb lasu [m]: gęstość drzew narasta 0 -> pełna
    /// na tej głębokości od krawędzi (otwarte, naturalne obrzeża). 0 = wył.
    #[serde(default = "default_blend_inside")]
    pub blend_inside_m: f64,
}

fn default_jagged() -> f64 {
    25.0
}

fn default_blend_inside() -> f64 {
    40.0
}

impl Default for EdgeSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            band_width_m: 15.0,
            density_per_ha: 160.0,
            // leszczyna, róża, bez, tarnina — indeksy wg posortowanej
            // vanilla_library() (przeliczone 09.2026; przy zmianie biblioteki
            // przeliczyć jak w zone_presets)
            species_weights: vec![(152, 3.0), (154, 3.0), (143, 2.0), (156, 1.0)],
            preset_mix: Vec::new(),
            blend: true,
            jagged_m: 25.0,
            blend_inside_m: 40.0,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Kolejność etapów generowania — konfigurowalna w lewym panelu Ustawień.
/// `Area(i)` oznacza i-ty obszar (generowanie jeśli `cutting==false`, wycinanie jeśli `true`).
/// Wariant `Areas` jest legacy (sprzed per-obszarowej kolejności) i migrowany w `sanitize`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenStep {
    /// Generowanie z maski (strefy kolorów).
    Mask,
    /// Wycinanie po kolorach maski (`cut_zones`).
    #[serde(alias = "color_cut")]
    Cut,
    /// Legacy: zbiorczy wpis "obszary" — rozwijany do `Area(i)` dla każdego obszaru.
    #[serde(alias = "areas")]
    Areas,
    /// Pojedynczy obszar `areas[index]`.
    Area(usize),
}

impl GenStep {
    pub fn label_generic(&self) -> String {
        match self {
            GenStep::Mask => "Maska".into(),
            GenStep::Cut => "Wycinanie (kolory)".into(),
            GenStep::Areas => "Obszary".into(),
            GenStep::Area(i) => format!("Obszar #{i}"),
        }
    }
    /// Etykieta do wyświetlenia w UI — dla `Area(i)` użyj nazwy obszaru jeśli dostępna.
    pub fn display_label(&self, areas: &[AreaDef]) -> String {
        match self {
            GenStep::Mask => "Maska".into(),
            GenStep::Cut => "Wycinanie (kolory)".into(),
            GenStep::Areas => "Obszary".into(),
            GenStep::Area(i) => {
                if let Some(a) = areas.get(*i) {
                    let kind = if a.cutting { "✂" } else { "🌲" };
                    format!("{kind} {} ", a.label)
                } else {
                    format!("Obszar #{i} (brak)")
                }
            }
        }
    }
}

// alias dla kompatybilności
pub type GenPhase = GenStep;

fn default_generation_order() -> Vec<GenStep> {
    vec![GenStep::Mask, GenStep::Cut]
}

fn default_scale_bound() -> f32 {
    1.0
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectPaths {
    #[serde(default)]
    pub mask: Option<String>,
    /// Podkład satelitarny (tylko podgląd, nie wpływa na generowanie).
    #[serde(default)]
    pub satellite: Option<String>,
    #[serde(default)]
    pub heightmap_asc: Option<String>,
    #[serde(default)]
    pub exclusions_geojson: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForestProject {
    pub version: u32,
    /// Rozmiar mapy w metrach (kwadrat), np. 15360 dla Chernarus.
    pub map_size_m: f64,
    /// Offset easting dodawany przy eksporcie TB (klasycznie 200000).
    pub easting_offset: f64,
    pub northing_offset: f64,
    pub seed: u64,

    /// Mnożnik odstępów Poisson-disk (>1 = rzadziej).
    pub spacing_multiplier: f64,
    /// Globalny zakres randomizacji skali stawianych obiektów.
    /// Dla każdego obiektu losowany jest mnożnik z tego zakresu
    /// i mnożony przez skalę wylosowaną z zakresu gatunku.
    /// 1.0..1.0 = brak dodatkowej randomizacji (tylko skala gatunku).
    #[serde(default = "default_scale_bound")]
    pub scale_min: f32,
    #[serde(default = "default_scale_bound")]
    pub scale_max: f32,
    /// Długość fali szumu polan w metrach (0 = wyłączone).
    pub clearing_scale_m: f64,
    /// Siła polan 0..0.95 (jaka część obszaru ma być prześwitem).
    pub clearing_strength: f64,

    pub min_altitude: Option<f64>,
    pub max_altitude: Option<f64>,
    pub max_slope_deg: Option<f64>,
    /// Margines od krawędzi mapy w metrach.
    pub edge_padding_m: f64,

    /// Tolerancja dopasowania koloru maski (suma różnic kanałów).
    pub color_tolerance: u32,
    /// Kolory maski traktowane jako zakazane (np. drogi, woda).
    pub exclusion_colors: Vec<Rgb8>,
    pub elevation_mode: ElevationMode,

    pub species: Vec<SpeciesDef>,
    pub zones: Vec<ZoneDef>,

    /// Włącz/wyłącz źródło: strefy z maski.
    #[serde(default = "default_true")]
    pub use_mask_zones: bool,
    /// Włącz/wyłącz źródło: obszary rysowane (poligony).
    #[serde(default = "default_true")]
    pub use_areas: bool,

    /// Obszary rysowane ręcznie (poligony) — alternatywa/uzupełnienie maski.
    #[serde(default)]
    pub areas: Vec<AreaDef>,

    /// Pas graniczny (krzewy itp.) wzdłuż krawędzi lasu.
    #[serde(default)]
    pub edges: EdgeSettings,

    /// Strefy wycinania po kolorze maski + bufor [m] — usuwają wygenerowane
    /// obiekty PO generowaniu (działa też na obszary rysowane).
    #[serde(default)]
    pub cut_zones: Vec<CutZone>,

    #[serde(default)]
    pub paths: ProjectPaths,

    /// Zaimportowane warstwy z `layers.cfg` (nazwa → kolor RGB).
    /// Używane do eksportu PNG z kolorami warstw zamiast kolorów gatunków.
    #[serde(default)]
    pub layer_library: crate::layers::LayerLibrary,

    /// Przypisanie warstw do gatunków PO MODELU TB (znormalizowanym):
    /// model → nazwa warstwy. Kluczowanie modelem (nie indeksem), żeby
    /// import, dodawanie i usuwanie gatunków nie rozjeżdżało kolorów.
    #[serde(default)]
    pub species_layers: std::collections::HashMap<String, String>,
    /// Przypisanie warstw do GRUP gatunków: grupa → nazwa warstwy.
    /// Podstawa do automatycznego nadawania warstw (import, przycisk
    /// "Zastosuj grupy do gatunków").
    #[serde(default)]
    pub group_layers: std::collections::HashMap<String, String>,

    /// Ustawienia eksportu PNG (rozmiar i kształt kropek).
    #[serde(default)]
    pub png_settings: PngSettings,

    /// Kolejność etapów generowania: maska / wycinanie kolory / pojedyncze obszary.
    #[serde(default = "default_generation_order")]
    pub generation_order: Vec<GenStep>,
}

/// Tryb renderowania PNG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PngMode {
    /// Małe kropki dla każdego drzewa (tryb drzew).
    Trees,
    /// Duże kolorowe plamy reprezentujące strefy lasu (tryb stref).
    Zones,
    /// Identyczny rendering jak podgląd w programie (koło 2px, bez randomizacji).
    Preview,
}

impl Default for PngMode {
    fn default() -> Self {
        Self::Trees
    }
}

/// Kształt kropki w eksporcie PNG.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PngShape {
    Circle,
    Square,
    Diamond,
    Blob,
}

impl Default for PngShape {
    fn default() -> Self {
        Self::Diamond
    }
}

/// Ustawienia renderowania kropek w eksporcie PNG.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct PngSettings {
    /// Tryb renderowania (drzewa vs strefy).
    pub mode: PngMode,
    /// Rozmiar kropki w metrach (domyślnie 1.5m) - tylko dla trybu Trees.
    pub dot_size_m: f64,
    /// Kształt kropki - tylko dla trybu Trees.
    pub shape: PngShape,
    /// Randomizacja rozmiaru (0.0 = brak, 0.5 = ±50%) - tylko dla trybu Trees.
    pub randomize_size: f64,
    /// Randomizacja rotacji (dla Square/Diamond) - tylko dla trybu Trees.
    pub randomize_rotation: bool,
}

impl Default for PngSettings {
    fn default() -> Self {
        Self {
            mode: PngMode::Trees,
            dot_size_m: 3.2,
            shape: PngShape::Blob,
            randomize_size: 0.35,
            randomize_rotation: false,
        }
    }
}

impl Default for ForestProject {
    fn default() -> Self {
        Self {
            version: 1,
            map_size_m: 15360.0,
            easting_offset: 200_000.0,
            northing_offset: 0.0,
            seed: 1337,
            spacing_multiplier: 1.0,
            scale_min: 1.0,
            scale_max: 1.0,
            clearing_scale_m: 350.0,
            clearing_strength: 0.35,
            min_altitude: None,
            max_altitude: None,
            max_slope_deg: Some(35.0),
            edge_padding_m: 16.0,
            color_tolerance: 12,
            exclusion_colors: Vec::new(),
            elevation_mode: ElevationMode::RelativeZero,
            species: crate::species::vanilla_library(),
            zones: Vec::new(),
            use_mask_zones: true,
            use_areas: true,
            cut_zones: Vec::new(),
            areas: Vec::new(),
            edges: EdgeSettings::default(),
            paths: ProjectPaths::default(),
            layer_library: crate::layers::LayerLibrary::default(),
            species_layers: std::collections::HashMap::new(),
            group_layers: std::collections::HashMap::new(),
            png_settings: PngSettings::default(),
            generation_order: default_generation_order(),
        }
    }
}

impl ForestProject {
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serializacja projektu: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Zapis projektu: {e}"))
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Odczyt projektu: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("Parse projektu: {e}"))
    }

    /// Klucz przypisań warstw dla modelu TB (normalizacja jak przy imporcie).
    pub fn layer_key(model: &str) -> String {
        crate::species::normalize_model_name(model)
    }

    /// Uzupełnij `species_layers` wg `group_layers` dla gatunków bez
    /// przypisania. Zwraca liczbę nadanych przypisań. Używane po imporcie
    /// obiektów i przyciskiem "Zastosuj grupy do gatunków".
    pub fn fill_layers_from_groups(&mut self) -> usize {
        let mut n = 0;
        for sp in &self.species {
            let k = Self::layer_key(&sp.model);
            if !self.species_layers.contains_key(&k) {
                if let Some(l) = self.group_layers.get(&sp.group).cloned() {
                    self.species_layers.insert(k, l);
                    n += 1;
                }
            }
        }
        n
    }

    /// Usuwa gatunek i przesuwa wszystkie odwołania do niego (strefy, obszary,
    /// miksy presetów, granice), żeby wyższe indeksy nadal wskazywały te same gatunki.
    pub fn remove_species(&mut self, removed: usize) {
        fn remap(weights: &mut Vec<(usize, f32)>, removed: usize) {
            weights.retain(|(i, _)| *i != removed);
            for (i, _) in weights.iter_mut() {
                if *i > removed {
                    *i -= 1;
                }
            }
        }
        fn remap_mix(mix: &mut [MixEntry], removed: usize) {
            for m in mix {
                remap(&mut m.species_weights, removed);
            }
        }
        fn remap_edges(e: &mut EdgeSettings, removed: usize) {
            remap(&mut e.species_weights, removed);
            remap_mix(&mut e.preset_mix, removed);
        }

        if removed >= self.species.len() {
            return;
        }
        self.species.remove(removed);
        for z in &mut self.zones {
            remap(&mut z.species_weights, removed);
        }
        for a in &mut self.areas {
            remap(&mut a.species_weights, removed);
            remap_mix(&mut a.preset_mix, removed);
            if let Some(e) = a.edges.as_mut() {
                remap_edges(e, removed);
            }
        }
        remap_edges(&mut self.edges, removed);
    }

    /// Usuwa z stref/obszarów odwołania do nieistniejących gatunków.
    pub fn sanitize(&mut self) {
        // Globalny zakres skali: przytnij do sensownych granic i napraw kolejność.
        self.scale_min = self.scale_min.clamp(0.1, 5.0);
        self.scale_max = self.scale_max.clamp(0.1, 5.0);
        if self.scale_min > self.scale_max {
            std::mem::swap(&mut self.scale_min, &mut self.scale_max);
        }
        for s in &mut self.species {
            if s.scale_min > s.scale_max {
                std::mem::swap(&mut s.scale_min, &mut s.scale_max);
            }
        }
        let n = self.species.len();
        for z in &mut self.zones {
            z.species_weights.retain(|(i, w)| *i < n && *w > 0.0);
        }
        self.zones.retain(|z| !z.species_weights.is_empty());
        for a in &mut self.areas {
            // obszary-wycinanie nie mają wag — nie filtrujemy
            if !a.cutting && !a.species_weights.is_empty() {
                a.species_weights.retain(|(i, w)| *i < n && *w > 0.0);
            }
            for e in &mut a.preset_mix {
                e.species_weights.retain(|(i, w)| *i < n && *w > 0.0);
            }
            if a.mix_scale_m <= 0.0 {
                a.mix_scale_m = default_mix_scale();
            }
            a.holes.retain(|h| h.len() >= 3);
        }
        self.areas.retain(|a| a.polygon.len() >= 3);
        if self.edges.enabled {
            // Waga 0 wycisza gatunek, ale go nie usuwa (usunięcie to decyzja
            // użytkownika w UI) — odrzucamy tylko martwe indeksy.
            // Przynajmniej jedna waga > 0 jest wymagana (sprawdza `validate`).
            self.edges.species_weights.retain(|(i, _)| *i < n);
            for e in &mut self.edges.preset_mix {
                e.species_weights.retain(|(i, w)| *i < n && *w > 0.0);
            }
        }
        for a in &mut self.areas {
            if let Some(e) = a.edges.as_mut() {
                if e.enabled {
                    e.species_weights.retain(|(i, _)| *i < n);
                    for m in &mut e.preset_mix {
                        m.species_weights.retain(|(i, w)| *i < n && *w > 0.0);
                    }
                }
            }
        }
        // --- warstwy: kluczowanie modelem — usuń wpisy dla nieistniejących
        // modeli i warstw (porządek po edycji biblioteki/layers.cfg)
        {
            use crate::species::normalize_model_name;
            let models: std::collections::HashSet<String> = self
                .species
                .iter()
                .map(|s| normalize_model_name(&s.model))
                .collect();
            self.species_layers
                .retain(|m, _| models.contains(m));
            if !self.layer_library.layers.is_empty() {
                let layers: std::collections::HashSet<&str> = self
                    .layer_library
                    .layers
                    .iter()
                    .map(|l| l.name.as_str())
                    .collect();
                self.species_layers.retain(|_, l| layers.contains(l.as_str()));
                self.group_layers.retain(|_, l| layers.contains(l.as_str()));
            }
        }
        // --- generation_order: per-obszarowa, dokładna kolejność ---
        // 1) rozwiń legacy Areas -> Area(i) dla każdego obszaru
        let mut expanded: Vec<GenStep> = Vec::new();
        for e in self.generation_order.clone() {
            match e {
                GenStep::Areas => {
                    for i in 0..self.areas.len() {
                        expanded.push(GenStep::Area(i));
                    }
                }
                other => expanded.push(other),
            }
        }
        // 2) usuń duplikaty i niepoprawne indeksy, zachowując kolejność
        let mut seen_mask = false;
        let mut seen_cut = false;
        let mut seen_area: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut clean: Vec<GenStep> = Vec::new();
        for e in expanded {
            match e {
                GenStep::Mask => {
                    if !seen_mask { seen_mask = true; clean.push(e); }
                }
                GenStep::Cut => {
                    if !seen_cut { seen_cut = true; clean.push(e); }
                }
                GenStep::Area(i) => {
                    if i < self.areas.len() && seen_area.insert(i) {
                        clean.push(e);
                    }
                }
                GenStep::Areas => {}
            }
        }
        // 3) dodaj brakujące Mask/Cut
        if !seen_mask { clean.insert(0, GenStep::Mask); }
        if !seen_cut {
            // Cut domyślnie na końcu (po wszystkich obszarach)
            clean.push(GenStep::Cut);
        }
        // 4) dodaj brakujące Area(i)
        for i in 0..self.areas.len() {
            if !seen_area.contains(&i) {
                // wstaw przed Cut jeśli istnieje, inaczej na końcu
                if let Some(pos) = clean.iter().position(|e| matches!(e, GenStep::Cut)) {
                    clean.insert(pos, GenStep::Area(i));
                } else {
                    clean.push(GenStep::Area(i));
                }
            }
        }
        // jeśli brak obszarów, zostaje [Mask, Cut]
        if clean.is_empty() {
            clean = default_generation_order();
        }
        self.generation_order = clean;
    }

    /// Walidacja przed generowaniem; zwraca listę problemów.
    pub fn validate(&self) -> Result<(), String> {
        if self.map_size_m <= 0.0 {
            return Err("Rozmiar mapy musi być > 0".into());
        }
        if !self.use_mask_zones && !self.use_areas {
            return Err(
                "Wszystkie źródła są wyłączone — włącz w '⚙ Źródła generowania' \
                 przynajmniej maskę albo obszary (poligony)."
                    .into(),
            );
        }
        if self.zones.is_empty() && self.areas.is_empty() {
            return Err("Brak stref lasu (dodaj kolory z maski albo narysuj obszary)".into());
        }
        if self.scale_min <= 0.0 || self.scale_max <= 0.0 {
            return Err("Zakres skali musi być > 0".into());
        }
        if self.scale_min > self.scale_max {
            return Err("Zakres skali: minimum nie może być większe od maksimum".into());
        }
        for s in &self.species {
            if s.scale_min > s.scale_max {
                return Err(format!(
                    "Gatunek '{}': minimalna skala nie może być większa od maksymalnej",
                    s.label
                ));
            }
        }
        for (zi, z) in self.zones.iter().enumerate() {
            if z.density_per_ha <= 0.0 {
                return Err(format!("Strefa {}: gęstość musi być > 0", zi + 1));
            }
            if z.species_weights.is_empty() {
                return Err(format!("Strefa '{}': brak gatunków", z.label));
            }
            check_weights(&z.species_weights, self.species.len())
                .map_err(|e| format!("Strefa '{}': {e}", z.label))?;
        }
        for (ai, a) in self.areas.iter().enumerate() {
            if !a.enabled {
                continue;
            }
            if a.polygon.len() < 3 {
                return Err(format!("Obszar {}: poligon wymaga >= 3 punktów", ai + 1));
            }
            // wycinanie: nie wymaga gęstości ani gatunków, tylko poprawnego poligonu
            if a.cutting {
                // sprawdź samoprzecięcia nawet dla wycinania
                for (pi, pt) in a.polygon.iter().enumerate() {
                    if !pt[0].is_finite() || !pt[1].is_finite() {
                        return Err(format!("Obszar {}: punkt {pi} ma nieprawidłowe współrzędne", ai + 1));
                    }
                }
                if let Some(ix) = crate::scatter::find_self_intersection(&a.polygon) {
                    return Err(format!(
                        "Obszar '{}' (wycinanie): obrys przecina sam siebie — odcinki #{}–#{} i #{}–#{} \
                         krzyżują się w punkcie ({:.1}, {:.1}). Zobacz czerwony znacznik na mapie; \
                         usuń lub przesuń wierzchołki tak, aby obrys był prostym wielokątem.",
                        a.label, ix.seg_a, ix.seg_a + 1, ix.seg_b, ix.seg_b + 1, ix.point[0], ix.point[1]
                    ));
                }
                for (hi, hole) in a.holes.iter().enumerate() {
                    if let Some(ix) = crate::scatter::find_self_intersection(hole) {
                        return Err(format!(
                            "Obszar '{}' (wycinanie): dziura {} przecina samą siebie — odcinki #{}–#{} i #{}–#{} \
                             krzyżują się w punkcie ({:.1}, {:.1}). Popraw obrys dziury.",
                            a.label, hi + 1, ix.seg_a, ix.seg_a + 1, ix.seg_b, ix.seg_b + 1, ix.point[0], ix.point[1]
                        ));
                    }
                }
                continue;
            }
            // własna granica obszaru — te same wymagania co globalna
            // (silnik liczy pas nawet dla obszaru bez drzew, więc sprawdzamy
            // przed `continue` dla nieskonfigurowanych)
            if let Some(e) = a.edges.as_ref().filter(|e| e.enabled) {
                check_edge_settings(
                    e,
                    self.species.len(),
                    &format!("Obszar '{}' (własna granica)", a.label),
                )?;
            }
            // obszar nieskonfigurowany (bez presetu) — legalny, generuje 0 obiektów
            if a.species_weights.is_empty() && a.density_per_ha <= 0.0 {
                continue;
            }
            for (pi, pt) in a.polygon.iter().enumerate() {
                if !pt[0].is_finite() || !pt[1].is_finite() {
                    return Err(format!("Obszar {}: punkt {pi} ma nieprawidłowe współrzędne", ai + 1));
                }
            }
            let (min_x, min_y, max_x, max_y) = crate::scatter::bbox_of(&a.polygon);
            if max_x <= 0.0 || max_y <= 0.0 || min_x >= self.map_size_m || min_y >= self.map_size_m {
                return Err(format!(
                    "Obszar '{}' leży całkowicie poza mapą (0–{:.0} m) — sprawdź układ \
                     współrzędnych i offset easting albo wyłącz ten obszar.",
                    a.label, self.map_size_m
                ));
            }
            if let Some(ix) = crate::scatter::find_self_intersection(&a.polygon) {
                return Err(format!(
                    "Obszar '{}': obrys przecina sam siebie — odcinki #{}–#{} i #{}–#{} \
                     krzyżują się w punkcie ({:.1}, {:.1}). Zobacz czerwony znacznik na mapie; \
                     usuń lub przesuń wierzchołki tak, aby obrys był prostym wielokątem.",
                    a.label,
                    ix.seg_a,
                    ix.seg_a + 1,
                    ix.seg_b,
                    ix.seg_b + 1,
                    ix.point[0],
                    ix.point[1]
                ));
            }
            for (hi, hole) in a.holes.iter().enumerate() {
                if let Some(ix) = crate::scatter::find_self_intersection(hole) {
                    return Err(format!(
                        "Obszar '{}': dziura {} przecina samą siebie — odcinki #{}–#{} i #{}–#{} \
                         krzyżują się w punkcie ({:.1}, {:.1}). Popraw obrys dziury.",
                        a.label,
                        hi + 1,
                        ix.seg_a,
                        ix.seg_a + 1,
                        ix.seg_b,
                        ix.seg_b + 1,
                        ix.point[0],
                        ix.point[1]
                    ));
                }
            }
            if a.density_per_ha <= 0.0 {
                return Err(format!("Obszar '{}': gęstość musi być > 0", a.label));
            }
            if a.species_weights.is_empty() {
                return Err(format!("Obszar '{}': brak gatunków", a.label));
            }
            check_weights(&a.species_weights, self.species.len())
                .map_err(|e| format!("Obszar '{}': {e}", a.label))?;
        }
        if self.edges.enabled {
            check_edge_settings(&self.edges, self.species.len(), "Granica lasu")?;
        }
        Ok(())
    }
}

fn check_weights(weights: &[(usize, f32)], species_len: usize) -> Result<(), String> {
    for (i, _) in weights {
        if *i >= species_len {
            return Err(format!("wskazuje nieistniejący gatunek #{}", i));
        }
    }
    Ok(())
}

fn check_edge_settings(
    e: &EdgeSettings,
    species_len: usize,
    ctx: &str,
) -> Result<(), String> {
    if e.band_width_m <= 0.0 {
        return Err(format!("{ctx}: szerokość pasa musi być > 0"));
    }
    if e.density_per_ha <= 0.0 {
        return Err(format!("{ctx}: gęstość musi być > 0"));
    }
    // gatunki z ręcznej listy LUB z gotowego wpisu miksu presetów
    let manual_ok =
        !e.species_weights.is_empty() && e.species_weights.iter().any(|(_, w)| *w > 0.0);
    let mix_ok = e
        .preset_mix
        .iter()
        .any(|m| m.density_per_ha > 0.0 && !m.species_weights.is_empty());
    if !(manual_ok || mix_ok) {
        return Err(format!(
            "{ctx}: brak gatunków (ustaw wagę > 0 dla przynajmniej jednego albo dodaj preset)"
        ));
    }
    if manual_ok {
        check_weights(&e.species_weights, species_len)
            .map_err(|er| format!("{ctx}: {er}"))?;
    }
    for m in &e.preset_mix {
        check_weights(&m.species_weights, species_len)
            .map_err(|er| format!("{ctx} (miks '{0}'): {er}", m.name))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_project_serializes_roundtrip() {
        let p = ForestProject::default();
        let json = serde_json::to_string(&p).unwrap();
        let back: ForestProject = serde_json::from_str(&json).unwrap();
        assert_eq!(back.map_size_m, 15360.0);
        assert_eq!(back.easting_offset, 200000.0);
        assert!(!back.species.is_empty());
    }

    #[test]
    fn save_and_load_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proj.json");
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            color: Rgb8([0, 128, 0]),
            label: "Las mieszany".into(),
            ..Default::default()
        });
        p.save(&path).unwrap();
        let q = ForestProject::load(&path).unwrap();
        assert_eq!(q.zones.len(), 1);
        assert_eq!(q.zones[0].color, Rgb8([0, 128, 0]));
    }

    #[test]
    fn sanitize_drops_dead_references() {
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            species_weights: vec![(0, 1.0), (9999, 2.0), (1, 0.0)],
            ..Default::default()
        });
        p.sanitize();
        assert_eq!(p.zones[0].species_weights, vec![(0, 1.0)]);
    }

    #[test]
    fn species_scale_inversion_is_repaired_or_rejected() {
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            species_weights: vec![(0, 1.0)],
            ..Default::default()
        });
        p.species[0].scale_min = 1.5;
        p.species[0].scale_max = 1.0;
        assert!(p.validate().unwrap_err().contains("skala"));
        p.sanitize();
        assert_eq!((p.species[0].scale_min, p.species[0].scale_max), (1.0, 1.5));
        assert!(p.validate().is_ok());
    }

    #[test]
    fn remove_species_remaps_every_reference() {
        let weights = vec![(0, 1.0), (1, 2.0), (2, 3.0)];
        let expected = vec![(0, 1.0), (1, 3.0)];
        let mix = vec![MixEntry {
            species_weights: weights.clone(),
            ..Default::default()
        }];
        let edges = EdgeSettings {
            species_weights: weights.clone(),
            preset_mix: mix.clone(),
            ..Default::default()
        };
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            species_weights: weights.clone(),
            ..Default::default()
        });
        p.areas.push(AreaDef {
            species_weights: weights.clone(),
            preset_mix: mix.clone(),
            edges: Some(edges.clone()),
            ..Default::default()
        });
        p.edges = edges;
        let n = p.species.len();
        let kept_model = p.species[2].model.clone();

        p.remove_species(1);

        assert_eq!(p.species.len(), n - 1);
        assert_eq!(p.species[1].model, kept_model);
        assert_eq!(p.zones[0].species_weights, expected);
        let a = &p.areas[0];
        assert_eq!(a.species_weights, expected);
        assert_eq!(a.preset_mix[0].species_weights, expected);
        let own = a.edges.as_ref().unwrap();
        assert_eq!(own.species_weights, expected);
        assert_eq!(own.preset_mix[0].species_weights, expected);
        assert_eq!(p.edges.species_weights, expected);
        assert_eq!(p.edges.preset_mix[0].species_weights, expected);
    }

    #[test]
    fn validate_catches_empty_zones() {
        let p = ForestProject::default();
        assert!(p.validate().is_err());
    }

    #[test]
    fn global_scale_defaults_to_neutral() {
        let p = ForestProject::default();
        assert_eq!((p.scale_min, p.scale_max), (1.0, 1.0));
        // stare projekty bez tych pól wczytują się jako neutralne
        let legacy: ForestProject = serde_json::from_str(
            r#"{"version":1,"map_size_m":1000.0,"easting_offset":200000.0,
                "northing_offset":0.0,"seed":1,"spacing_multiplier":1.0,
                "clearing_scale_m":0.0,"clearing_strength":0.0,
                "min_altitude":null,"max_altitude":null,"max_slope_deg":null,
                "edge_padding_m":0.0,"color_tolerance":12,"exclusion_colors":[],
                "elevation_mode":"relative_zero","species":[],"zones":[]}"#,
        )
        .unwrap();
        assert_eq!((legacy.scale_min, legacy.scale_max), (1.0, 1.0));
    }

    #[test]
    fn sanitize_repairs_scale_range() {
        let mut p = ForestProject::default();
        p.scale_min = 1.5;
        p.scale_max = 0.8;
        p.sanitize();
        assert_eq!((p.scale_min, p.scale_max), (0.8, 1.5));
        p.scale_min = -2.0;
        p.scale_max = 99.0;
        p.sanitize();
        assert!((p.scale_min - 0.1).abs() < 1e-6);
        assert!((p.scale_max - 5.0).abs() < 1e-6);
    }

    #[test]
    fn edge_zero_weight_survives_sanitize_but_all_zero_fails_validate() {
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            color: Rgb8([0, 128, 0]),
            label: "Las".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            preset_mix: Vec::new(),
        });
        p.edges.enabled = true;
        p.edges.species_weights = vec![(0, 0.0), (1, 2.0)];
        p.sanitize();
        // waga 0 zostaje (wyciszony, ale widoczny w UI)
        assert_eq!(p.edges.species_weights, vec![(0, 0.0), (1, 2.0)]);
        assert!(p.validate().is_ok());
        // same zera = brak udziału w losowaniu -> błąd zamiast NaN w silniku
        p.edges.species_weights = vec![(0, 0.0), (1, 0.0)];
        let err = p.validate().unwrap_err();
        assert!(err.contains("Granica lasu"), "{err}");
    }

    #[test]
    fn validate_rejects_area_own_edge_without_species() {
        // regresja: własna granica bez gatunków powodowała panikę silnika
        // (scatter.rs pick_species na pustej liście) zamiast czytelnego błędu
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            color: Rgb8([0, 128, 0]),
            label: "Las".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            preset_mix: Vec::new(),
        });
        p.areas.push(AreaDef {
            enabled: true,
            label: "Wycinka".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            polygon: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            edges: Some(EdgeSettings {
                enabled: true,
                band_width_m: 15.0,
                density_per_ha: 80.0,
                species_weights: Vec::new(),
                ..Default::default()
            }),
            ..Default::default()
        });
        let err = p.validate().unwrap_err();
        assert!(err.contains("własna granica"), "{err}");

        // same zera zamiast pustej listy — ten sam błąd
        p.areas[0].edges.as_mut().unwrap().species_weights = vec![(0, 0.0)];
        let err = p.validate().unwrap_err();
        assert!(err.contains("własna granica"), "{err}");
    }

    #[test]
    fn default_edge_points_at_shrubs_not_pines() {
        // regresja: zahardkodowane indeksy wskazywały sosny po rozrośnięciu
        // biblioteki — granica ma sadzić krzewy
        let lib = crate::species::vanilla_library();
        let e = EdgeSettings::default();
        let models: Vec<&str> = e
            .species_weights
            .iter()
            .map(|(i, _)| lib[*i].model.as_str())
            .collect();
        assert_eq!(
            models,
            vec![
                "b_corylusAvellana_2s",
                "b_rosaCanina_2s",
                "b_sambucusNigra_2s",
                "b_prunusSpinosa_2s"
            ]
        );
    }

    #[test]
    fn edge_preset_mix_satisfies_validation_without_manual_species() {
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            color: Rgb8([0, 128, 0]),
            label: "Las".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            preset_mix: Vec::new(),
        });
        p.edges.enabled = true;
        p.edges.species_weights.clear();
        p.edges.preset_mix = vec![MixEntry {
            name: "Miks".into(),
            share: 1.0,
            spatial: false,
            color_filter: None,
            density_per_ha: 150.0,
            species_weights: vec![(0, 1.0)],
        }];
        assert!(p.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_scale_range() {
        // strefa potrzebna, żeby walidacja doszła do sprawdzenia skali
        let mut p = ForestProject::default();
        p.zones.push(ZoneDef {
            color: Rgb8([0, 128, 0]),
            label: "Las".into(),
            density_per_ha: 100.0,
            species_weights: vec![(0, 1.0)],
            preset_mix: Vec::new(),
        });
        assert!(p.validate().is_ok());
        p.scale_min = 0.0;
        let err = p.validate().unwrap_err();
        assert!(err.contains("skali"), "{err}");
        p.scale_min = 1.5;
        p.scale_max = 1.0;
        let err = p.validate().unwrap_err();
        assert!(err.contains("skali"), "{err}");
    }

    #[test]
    fn mix_entry_legacy_pair_and_full_object() {
        let legacy: Vec<MixEntry> =
            serde_json::from_str(r#"[["Sosna", 1.0], ["Brzoza", 0.5]]"#).unwrap();
        assert_eq!(legacy.len(), 2);
        assert_eq!(legacy[0].name, "Sosna");
        assert_eq!(legacy[0].share, 1.0);
        assert!(legacy[0].spatial);
        assert!(legacy[0].species_weights.is_empty());
        assert_eq!(legacy[1].name, "Brzoza");

        let full: Vec<MixEntry> = serde_json::from_str(
            r#"[{"name":"Sosna","share":1.0,"spatial":false,"density_per_ha":180.0,"species_weights":[[0,2.0]]}]"#,
        )
        .unwrap();
        assert_eq!(full[0].name, "Sosna");
        assert!(!full[0].spatial);
        assert_eq!(full[0].density_per_ha, 180.0);
        assert_eq!(full[0].species_weights, vec![(0, 2.0)]);

        let json = serde_json::to_string(&full).unwrap();
        let back: Vec<MixEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].name, "Sosna");
        assert!(!back[0].spatial);
    }

    #[test]
    fn area_mix_spatial_roundtrip() {
        let mut p = ForestProject::default();
        p.areas.push(AreaDef {
            label: "Las".into(),
            mix_spatial: true,
            mix_scale_m: 150.0,
            preset_mix: vec![MixEntry {
                name: "Sosna".into(),
                share: 1.0,
                spatial: true,
                density_per_ha: 200.0,
                species_weights: vec![(0, 1.0)],
                ..Default::default()
            }],
            polygon: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]],
            ..Default::default()
        });
        let json = serde_json::to_string(&p).unwrap();
        let q: ForestProject = serde_json::from_str(&json).unwrap();
        assert!(q.areas[0].mix_spatial);
        assert!((q.areas[0].mix_scale_m - 150.0).abs() < 1e-9);
        assert_eq!(q.areas[0].preset_mix[0].name, "Sosna");
        assert_eq!(q.areas[0].preset_mix[0].species_weights, vec![(0, 1.0)]);
    }
}