# DZ World Editor: instrukcje dla Claude

Edytor map DayZ w Rust (egui/eframe + OpenGL 3.3), tylko Windows 10/11. Repo: `MrKamil404/DZWorldEditor`.
Opis funkcji dla użytkownika jest w `README.md` i `docs/` (`guide.md`, `mission-editor.md`, `team-work.md`).

## Język

- Z użytkownikiem rozmawiaj po polsku.
- Kod, komentarze, komunikaty commitów, `README.md` i `docs/` piszemy po angielsku, prostym językiem
  (krótkie zdania, bez żargonu, opis tego, co widzi użytkownik).

## Struktura

Workspace Cargo (`Cargo.toml` w katalogu głównym, wersja wspólna w `[workspace.package]`):

| Crate | Co robi |
| --- | --- |
| `crates/dz-core` | model świata, projekt (`project.dzwe`), import/eksport (Terrain Builder, WRP, TV4L/TV4P), generatory (`gen.rs`), build (AddonBuilder, PBO), misja serwera (`mission/`), praca zespołowa (`team/`) |
| `crates/dz-editor` | aplikacja GUI, binarka `DZWorldEditor` (`main.rs`, `app.rs`, `viewport.rs`, `render.rs`, `panels.rs`, ...) |
| `crates/forest-core` | lasy i formaty BI: PAA, MLOD/ODOL, rvmat, maski, heightmapy, SHP |
| `crates/road-core` | drogi (z DayZ Road Studio), lib nazywa się `dayz_road_tool`, edition 2024 |

- `assets/app-icon.ico` jest wkompilowana przez `crates/dz-editor/build.rs` (rc.exe + cvtres z MSVC).
- `assets/lang/catalog.json`: tłumaczenia interfejsu (klucz = tekst angielski, wartości `pl` i `de`).
- Ustawienia użytkownika leżą w `%LOCALAPPDATA%\DZWorldEditor` (`settings.json`, `brushes`, `lang`).
- Część plików ma kilka tysięcy linii (`viewport.rs`, `gen.rs`, `mission_ui.rs`). Szukaj Grepem i czytaj fragmenty zamiast całości.

## Tłumaczenia (ważne)

- Każdy tekst w UI piszemy po angielsku i owijamy w `i18n::t("...")` albo `tf("... {0}", &[..])`.
- **Każdy nowy lub zmieniony tekst w `t(`/`tf(` musi dostać wpis `pl` i `de` w `assets/lang/catalog.json`**.
  Test `every_interface_string_is_translated` w `crates/dz-editor/src/i18n.rs` to sprawdza, a
  `placeholders_survive_translation` pilnuje, żeby `{0}`, `{1}` itd. były też w tłumaczeniu.
- Przy zmianie angielskiego tekstu zaktualizuj klucz w katalogu i usuń stary wpis.

## Budowanie i testy

```bash
cargo test --workspace --release
cargo build --release -p dz-editor
```

- Gdy edytor użytkownika jest uruchomiony, blokuje `target\release\DZWorldEditor.exe`. Wtedy buduj z
  `CARGO_TARGET_DIR` ustawionym na katalog w scratchpadzie. Nigdy nie zabijaj procesu edytora użytkownika.
- CI (`.github/workflows/release.yml`) uruchamia `cargo test --workspace --release` na każdym pushu do `main`.
  Przed commitem testy mają przechodzić.
- Zmienne środowiskowe do testów i automatyzacji:
  - `DZWE_AUTO="cmd;cmd;..."` skryptuje edytor (jedna komenda na klatkę: `open`, `cam`, `perf`, `viewstats`, `shot`, `quit`, ...).
  - `DZWE_CE_DIR` i `DZWE_WORK_DRIVE` włączają testy misji na prawdziwych danych gry. Bez nich te testy pomijają tę część.
