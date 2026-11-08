//! Definicje gatunków drzew/krzewów + wbudowana biblioteka modeli vanilla DayZ
//! + pogrupowane presety stref roślinności.
//!
//! `model` to nazwa wpisu w Template Library Terrain Buildera — domyślnie
//! nazwa pliku .p3d bez ścieżki i rozszerzenia (konwencja Chernarus:
//! dz\plants\tree\*.p3d oraz dz\plants\bush\*.p3d).

use serde::{Deserialize, Serialize};

use crate::mask::Rgb8;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeciesDef {
    /// Nazwa wyświetlana (np. "Brzoza wysoka").
    pub label: String,
    /// Nazwa modelu dla Terrain Buildera (bez .p3d).
    pub model: String,
    pub scale_min: f32,
    pub scale_max: f32,
    /// Maksymalny losowy przechył pitch/roll w stopniach.
    pub tilt_max_deg: f32,
    /// Grupa (do pogrupowanego widoku w UI), np. "Liściaste".
    #[serde(default)]
    pub group: String,
    /// Indywidualny kolor gatunku (None = automatyczny z palety).
    #[serde(default)]
    pub color: Option<crate::mask::Rgb8>,
}

impl SpeciesDef {
    pub fn new(label: &str, model: &str, scale_min: f32, scale_max: f32) -> Self {
        Self {
            label: label.to_string(),
            model: model.to_string(),
            scale_min,
            scale_max,
            tilt_max_deg: 2.0,
            group: String::new(),
            color: None,
        }
    }

    pub fn grouped(label: &str, model: &str, smin: f32, smax: f32, group: &str) -> Self {
        Self {
            group: group.to_string(),
            ..Self::new(label, model, smin, smax)
        }
    }

    /// Efektywny kolor podglądu gatunku.
    pub fn effective_color(&self, index: usize) -> [u8; 3] {
        match self.color {
            Some(c) => c.0,
            None => species_preview_color(index),
        }
    }
}

/// Normalizacja nazwy modelu do porównań: bez cudzysłowu i spacji,
/// bez rozszerzenia `.p3d`, sama nazwa pliku bez ścieżki, małe litery.
/// Dzięki temu import dogaduje się z plikami edytowanymi ręcznie
/// (`"T_X.P3D"`, `dz\plants\tree\t_x.p3d`, …).
pub fn normalize_model_name(s: &str) -> String {
    let t = s.trim().trim_matches('"').trim();
    let t = t
        .strip_suffix(".p3d")
        .or_else(|| t.strip_suffix(".P3D"))
        .or_else(|| t.strip_suffix(".P3d"))
        .unwrap_or(t);
    let base = t.rsplit(['/', '\\']).next().unwrap_or(t);
    base.to_lowercase()
}

/// Indeks gatunku o danej nazwie modelu (dopasowanie znormalizowane).
pub fn find_species_in(species: &[SpeciesDef], model: &str) -> Option<usize> {
    let n = normalize_model_name(model);
    if n.is_empty() {
        return None;
    }
    species
        .iter()
        .position(|s| normalize_model_name(&s.model) == n)
}

/// Wbudowana biblioteka roślinności vanilla DayZ — WSZYSTKIE nazwy zweryfikowane
/// względem P:\DZ\plants*, P:\DZ\plants_bliss i P:\DZ\plants_sakhal
/// (żywe t_*/b_* oraz martwe drewno d_* — pniaki i wykroty).
/// Nazwy muszą się zgadzać z wpisami w Template Library użytkownika —
/// można edytować w GUI.
///
/// UWAGA: lista jest na końcu sortowana (grupa, etykieta), więc indeksy
/// używane przez `zone_presets()` trzeba przeliczyć po każdej zmianie —
/// nowe gatunki dopisuj na końcu funkcji (przed sortowaniem).
pub fn vanilla_library() -> Vec<SpeciesDef> {
    const L: &str = "Liściaste";
    const I: &str = "Iglaste";
    const K: &str = "Krzewy";
    const B: &str = "Bliss (lato)";
    const S: &str = "Sakhal (zima/mrok)";

    let mut v = Vec::new();
    // UWAGA: kolejność wpisów nie ma znaczenia (sortowanie na końcu), ale po
    // każdej zmianie trzeba przeliczyć indeksy w zone_presets() — nowe wpisy
    // dopisuj na końcu.
    let mut t = |label: &str, model: &str, smin: f32, smax: f32, group: &str| {
        v.push(SpeciesDef::grouped(label, model, smin, smax, group));
    };

    // --- drzewa liściaste (dz\plants\tree) ---
    t("Brzoza młoda", "t_BetulaPendula_1s", 0.85, 1.15, L);
    t("Brzoza", "t_BetulaPendula_2f", 0.9, 1.15, L);
    t("Brzoza wysoka", "t_BetulaPendula_3f", 0.9, 1.1, L);
    t("Dąb młody", "t_quercusRobur_1f", 0.9, 1.1, L);
    t("Dąb", "t_quercusRobur_2f", 0.9, 1.1, L);
    t("Dąb wysoki", "t_quercusRobur_3f", 0.95, 1.05, L);
    t("Buk", "t_FagusSylvatica_2f", 0.9, 1.1, L);
    t("Buk wysoki", "t_FagusSylvatica_3f", 0.95, 1.05, L);
    t("Jesion", "t_FraxinusExcelsior_2f", 0.9, 1.1, L);
    t("Modrzew", "t_LarixDecidua_2f", 0.9, 1.1, I);
    t("Robinia (akacja)", "t_robiniaPseudoacacia_2f", 0.9, 1.1, L);

    // --- drzewa iglaste ---
    t("Świerk młody", "t_PiceaAbies_1s", 0.85, 1.15, I);
    t("Świerk", "t_PiceaAbies_2f", 0.9, 1.15, I);
    t("Świerk wysoki", "t_PiceaAbies_3f", 0.95, 1.05, I);
    t("Sosna młoda", "t_PinusSylvestris_1s", 0.85, 1.15, I);
    t("Sosna", "t_PinusSylvestris_2f", 0.9, 1.15, I);
    t("Sosna wysoka", "t_PinusSylvestris_3f", 0.95, 1.05, I);

    // --- krzewy (dz\plants\bush) ---
    t("Leszczyna", "b_corylusAvellana_2s", 0.85, 1.2, K);
    t("Bez czarny", "b_sambucusNigra_2s", 0.85, 1.2, K);
    t("Róża dzika", "b_rosaCanina_2s", 0.85, 1.2, K);
    t("Tarnina", "b_prunusSpinosa_2s", 0.85, 1.2, K);
    t("Głóg", "b_crataegusLaevigata_2s", 0.85, 1.2, K);
    t("Brzoza karłowata", "b_betulaHumilis_1s", 0.85, 1.2, K);

    // --- Bliss / Livonia — warianty letnie (dz\plants_bliss) ---
    t("Klon", "t_acer_2s_summer", 0.9, 1.1, B);
    t("Brzoza E młoda", "t_BetulaPendulaE_1s_summer", 0.85, 1.15, B);
    t("Brzoza E", "t_BetulaPendulaE_2f_summer", 0.9, 1.15, B);
    t("Brzoza E wysoka", "t_BetulaPendulaE_3f_summer", 0.9, 1.1, B);
    t("Karagana", "b_caraganaArborescens_2s_summer", 0.85, 1.2, B);
    t("Leszczyna syberyjska", "b_corylusHeterophylla_2s_summer", 0.85, 1.2, B);
    t("Trzcina", "b_phragmitesAustralis_summer", 0.8, 1.3, B);
    t("Orzech włoski", "t_juglansRegia_3s_summer", 0.9, 1.1, B);
    t("Jabłoń dzika", "t_malusDomestica_3s_summer", 0.85, 1.15, B);
    t("Grusza dzika", "t_pyrusCommunis_3s_summer", 0.85, 1.15, B);
    t("Wierzba biała", "t_salixAlba_2sb_summer", 0.9, 1.15, B);
    t("Jarzębina", "t_sorbus_2s_summer", 0.85, 1.15, B);
    t("Topola czarna", "t_populusNigra_3sb_summer", 0.9, 1.1, B);

    // --- Sakhal — warianty mroczne/zimowe/jesienne (dz\plants_sakhal) ---
    t("Świerk mroczny młody", "t_PiceaAbies_1s_dark", 0.85, 1.15, S);
    t("Świerk mroczny", "t_PiceaAbies_2f_dark", 0.9, 1.15, S);
    t("Świerk zamrożony młody", "t_PiceaAbies_1s_frozen", 0.85, 1.15, S);
    t("Świerk zamrożony", "t_PiceaAbies_2f_frozen", 0.9, 1.15, S);
    t("Brzoza zimowa", "t_BetulaPendula_2f_winter", 0.9, 1.15, S);
    t("Brzoza zimowa wysoka", "t_BetulaPendula_3f_winter", 0.9, 1.1, S);
    t("Brzoza jesienna", "t_BetulaPendula_2f_latefall", 0.9, 1.15, S);
    t("Topola biała (jesień)", "t_populusAlba_2s_latefall", 0.9, 1.15, S);

    // --- Bliss — uzupełniające (dz\plants_bliss) ---
    t("Buk młody (lato)", "t_FagusSylvatica_1s_summer", 0.85, 1.15, B);
    t("Świerk krzewiasty (lato)", "b_PiceaAbies_1f_summer", 0.85, 1.2, B);
    t("Modrzew młody (lato)", "t_LarixDecidua_1s_summer", 0.85, 1.15, B);
    t("Sosna 1f (lato)", "t_PinusSylvestris_1f_summer", 0.85, 1.15, B);
    t("Orzech włoski 2s", "t_juglansRegia_2s_summer", 0.85, 1.15, B);
    t("Grusza dzika 2s", "t_pyrusCommunis_2s_summer", 0.85, 1.15, B);

    // --- Sakhal — uzupełniające (dz\plants_sakhal) ---
    t("Świerk krzewiasty mroczny", "b_PiceaAbies_1f_dark", 0.85, 1.2, S);
    t("Świerk krzewiasty zamrożony", "b_PiceaAbies_1f_frozen", 0.85, 1.2, S);
    t("Brzoza karłowata zimowa", "b_betulaNana_1s_winter", 0.85, 1.2, S);
    t("Topola biała młoda (jesień)", "t_populusAlba_1f_latefall", 0.85, 1.15, S);
    t("Karagana bezlistna", "b_caraganaArborescens_2s_leafless", 0.85, 1.2, S);

    // --- AUTO: pozostałe modele z P:\DZ\plants* (231) + pniaki i wykroty d_* (46) ---
    t("Brzoza 1f", "b_BetulaPendula_1f", 0.85, 1.15, K);
    t("Brzoza 1f (jesień)", "b_BetulaPendula_1f_latefall", 0.85, 1.15, S);
    t("Brzoza 1f (zima)", "b_BetulaPendula_1f_winter", 0.85, 1.15, S);
    t("Buk 1f", "b_FagusSylvatica_1f", 0.85, 1.15, K);
    t("Buk 1f (lato)", "b_FagusSylvatica_1f_summer", 0.85, 1.15, B);
    t("Świerk 1f", "b_PiceaAbies_1f", 0.85, 1.15, K);
    t("Świerk 1f (zima)", "b_PiceaAbies_1f_winter", 0.85, 1.15, S);
    t("Świerk 1fb", "b_PiceaAbies_1fb", 0.85, 1.15, K);
    t("Świerk 1fb (mrok)", "b_PiceaAbies_1fb_dark", 0.85, 1.15, S);
    t("Świerk 1fb (mróz)", "b_PiceaAbies_1fb_frozen", 0.85, 1.15, S);
    t("Świerk 1fb (lato)", "b_PiceaAbies_1fb_summer", 0.85, 1.15, B);
    t("Świerk 1fb (zima)", "b_PiceaAbies_1fb_winter", 0.85, 1.15, S);
    t("Brzoza karłowata 1s (jesień)", "b_betulaHumilis_1s_latefall", 0.85, 1.15, S);
    t("Brzoza karłowata 1s (zima)", "b_betulaHumilis_1s_winter", 0.85, 1.15, S);
    t("Brzoza nana 1s (jesień)", "b_betulaNana_1s_latefall", 0.85, 1.15, S);
    t("Brzoza nana 1s (lato)", "b_betulaNana_1s_summer", 0.85, 1.15, B);
    t("Karagana 2s (zima)", "b_caraganaArborescens_2s_winter", 0.85, 1.2, S);
    t("Leszczyna 1f", "b_corylusAvellana_1f", 0.85, 1.15, K);
    t("Leszczyna syberyjska 1s (lato)", "b_corylusHeterophylla_1s_summer", 0.85, 1.15, B);
    t("Głóg 1s", "b_crataegusLaevigata_1s", 0.85, 1.15, K);
    t("Krzew nagi 2s", "b_naked_2s", 0.85, 1.2, K);
    t("Tarnina 1s", "b_prunusSpinosa_1s", 0.85, 1.15, K);
    t("Tarnina 1s (lato)", "b_prunusSpinosa_1s_summer", 0.85, 1.15, B);
    t("Tarnina 2s (lato)", "b_prunusSpinosa_2s_summer", 0.85, 1.2, B);
    t("Dąb 1f", "b_quercusRobur_1f", 0.85, 1.15, K);
    t("Róża dzika 1s", "b_rosaCanina_1s", 0.85, 1.15, K);
    t("Róża dzika 1s (lato)", "b_rosaCanina_1s_summer", 0.85, 1.15, B);
    t("Róża dzika 2s (lato)", "b_rosaCanina_2s_summer", 0.85, 1.2, B);
    t("Bez czarny 1s", "b_sambucusNigra_1s", 0.85, 1.15, K);
    t("Bez czarny 1s (lato)", "b_sambucusNigra_1s_summer", 0.85, 1.15, B);
    t("Bez czarny 2s (lato)", "b_sambucusNigra_2s_summer", 0.85, 1.2, B);
    t("Brzoza E 2s (lato)", "t_BetulaPendulaE_2s_summer", 0.9, 1.15, B);
    t("Brzoza E 2w (jesień)", "t_BetulaPendulaE_2w_latefall", 0.9, 1.15, S);
    t("Brzoza E 2w (lato)", "t_BetulaPendulaE_2w_summer", 0.9, 1.15, B);
    t("Brzoza E 2w (zima)", "t_BetulaPendulaE_2w_winter", 0.9, 1.15, S);
    t("Brzoza E 3s (lato)", "t_BetulaPendulaE_3s_summer", 0.95, 1.05, B);
    t("Brzoza 1f", "t_BetulaPendula_1f", 0.85, 1.15, L);
    t("Brzoza 1f (jesień)", "t_BetulaPendula_1f_latefall", 0.85, 1.15, S);
    t("Brzoza 1f (zima)", "t_BetulaPendula_1f_winter", 0.85, 1.15, S);
    t("Brzoza 1fb", "t_BetulaPendula_1fb", 0.85, 1.15, L);
    t("Brzoza 1fb (jesień)", "t_BetulaPendula_1fb_latefall", 0.85, 1.15, S);
    t("Brzoza 1fb (zima)", "t_BetulaPendula_1fb_winter", 0.85, 1.15, S);
    t("Brzoza 1s (jesień)", "t_BetulaPendula_1s_latefall", 0.85, 1.15, S);
    t("Brzoza 1s (zima)", "t_BetulaPendula_1s_winter", 0.85, 1.15, S);
    t("Brzoza 2fb", "t_BetulaPendula_2fb", 0.9, 1.15, L);
    t("Brzoza 2fb (jesień)", "t_BetulaPendula_2fb_latefall", 0.9, 1.15, S);
    t("Brzoza 2fb (zima)", "t_BetulaPendula_2fb_winter", 0.9, 1.15, S);
    t("Brzoza 2fc", "t_BetulaPendula_2fc", 0.9, 1.15, L);
    t("Brzoza 2fc (jesień)", "t_BetulaPendula_2fc_latefall", 0.9, 1.15, S);
    t("Brzoza 2fc (zima)", "t_BetulaPendula_2fc_winter", 0.9, 1.15, S);
    t("Brzoza 2s", "t_BetulaPendula_2s", 0.9, 1.15, L);
    t("Brzoza 2s (jesień)", "t_BetulaPendula_2s_latefall", 0.9, 1.15, S);
    t("Brzoza 2s (zima)", "t_BetulaPendula_2s_winter", 0.9, 1.15, S);
    t("Brzoza 2w", "t_BetulaPendula_2w", 0.9, 1.15, L);
    t("Brzoza 2w (jesień)", "t_BetulaPendula_2w_latefall", 0.9, 1.15, S);
    t("Brzoza 2w (zima)", "t_BetulaPendula_2w_winter", 0.9, 1.15, S);
    t("Brzoza 3f (jesień)", "t_BetulaPendula_3f_latefall", 0.95, 1.05, S);
    t("Brzoza 3fb", "t_BetulaPendula_3fb", 0.95, 1.05, L);
    t("Brzoza 3fb (jesień)", "t_BetulaPendula_3fb_latefall", 0.95, 1.05, S);
    t("Brzoza 3fb (zima)", "t_BetulaPendula_3fb_winter", 0.95, 1.05, S);
    t("Brzoza 3fc", "t_BetulaPendula_3fc", 0.95, 1.05, L);
    t("Brzoza 3fc (jesień)", "t_BetulaPendula_3fc_latefall", 0.95, 1.05, S);
    t("Brzoza 3fc (zima)", "t_BetulaPendula_3fc_winter", 0.95, 1.05, S);
    t("Brzoza 3s", "t_BetulaPendula_3s", 0.95, 1.05, L);
    t("Brzoza 3s (jesień)", "t_BetulaPendula_3s_latefall", 0.95, 1.05, S);
    t("Brzoza 3s (zima)", "t_BetulaPendula_3s_winter", 0.95, 1.05, S);
    t("Buk E 2s (lato)", "t_FagusSylvaticaE_2s_summer", 0.9, 1.15, B);
    t("Buk E 3f (lato)", "t_FagusSylvaticaE_3f_summer", 0.95, 1.05, B);
    t("Buk 1f", "t_FagusSylvatica_1f", 0.85, 1.15, L);
    t("Buk 1f (lato)", "t_FagusSylvatica_1f_summer", 0.85, 1.15, B);
    t("Buk 1fb", "t_FagusSylvatica_1fb", 0.85, 1.15, L);
    t("Buk 1fb (lato)", "t_FagusSylvatica_1fb_summer", 0.85, 1.15, B);
    t("Buk 1fc", "t_FagusSylvatica_1fc", 0.85, 1.15, L);
    t("Buk 1fc (lato)", "t_FagusSylvatica_1fc_summer", 0.85, 1.15, B);
    t("Buk 1fd", "t_FagusSylvatica_1fd", 0.85, 1.15, L);
    t("Buk 1fd (lato)", "t_FagusSylvatica_1fd_summer", 0.85, 1.15, B);
    t("Buk 1fe", "t_FagusSylvatica_1fe", 0.85, 1.15, L);
    t("Buk 1fe (lato)", "t_FagusSylvatica_1fe_summer", 0.85, 1.15, B);
    t("Buk 1s", "t_FagusSylvatica_1s", 0.85, 1.15, L);
    t("Buk 2d", "t_FagusSylvatica_2d", 0.9, 1.15, L);
    t("Buk 2d (lato)", "t_FagusSylvatica_2d_summer", 0.9, 1.15, B);
    t("Buk 2f cut", "t_FagusSylvatica_2f_cut", 0.9, 1.15, L);
    t("Buk 2f stump", "t_FagusSylvatica_2f_stump", 0.9, 1.15, L);
    t("Buk 2f (lato)", "t_FagusSylvatica_2f_summer", 0.9, 1.15, B);
    t("Buk 2fb", "t_FagusSylvatica_2fb", 0.9, 1.15, L);
    t("Buk 2fb (lato)", "t_FagusSylvatica_2fb_summer", 0.9, 1.15, B);
    t("Buk 2fc", "t_FagusSylvatica_2fc", 0.9, 1.15, L);
    t("Buk 2fc (lato)", "t_FagusSylvatica_2fc_summer", 0.9, 1.15, B);
    t("Buk 2s", "t_FagusSylvatica_2s", 0.9, 1.15, L);
    t("Buk 2s (lato)", "t_FagusSylvatica_2s_summer", 0.9, 1.15, B);
    t("Buk 2sb", "t_FagusSylvatica_2sb", 0.9, 1.15, L);
    t("Buk 2sb Trail B", "t_FagusSylvatica_2sb_Trail_B", 0.9, 1.15, L);
    t("Buk 2sb Trail B (lato)", "t_FagusSylvatica_2sb_Trail_B_summer", 0.9, 1.15, B);
    t("Buk 2sb Trail G", "t_FagusSylvatica_2sb_Trail_G", 0.9, 1.15, L);
    t("Buk 2sb Trail G (lato)", "t_FagusSylvatica_2sb_Trail_G_summer", 0.9, 1.15, B);
    t("Buk 2sb Trail R", "t_FagusSylvatica_2sb_Trail_R", 0.9, 1.15, L);
    t("Buk 2sb Trail R (lato)", "t_FagusSylvatica_2sb_Trail_R_summer", 0.9, 1.15, B);
    t("Buk 2sb Trail Y", "t_FagusSylvatica_2sb_Trail_Y", 0.9, 1.15, L);
    t("Buk 2sb Trail Y (lato)", "t_FagusSylvatica_2sb_Trail_Y_summer", 0.9, 1.15, B);
    t("Buk 2sb (lato)", "t_FagusSylvatica_2sb_summer", 0.9, 1.15, B);
    t("Buk 3d", "t_FagusSylvatica_3d", 0.9, 1.15, L);
    t("Buk 3d (lato)", "t_FagusSylvatica_3d_summer", 0.9, 1.15, B);
    t("Buk 3f (lato)", "t_FagusSylvatica_3f_summer", 0.95, 1.05, B);
    t("Buk 3fb", "t_FagusSylvatica_3fb", 0.95, 1.05, L);
    t("Buk 3fb (lato)", "t_FagusSylvatica_3fb_summer", 0.95, 1.05, B);
    t("Buk 3fb test", "t_FagusSylvatica_3fb_test", 0.95, 1.05, L);
    t("Buk 3s", "t_FagusSylvatica_3s", 0.95, 1.05, L);
    t("Buk 3s (lato)", "t_FagusSylvatica_3s_summer", 0.95, 1.05, B);
    t("Jesion 2s", "t_FraxinusExcelsior_2s", 0.9, 1.15, L);
    t("Jesion 2w", "t_FraxinusExcelsior_2w", 0.9, 1.15, L);
    t("Jesion 3s", "t_FraxinusExcelsior_3s", 0.95, 1.05, L);
    t("Modrzew 1f", "t_LarixDecidua_1f", 0.85, 1.15, I);
    t("Modrzew 1f (lato)", "t_LarixDecidua_1f_summer", 0.85, 1.15, B);
    t("Modrzew 1s", "t_LarixDecidua_1s", 0.85, 1.15, I);
    t("Modrzew 2f (lato)", "t_LarixDecidua_2f_summer", 0.9, 1.15, B);
    t("Modrzew 2fb", "t_LarixDecidua_2fb", 0.9, 1.15, I);
    t("Modrzew 2fb (lato)", "t_LarixDecidua_2fb_summer", 0.9, 1.15, B);
    t("Modrzew 2s", "t_LarixDecidua_2s", 0.9, 1.15, I);
    t("Modrzew 2s (lato)", "t_LarixDecidua_2s_summer", 0.9, 1.15, B);
    t("Modrzew 3f", "t_LarixDecidua_3f", 0.95, 1.05, I);
    t("Modrzew 3f (lato)", "t_LarixDecidua_3f_summer", 0.95, 1.05, B);
    t("Modrzew 3fb", "t_LarixDecidua_3fb", 0.95, 1.05, I);
    t("Modrzew 3fb (lato)", "t_LarixDecidua_3fb_summer", 0.95, 1.05, B);
    t("Świerk 1f", "t_PiceaAbies_1f", 0.85, 1.15, I);
    t("Świerk 1f (mrok)", "t_PiceaAbies_1f_dark", 0.85, 1.15, S);
    t("Świerk 1f (mróz)", "t_PiceaAbies_1f_frozen", 0.85, 1.15, S);
    t("Świerk 1f (lato)", "t_PiceaAbies_1f_summer", 0.85, 1.15, B);
    t("Świerk 1f (zima)", "t_PiceaAbies_1f_winter", 0.85, 1.15, S);
    t("Świerk 1s (lato)", "t_PiceaAbies_1s_summer", 0.85, 1.15, B);
    t("Świerk 1s (zima)", "t_PiceaAbies_1s_winter", 0.85, 1.15, S);
    t("Świerk 1sb", "t_PiceaAbies_1sb", 0.85, 1.15, I);
    t("Świerk 1sb (mrok)", "t_PiceaAbies_1sb_dark", 0.85, 1.15, S);
    t("Świerk 1sb (mróz)", "t_PiceaAbies_1sb_frozen", 0.85, 1.15, S);
    t("Świerk 1sb (lato)", "t_PiceaAbies_1sb_summer", 0.85, 1.15, B);
    t("Świerk 1sb (zima)", "t_PiceaAbies_1sb_winter", 0.85, 1.15, S);
    t("Świerk 2f (lato)", "t_PiceaAbies_2f_summer", 0.9, 1.15, B);
    t("Świerk 2f (zima)", "t_PiceaAbies_2f_winter", 0.9, 1.15, S);
    t("Świerk 2fb", "t_PiceaAbies_2fb", 0.9, 1.15, I);
    t("Świerk 2fb (mrok)", "t_PiceaAbies_2fb_dark", 0.9, 1.15, S);
    t("Świerk 2fb (mróz)", "t_PiceaAbies_2fb_frozen", 0.9, 1.15, S);
    t("Świerk 2fb (lato)", "t_PiceaAbies_2fb_summer", 0.9, 1.15, B);
    t("Świerk 2fb (zima)", "t_PiceaAbies_2fb_winter", 0.9, 1.15, S);
    t("Świerk 2s", "t_PiceaAbies_2s", 0.9, 1.15, I);
    t("Świerk 2s (mrok)", "t_PiceaAbies_2s_dark", 0.9, 1.15, S);
    t("Świerk 2s (mróz)", "t_PiceaAbies_2s_frozen", 0.9, 1.15, S);
    t("Świerk 2s green xmas", "t_PiceaAbies_2s_green_xmas", 0.9, 1.15, I);
    t("Świerk 2s (lato)", "t_PiceaAbies_2s_summer", 0.9, 1.15, B);
    t("Świerk 2s (zima)", "t_PiceaAbies_2s_winter", 0.9, 1.15, S);
    t("Świerk 2s xmas", "t_PiceaAbies_2s_xmas", 0.9, 1.15, I);
    t("Świerk 2sb", "t_PiceaAbies_2sb", 0.9, 1.15, I);
    t("Świerk 2sb (mrok)", "t_PiceaAbies_2sb_dark", 0.9, 1.15, S);
    t("Świerk 2sb (mróz)", "t_PiceaAbies_2sb_frozen", 0.9, 1.15, S);
    t("Świerk 2sb (lato)", "t_PiceaAbies_2sb_summer", 0.9, 1.15, B);
    t("Świerk 2sb (zima)", "t_PiceaAbies_2sb_winter", 0.9, 1.15, S);
    t("Świerk 3f Trail B", "t_PiceaAbies_3f_Trail_B", 0.95, 1.05, I);
    t("Świerk 3f Trail B (lato)", "t_PiceaAbies_3f_Trail_B_summer", 0.95, 1.05, B);
    t("Świerk 3f Trail G", "t_PiceaAbies_3f_Trail_G", 0.95, 1.05, I);
    t("Świerk 3f Trail G (lato)", "t_PiceaAbies_3f_Trail_G_summer", 0.95, 1.05, B);
    t("Świerk 3f Trail R", "t_PiceaAbies_3f_Trail_R", 0.95, 1.05, I);
    t("Świerk 3f Trail R (lato)", "t_PiceaAbies_3f_Trail_R_summer", 0.95, 1.05, B);
    t("Świerk 3f Trail Y", "t_PiceaAbies_3f_Trail_Y", 0.95, 1.05, I);
    t("Świerk 3f Trail Y (lato)", "t_PiceaAbies_3f_Trail_Y_summer", 0.95, 1.05, B);
    t("Świerk 3f (lato)", "t_PiceaAbies_3f_summer", 0.95, 1.05, B);
    t("Świerk 3s", "t_PiceaAbies_3s", 0.95, 1.05, I);
    t("Świerk 3s (lato)", "t_PiceaAbies_3s_summer", 0.95, 1.05, B);
    t("Sosna 1f", "t_PinusSylvestris_1f", 0.85, 1.15, I);
    t("Sosna 1s (lato)", "t_PinusSylvestris_1s_summer", 0.85, 1.15, B);
    t("Sosna 2f (lato)", "t_PinusSylvestris_2f_summer", 0.9, 1.15, B);
    t("Sosna 2fb", "t_PinusSylvestris_2fb", 0.9, 1.15, I);
    t("Sosna 2fb (lato)", "t_PinusSylvestris_2fb_summer", 0.9, 1.15, B);
    t("Sosna 2s", "t_PinusSylvestris_2s", 0.9, 1.15, I);
    t("Sosna 2s (lato)", "t_PinusSylvestris_2s_summer", 0.9, 1.15, B);
    t("Sosna 2sb", "t_PinusSylvestris_2sb", 0.9, 1.15, I);
    t("Sosna 2sb (lato)", "t_PinusSylvestris_2sb_summer", 0.9, 1.15, B);
    t("Sosna 3d", "t_PinusSylvestris_3d", 0.9, 1.15, I);
    t("Sosna 3d (lato)", "t_PinusSylvestris_3d_summer", 0.9, 1.15, B);
    t("Sosna 3f (lato)", "t_PinusSylvestris_3f_summer", 0.95, 1.05, B);
    t("Sosna 3fb", "t_PinusSylvestris_3fb", 0.95, 1.05, I);
    t("Sosna 3fb (lato)", "t_PinusSylvestris_3fb_summer", 0.95, 1.05, B);
    t("Sosna 3fc", "t_PinusSylvestris_3fc", 0.95, 1.05, I);
    t("Sosna 3fc (lato)", "t_PinusSylvestris_3fc_summer", 0.95, 1.05, B);
    t("Sosna 3s", "t_PinusSylvestris_3s", 0.95, 1.05, I);
    t("Sosna 3s (lato)", "t_PinusSylvestris_3s_summer", 0.95, 1.05, B);
    t("carpinus 2s", "t_carpinus_2s", 0.9, 1.15, I);
    t("Orzech włoski 2s", "t_juglansRegia_2s", 0.9, 1.15, L);
    t("Orzech włoski 3s", "t_juglansRegia_3s", 0.95, 1.05, L);
    t("Jabłoń 1s", "t_malusDomestica_1s", 0.85, 1.15, L);
    t("Jabłoń 2s", "t_malusDomestica_2s", 0.9, 1.15, L);
    t("Jabłoń 2s (lato)", "t_malusDomestica_2s_summer", 0.9, 1.15, B);
    t("Jabłoń 3s", "t_malusDomestica_3s", 0.95, 1.05, L);
    t("Krzew nagi 1s", "t_naked_1s", 0.85, 1.15, L);
    t("Krzew nagi 2s", "t_naked_2s", 0.9, 1.15, L);
    t("piceaabies 2d", "t_piceaabies_2d", 0.9, 1.15, I);
    t("piceaabies 2d (mrok)", "t_piceaabies_2d_dark", 0.9, 1.15, S);
    t("piceaabies 2d (zima)", "t_piceaabies_2d_winter", 0.9, 1.15, S);
    t("piceaabies 3d", "t_piceaabies_3d", 0.9, 1.15, I);
    t("piceaabies 3d (mrok)", "t_piceaabies_3d_dark", 0.9, 1.15, S);
    t("piceaabies 3d (zima)", "t_piceaabies_3d_winter", 0.9, 1.15, S);
    t("Topola biała 1f", "t_populusAlba_1f", 0.85, 1.15, L);
    t("Topola biała 2s", "t_populusAlba_2s", 0.9, 1.15, L);
    t("Topola biała 3s", "t_populusAlba_3s", 0.95, 1.05, L);
    t("Topola biała 3s (jesień)", "t_populusAlba_3s_latefall", 0.95, 1.05, S);
    t("Topola czarna 3s", "t_populusNigra_3s", 0.95, 1.05, L);
    t("Topola czarna 3s (jesień)", "t_populusNigra_3s_latefall", 0.95, 1.05, S);
    t("prunusDomestica 2s", "t_prunusDomestica_2s", 0.9, 1.15, L);
    t("Grusza 2s", "t_pyrusCommunis_2s", 0.9, 1.15, L);
    t("Grusza 3s", "t_pyrusCommunis_3s", 0.95, 1.05, L);
    t("Grusza 3sb", "t_pyrusCommunis_3sb", 0.95, 1.05, L);
    t("Dąb 1fb", "t_quercusRobur_1fb", 0.85, 1.15, L);
    t("Dąb 1fc", "t_quercusRobur_1fc", 0.85, 1.15, L);
    t("Dąb 1fd", "t_quercusRobur_1fd", 0.85, 1.15, L);
    t("Dąb 1s", "t_quercusRobur_1s", 0.85, 1.15, L);
    t("Dąb 2d", "t_quercusRobur_2d", 0.9, 1.15, L);
    t("Dąb 2fb", "t_quercusRobur_2fb", 0.9, 1.15, L);
    t("Dąb 2fc", "t_quercusRobur_2fc", 0.9, 1.15, L);
    t("Dąb 2s", "t_quercusRobur_2s", 0.9, 1.15, L);
    t("Dąb 2sb", "t_quercusRobur_2sb", 0.9, 1.15, L);
    t("Dąb 2sc", "t_quercusRobur_2sc", 0.9, 1.15, L);
    t("Dąb 3d", "t_quercusRobur_3d", 0.9, 1.15, L);
    t("Dąb 3fb", "t_quercusRobur_3fb", 0.95, 1.05, L);
    t("Dąb 3s", "t_quercusRobur_3s", 0.95, 1.05, L);
    t("Dąb 3sb", "t_quercusRobur_3sb", 0.95, 1.05, L);
    t("Robinia 1f", "t_robiniaPseudoacacia_1f", 0.85, 1.15, L);
    t("Robinia 2s", "t_robiniaPseudoacacia_2s", 0.9, 1.15, L);
    t("Robinia 3f", "t_robiniaPseudoacacia_3f", 0.95, 1.05, L);
    t("Wierzba biała 2s", "t_salixAlba_2s", 0.9, 1.15, L);
    t("Wierzba biała 2sb", "t_salixAlba_2sb", 0.9, 1.15, L);
    t("Wierzba biała 2sb swamp", "t_salixAlba_2sb_swamp", 0.9, 1.15, L);
    t("Jarzębina 2s", "t_sorbus_2s", 0.9, 1.15, L);
    t("town 1s", "t_town_1s", 0.85, 1.15, L);
    t("town 1sb", "t_town_1sb", 0.85, 1.15, L);
    // --- pniaki i wykroty (d_*) — martwe drewno z P:\DZ\plants*\tree ---
    // Chernarus (dz\plants\tree) + neutralne z Bliss (bez suffixu sezonu);
    // warianty _dark/_winter/_latefall w grupie Sakhal.
    t("Buk (wykrot)", "d_FagusSylvatica_fallen", 0.9, 1.15, L);
    t("Buk (wykrot B)", "d_FagusSylvatica_fallenb", 0.9, 1.15, L);
    t("Buk (wykrot C)", "d_FagusSylvatica_fallenc", 0.9, 1.15, L);
    t("Buk (pniak)", "d_FagusSylvatica_stump", 0.9, 1.15, L);
    t("Buk (pniak B)", "d_FagusSylvatica_stumpb", 0.9, 1.15, L);
    t("Buk (pniak C)", "d_FagusSylvatica_stumpc", 0.9, 1.15, L);
    t("Dąb (wykrot B)", "d_quercusRobur_fallenb", 0.9, 1.15, L);
    t("Dąb (wykrot C)", "d_quercusRobur_fallenc", 0.9, 1.15, L);
    t("Dąb (pniak)", "d_quercusRobur_stump", 0.9, 1.15, L);
    t("Dąb (pniak B)", "d_quercusRobur_stumpb", 0.9, 1.15, L);
    t("Brzoza (wykrot)", "d_BetulaPendula_fallen", 0.9, 1.15, L);
    t("Brzoza (pniak)", "d_BetulaPendula_stump", 0.9, 1.15, L);
    t("Sosna (wykrot)", "d_PinusSylvestris_fallen", 0.9, 1.15, I);
    t("Sosna (wykrot B)", "d_PinusSylvestris_fallenb", 0.9, 1.15, I);
    t("Sosna (wykrot C)", "d_PinusSylvestris_fallenc", 0.9, 1.15, I);
    t("Sosna (pniak)", "d_PinusSylvestris_stump", 0.9, 1.15, I);
    t("Sosna (pniak B)", "d_PinusSylvestris_stumpb", 0.9, 1.15, I);
    t("Świerk (wykrot)", "d_piceaabies_fallen", 0.9, 1.15, I);
    t("Świerk (wykrot B)", "d_piceaabies_fallenb", 0.9, 1.15, I);
    t("Świerk (wykrot C)", "d_piceaabies_fallenc", 0.9, 1.15, I);
    t("Świerk (wykrot D)", "d_piceaabies_fallend", 0.9, 1.15, I);
    t("Świerk (wykrot E)", "d_piceaabies_fallene", 0.9, 1.15, I);
    t("Świerk (pniak)", "d_piceaabies_stump", 0.9, 1.15, I);
    t("Świerk (pniak B)", "d_piceaabies_stumpb", 0.9, 1.15, I);
    t("Świerk (pniak C)", "d_piceaabies_stumpc", 0.9, 1.15, I);
    t("Świerk (pniak D)", "d_piceaabies_stumpd", 0.9, 1.15, I);
    t("Świerk (wykrot, mrok)", "d_piceaabies_fallen_dark", 0.9, 1.15, S);
    t("Świerk (wykrot B, mrok)", "d_piceaabies_fallenb_dark", 0.9, 1.15, S);
    t("Świerk (wykrot C, mrok)", "d_piceaabies_fallenc_dark", 0.9, 1.15, S);
    t("Świerk (wykrot D, mrok)", "d_piceaabies_fallend_dark", 0.9, 1.15, S);
    t("Świerk (wykrot E, mrok)", "d_piceaabies_fallene_dark", 0.9, 1.15, S);
    t("Świerk (pniak, mrok)", "d_piceaabies_stump_dark", 0.9, 1.15, S);
    t("Świerk (pniak B, mrok)", "d_piceaabies_stumpb_dark", 0.9, 1.15, S);
    t("Świerk (pniak C, mrok)", "d_piceaabies_stumpc_dark", 0.9, 1.15, S);
    t("Świerk (wykrot, zima)", "d_piceaabies_fallen_winter", 0.9, 1.15, S);
    t("Świerk (wykrot B, zima)", "d_piceaabies_fallenb_winter", 0.9, 1.15, S);
    t("Świerk (wykrot C, zima)", "d_piceaabies_fallenc_winter", 0.9, 1.15, S);
    t("Świerk (wykrot D, zima)", "d_piceaabies_fallend_winter", 0.9, 1.15, S);
    t("Świerk (wykrot E, zima)", "d_piceaabies_fallene_winter", 0.9, 1.15, S);
    t("Świerk (pniak, zima)", "d_piceaabies_stump_winter", 0.9, 1.15, S);
    t("Świerk (pniak B, zima)", "d_piceaabies_stumpb_winter", 0.9, 1.15, S);
    t("Świerk (pniak C, zima)", "d_piceaabies_stumpc_winter", 0.9, 1.15, S);
    t("Brzoza (wykrot, jesień)", "d_BetulaPendula_fallen_latefall", 0.9, 1.15, S);
    t("Brzoza (pniak, jesień)", "d_BetulaPendula_stump_latefall", 0.9, 1.15, S);
    t("Brzoza (wykrot, zima)", "d_BetulaPendula_fallen_winter", 0.9, 1.15, S);
    t("Brzoza (pniak, zima)", "d_BetulaPendula_stump_winter", 0.9, 1.15, S);
    // posortuj wg grupy dla spójnego wyświetlania (Iglaste/Liściaste/Krzewy + DLC)
    v.sort_by(|a, b| {
        let order = |g: &str| match g {
            "Iglaste" => 0,
            "Liściaste" => 1,
            "Krzewy" => 2,
            "Bliss (lato)" => 3,
            "Sakhal (zima/mrok)" => 4,
            _ => 99,
        };
        order(&a.group)
            .cmp(&order(&b.group))
            .then_with(|| a.group.cmp(&b.group))
            .then_with(|| a.label.cmp(&b.label))
    });
    v
}