- Przykłady headless: `crates/dz-core/examples/` (`headless.rs`, `server_test.rs`, ...).
- Dane gry są na dysku roboczym `P:\` (DayZ Tools). Nie zakładaj, że istnieje w CI.

## Git i wydania

- **Nie commituj, nie pushuj, nie taguj i nie wydawaj bez wyraźnej prośby użytkownika.**
- Większe zmiany robimy na gałęzi `feature/<nazwa>`, `fix/<nazwa>` albo `perf/<nazwa>`, potem merge (`Merge branch '...'`) do `main`.
- Komunikaty commitów: `feat:`, `fix:`, `perf:`, `docs:`, `chore:`. Opis mówi, co zmienia się dla użytkownika, np.
  `feat: surface under road parts`, `fix: bridges line up with their road`. Kilka zmian łączymy `;`.
- Wydanie:
  1. podbij `version` w `[workspace.package]` w `Cargo.toml` (i odśwież `Cargo.lock`),
  2. zrób commit `chore: release X.Y.Z` na `main`,
  3. dodaj tag `vX.Y.Z` i wypchnij `main` oraz tag.

  Tag `v*` uruchamia CI, które publikuje `DZWorldEditor.exe` dla wbudowanego aktualizatora.
- Cargo przyjmuje tylko trzy liczby w wersji. Czwartą zapisujemy jako `0.2.4+3`. Edytor pokazuje ją jako 0.2.4.3, a tag to `v0.2.4.3`.
- `gh` nie jest zainstalowany. Status Actions można sprawdzić przez publiczne API (curl).

## Tworzenie i zmiana funkcji (spójność)

Zanim napiszesz nową funkcję, struct albo okno:

1. **Poszukaj, czy to już istnieje.** Przeszukaj Grepem cały workspace (wszystkie cztery crate'y), a nie tylko
   plik, nad którym pracujesz. Szukaj po nazwach, formatach plików (`rvmat`, `paa`, `CfgSurfaces`, `layers.cfg`...)
   i po pojęciach z gry. Podobne rzeczy często już są, np. parsery PAA i ODOL są w `forest-core` i w `road-core`,
   a parser configów to `dz_core::surfaces::parse`.
2. **Jeśli istnieje, użyj jej albo ją rozbuduj.** Dodaj parametr, opcję albo wariant zamiast pisać drugą,
   uboższą wersję obok. Wszystkie miejsca, które z niej korzystają, mają dostać nową możliwość.
3. **Jeśli znalazłeś dwie implementacje tego samego, połącz je** albo przynajmniej powiedz o tym użytkownikowi.
   Nie dokładaj trzeciej.
4. **Logika należy do `dz-core`**, a `dz-editor` ją tylko wyświetla i wywołuje. Dzięki temu import, eksport,
   build, testy i przykłady headless korzystają z tego samego kodu co UI.
5. **Wygląd i zachowanie UI mają być takie same jak w podobnych miejscach edytora**: te same widgety, ikony
   (`icons.rs`), skróty (`keys.rs`), undo, okna dialogowe i nazwy. Zanim zrobisz nowy panel, przeczytaj podobny
   panel, który już istnieje.

## Czy to trafia do mapy? (zawsze sprawdź)

Każda nowa rzecz w projekcie (dane, ustawienie, kreator, generator, zasób) może być potrzebna w grze. **Bez
osobnej prośby** sprawdź i podłącz wszystkie ścieżki, które jej dotyczą:

- **Build / eksport do `P:\<map id>`**: `dz_core::export::run`, `write_configs` (`config.cpp`, `CfgWorlds`),
  `write_ce` (Central Economy), `surfaces::write_config` (`CfgSurfaces`) i pliki w folderach `world`/`data`/`navmesh`
  pakowanych przez `build::build_pbo`. Jeśli gra ma to widzieć, musi to być w PBO.
- **Zapis i odczyt projektu** (`project.dzwe`, `project.rs`): nowe pola z `#[serde(default)]`, żeby stare projekty
  dalej się otwierały.
- **Import**: Terrain Builder, WRP, `P:\` (`import.rs`, `wrp.rs`). Jeśli mapa już ma takie dane w swoich plikach
  lub configach, import powinien je wczytać.
- **Eksport dla Terrain Buildera** (`export_files.rs`, `write_source`), jeśli TB ma odpowiednik.
- **Misja serwera i praca zespołowa** (`mission/`, `team/`), jeśli dane dotyczą misji albo są edytowane przez kilka osób
  (scalanie zmian).
- **Undo, kopie zapasowe i autosave.**

Przykład z historii: kreator powierzchni (map surfaces) powstał najpierw tylko w edytorze, a pakowanie
`CfgSurfaces` do mapy trzeba było dorobić osobno. Tak ma nie być. Funkcja jest skończona dopiero wtedy, gdy działa
w grze po Build (F7). Jeśli czegoś świadomie nie podłączasz, napisz o tym użytkownikowi.

## Configi i dane gry

- Configi gry (`config.cpp` w `P:\DZ\...`, np. `CfgVehicles`, `CfgSurfaces`, `CfgSurfaceCharacters`, `CfgWorlds`,
  `CfgSoundTables`, `CfgModels`) i pliki misji vanilla (CE: `types.xml`, `events.xml`, `cfgeconomycore.xml`...) są
  źródłem prawdy o tym, jak gra działa. **Zanim zaprojektujesz funkcję, która dotyczy czegoś z gry, przeczytaj
  odpowiednie configi vanilla** na `P:\` i oprzyj się na nich: nazwy klas, dziedziczenie, wartości domyślne,
  powiązania między klasami.
- Do czytania configów używaj istniejących parserów (`surfaces::parse`, `mission::classes::ClassIndex`,
  parsery CE w `mission/`) zamiast pisać nowe.
- To, co edytor zapisuje, ma mieć ten sam format i strukturę co vanilla, żeby gra i narzędzia BI to przyjęły.
- Jeśli nie wiesz, jak gra coś interpretuje, sprawdź to w configach vanilla albo zapytaj użytkownika. Nie zgaduj.
- Wiedzę o działaniu gry, którą odkryjesz, zapisuj w komentarzu modułu (`//!`), tak jak w `surfaces.rs`.

## Zasady pracy

- Nowa funkcja widoczna dla użytkownika: zaktualizuj `README.md` i/lub `docs/guide.md`, jeśli opisują ten obszar.
- Funkcje eksperymentalne (edytor misji, praca zespołowa) są za ustawieniem
  `Edit > Settings > General > Show experimental features`. Nowe, niedokończone rzeczy też tam chowaj.
- Formaty plików gry i Terrain Buildera (WRP, PAA, P3D, TV4P, `layers.cfg`, `config.cpp`, pliki CE) muszą
  dawać wynik, który przyjmie gra i narzędzia BI. Przy imporcie i eksporcie dodawaj test round-trip, jeśli się da.
- Wydajność ma znaczenie: mapy mają setki tysięcy obiektów. Używaj `rayon` do ciężkich obliczeń i nie
  alokuj w pętli renderowania.