/// Skanuje katalogi roślinności na P:\ i zwraca zestaw nazw modeli (bez .p3d,
/// z zachowaną wielkością liter jak na dysku). Obejmuje żywe drzewa/krzewy
/// (t_*/b_*) oraz martwe drewno — pniaki i wykroty (d_*):
/// - P:\DZ\plants\tree + bush (Chernarus),
/// - P:\DZ\plants_bliss (Livonia/Bliss, warianty letnie),
/// - P:\DZ\plants_sakhal (Sakhal, mrok/zima/jesień).
/// Błąd, gdy workdrive nie jest zamontowany.
pub fn game_plant_models() -> Result<std::collections::HashSet<String>, String> {
    Ok(game_plant_files()?.into_iter().map(|f| f.model).collect())
}

/// Plik modelu rośliny znaleziony na dysku roboczym.
#[derive(Clone, Debug)]
pub struct PlantFile {
    /// Nazwa modelu (plik bez `.p3d`).
    pub model: String,
    pub path: std::path::PathBuf,
}

/// Wszystkie pliki modeli roślin z P:\DZ\plants* (zakres jak w `game_plant_models`).
pub fn game_plant_files() -> Result<Vec<PlantFile>, String> {
    fn scan_recursive(dir: &str, out: &mut Vec<PlantFile>) -> Result<(), String> {
        let rd = std::fs::read_dir(dir)
            .map_err(|e| format!("Nie mogę otworzyć {dir}: {e}"))?;
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                scan_recursive(&path.to_string_lossy(), out)?;
            } else if let Some(stem) = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .and_then(|n| n.strip_suffix(".p3d").map(|s| s.to_string()))
            {
                let low = stem.to_lowercase();
                if low.starts_with("t_") || low.starts_with("b_") || low.starts_with("d_") {
                    out.push(PlantFile { model: stem, path });
                }
            }
        }
        Ok(())
    }

    let mut out = Vec::new();
    let mut any = false;
    // rekurencyjnie — martwe drewno leży też w podkatalogach
    // (tree\cutted_cut, tree\cutted_stump)
    for dir in [
        "P:\\DZ\\plants\\tree",
        "P:\\DZ\\plants\\bush",
    ] {
        scan_recursive(dir, &mut out)?;
        any = true;
    }
    for root in [
        "P:\\DZ\\plants_bliss",
        "P:\\DZ\\plants_sakhal",
        "P:\\DZ\\plants\\clutter",
    ] {
        if std::path::Path::new(root).exists() {
            scan_recursive(root, &mut out)?;
            any = true;
        }
    }
    if !any || out.is_empty() {
        return Err("W P:\\DZ\\plants* nie znaleziono żadnych modeli t_/b_/d_".into());
    }
    Ok(out)
}

/// Zwraca listę gatunków, których modelu nie ma wśród plików gry.
/// `available` — wynik `game_plant_models()`.
pub fn missing_in_game(species: &[SpeciesDef], available: &std::collections::HashSet<String>) -> Vec<String> {
    species
        .iter()
        .filter(|s| !available.contains(&s.model))
        .map(|s| format!("{} ({})", s.label, s.model))
        .collect()
}

/// Kolory podglądu dla gatunków (HSV -> RGB, rozłożone po kole barw).
pub fn species_preview_color(index: usize) -> [u8; 3] {
    let hue = (index as f64 * 0.618_033_988_7) % 1.0; // złoty kąt — dobrze rozróżnialne
    hsv_to_rgb(hue, 0.75, 0.95)
}

/// Gotowy szablon strefy lasu: gęstość + proporcje gatunków (indeksy wg
/// `vanilla_library()`). Używany przez GUI i CLI do szybkiego dodawania stref.
#[derive(Clone, Copy, Debug)]
pub struct ZonePreset {
    /// Grupa (np. "Iglaste") — używana tylko do prezentacji w UI.
    pub group: &'static str,
    pub name: &'static str,
    pub density_per_ha: f32,
    /// (indeks gatunku, waga)
    pub weights: &'static [(usize, f32)],
}

impl ZonePreset {
    pub fn to_zone_def(&self, color: Rgb8) -> crate::preset::ZoneDef {
        crate::preset::ZoneDef {
            color,
            label: self.name.to_string(),
            density_per_ha: self.density_per_ha,
            species_weights: self.weights.iter().map(|(i, w)| (*i, *w)).collect(),
            preset_mix: Vec::new(),
        }
    }
}

/// Pogrupowane presety roślinności (typy lasów jak w klimacie Chernarus).
/// Indeksy gatunków wg `vanilla_library()` powyżej.
pub fn zone_presets() -> Vec<ZonePreset> {
    use ZonePreset as P;
    // UWAGA: indeksy odnoszą się do posortowanej listy z vanilla_library()
    // (sortowanie na końcu tej funkcji). Po dodaniu gatunków indeksy trzeba
    // przeliczyć — patrz skrypt gen_deadwood.py w katalogu roboczym.
    let mut v = vec![
        P { group: "Iglaste", name: "Bór świerkowy (góry)", density_per_ha: 260.0, weights: &[(26, 4.0), (48, 3.0), (49, 2.0), (0, 1.0)] },
        P { group: "Iglaste", name: "Bór sosnowy (niziny)", density_per_ha: 230.0, weights: &[(7, 4.0), (21, 3.0), (22, 2.0), (26, 1.0)] },
        P { group: "Iglaste", name: "Młodnik iglasty", density_per_ha: 340.0, weights: &[(48, 5.0), (21, 3.0), (2, 2.0)] },
        P { group: "Iglaste", name: "Modrzewina", density_per_ha: 220.0, weights: &[(0, 4.0), (2, 3.0), (5, 2.0)] },
        P { group: "Liściaste", name: "Dębowa puszcza", density_per_ha: 180.0, weights: &[(93, 4.0), (112, 3.0), (113, 2.0), (64, 1.0)] },
        P { group: "Liściaste", name: "Grąd (dąb-buk)", density_per_ha: 200.0, weights: &[(64, 3.0), (93, 3.0), (112, 2.0), (92, 1.0)] },
        P { group: "Liściaste", name: "Buczyna karpacka", density_per_ha: 210.0, weights: &[(64, 4.0), (92, 3.0), (76, 2.0), (26, 1.0)] },
        P { group: "Liściaste", name: "Brzozowy zagajnik", density_per_ha: 190.0, weights: &[(50, 4.0), (62, 3.0), (63, 2.0), (93, 1.0)] },
        P { group: "Liściaste", name: "Łęg nadrzeczny", density_per_ha: 170.0, weights: &[(121, 3.0), (137, 3.0), (136, 2.0), (50, 1.0)] },
        P { group: "Mieszane", name: "Las mieszany nizinny", density_per_ha: 200.0, weights: &[(50, 3.0), (93, 2.0), (7, 2.0), (26, 1.0)] },
        P { group: "Mieszane", name: "Las mieszany wyżynny", density_per_ha: 210.0, weights: &[(26, 3.0), (64, 2.0), (50, 2.0), (113, 1.0)] },
        P { group: "Krzewy i zarośla", name: "Zarośla krzewiaste", density_per_ha: 420.0, weights: &[(152, 3.0), (154, 3.0), (156, 2.0), (143, 2.0)] },
        P { group: "Krzewy i zarośla", name: "Samosiewy (młodnik)", density_per_ha: 360.0, weights: &[(62, 3.0), (146, 2.0), (149, 2.0), (152, 1.0)] },
        P { group: "Bliss (lato)", name: "Letni gaj liściasty", density_per_ha: 200.0, weights: &[(198, 3.0), (162, 3.0), (176, 2.0), (196, 1.0)] },
        P { group: "Bliss (lato)", name: "Letni bór sosnowy", density_per_ha: 220.0, weights: &[(214, 4.0), (219, 3.0), (202, 2.0), (232, 1.0)] },
        P { group: "Sakhal (zima/mrok)", name: "Mroczny bór świerkowy", density_per_ha: 260.0, weights: &[(328, 4.0), (329, 3.0), (326, 2.0), (317, 1.0)] },
        P { group: "Sakhal (zima/mrok)", name: "Zimowy las mieszany", density_per_ha: 210.0, weights: &[(277, 3.0), (278, 3.0), (316, 2.0), (330, 1.0)] },
        P { group: "Sakhal (zima/mrok)", name: "Mroczna brzezina", density_per_ha: 190.0, weights: &[(277, 4.0), (278, 3.0), (275, 2.0), (328, 1.0)] },
        // --- martwe drewno per gatunek (d_*): pniaki po zrębie, wykroty po wiatrołomach ---
        P { group: "Pniaki i wykroty", name: "Buk — pniaki i wykroty", density_per_ha: 30.0, weights: &[(65, 1.0), (66, 1.0), (67, 1.0), (68, 1.0), (69, 1.0), (70, 1.0), (78, 1.0), (79, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Dąb — pniaki i wykroty", density_per_ha: 30.0, weights: &[(94, 1.0), (95, 1.0), (96, 1.0), (97, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Sosna — pniaki i wykroty", density_per_ha: 30.0, weights: &[(8, 1.0), (9, 1.0), (10, 1.0), (11, 1.0), (12, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Świerk — pniaki i wykroty", density_per_ha: 30.0, weights: &[(27, 1.0), (28, 1.0), (29, 1.0), (30, 1.0), (31, 1.0), (32, 1.0), (33, 1.0), (34, 1.0), (35, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Brzoza — pniaki i wykroty", density_per_ha: 30.0, weights: &[(51, 1.0), (52, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Świerk mroczny — pniaki i wykroty", density_per_ha: 30.0, weights: &[(289, 1.0), (291, 1.0), (293, 1.0), (295, 1.0), (297, 1.0), (299, 1.0), (301, 1.0), (303, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Świerk zimowy — pniaki i wykroty", density_per_ha: 30.0, weights: &[(290, 1.0), (292, 1.0), (294, 1.0), (296, 1.0), (298, 1.0), (300, 1.0), (302, 1.0), (304, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Brzoza zimowa — pniaki i wykroty", density_per_ha: 30.0, weights: &[(244, 1.0), (246, 1.0)] },
        P { group: "Pniaki i wykroty", name: "Brzoza jesienna — pniaki i wykroty", density_per_ha: 30.0, weights: &[(243, 1.0), (245, 1.0)] },
        // --- gotowe miksy na pas graniczny (do wyboru w Granicy lasu) ---
        P { group: "Krzewy i zarośla", name: "Żywopłot liściasty", density_per_ha: 160.0, weights: &[(152, 3.0), (154, 2.0), (156, 2.0), (143, 1.0), (149, 1.0)] },
        P { group: "Krzewy i zarośla", name: "Podszyt — samosiewy", density_per_ha: 220.0, weights: &[(62, 3.0), (76, 2.0), (101, 2.0), (48, 1.0), (21, 1.0)] },
        P { group: "Krzewy i zarośla", name: "Ciernisty pas", density_per_ha: 180.0, weights: &[(156, 3.0), (149, 3.0), (154, 2.0), (157, 1.0)] },
        P { group: "Sakhal (zima/mrok)", name: "Zimowy pas krzewów", density_per_ha: 150.0, weights: &[(275, 3.0), (274, 2.0), (279, 2.0), (307, 1.0)] },
    ];
    v.sort_by(|a, b| {
        let order = |g: &str| match g {
            "Iglaste" => 0,
            "Liściaste" => 1,
            "Mieszane" => 2,
            "Krzewy i zarośla" => 3,
            "Bliss (lato)" => 4,
            "Sakhal (zima/mrok)" => 5,
            "Pniaki i wykroty" => 6,
            _ => 99,
        };
        order(a.group)
            .cmp(&order(b.group))
            .then_with(|| a.group.cmp(b.group))
            .then_with(|| a.name.cmp(b.name))
    });
    v
}

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [u8; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i64) % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vanilla_library_has_unique_models() {
        let lib = vanilla_library();
        assert!(lib.len() >= 10);
        let mut models: Vec<&str> = lib.iter().map(|s| s.model.as_str()).collect();
        models.sort_unstable();
        models.dedup();
        assert_eq!(models.len(), lib.len(), "zduplikowane modele w bibliotece");
    }

    #[test]
    fn preview_colors_differ() {
        let a = species_preview_color(0);
        let b = species_preview_color(5);
        assert_ne!(a, b);
    }

    #[test]
    fn zone_presets_are_valid() {
        let lib_len = vanilla_library().len();
        let presets = zone_presets();
        assert!(presets.len() >= 8, "za mało presetów");
        let mut names: Vec<&str> = presets.iter().map(|p| p.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), presets.len(), "zduplikowane nazwy presetów");

        for p in &presets {
            assert!(!p.group.is_empty());
            assert!(p.density_per_ha > 0.0, "{}: gęstość", p.name);
            assert!(!p.weights.is_empty(), "{}: brak gatunków", p.name);
            for (i, w) in p.weights {
                assert!((*i as usize) < lib_len, "{}: zły indeks gatunku {i}", p.name);
                assert!(*w > 0.0, "{}: waga <= 0", p.name);
            }
        }
    }

    #[test]
    fn preset_to_zone_def_roundtrip() {
        let p = &zone_presets()[0];
        let z = p.to_zone_def(Rgb8([1, 2, 3]));
        assert_eq!(z.color, Rgb8([1, 2, 3]));
        assert_eq!(z.label, p.name);
        assert_eq!(z.density_per_ha, p.density_per_ha);
        assert_eq!(z.species_weights.len(), p.weights.len());
    }
}
