# DZ World Editor

Edytor map do DayZ napisany w Rust. Łączy DayZForests i DayZ Road Studio w jeden program:
tworzenie terenu, malowanie maski, stawianie obiektów, modyfikatory wysokości, generatory,
widok 2D i 3D, biblioteka modeli, eksport do PBO oraz uruchamianie mapy w grze.

## Wymagania

- Windows 10 lub 11, karta z OpenGL 3.3.
- Do biblioteki modeli i budowania: DayZ Tools z zamontowanym dyskiem `P:\` oraz DayZ.

## Budowanie

```bash
cargo build --release -p dz-editor
```

Program: `target/release/DZWorldEditor.exe`.

## Praca z mapą

| Tryb (klawisz) | Działanie |
| --- | --- |
| Zaznacz (1) | Wybór obiektów, gizmo W/E/R, modyfikator terenu pod zaznaczeniem |
| Rzeźbienie (2) | Pędzle wysokości; Ctrl odwraca, Shift wygładza |
| Malowanie (3) | Malowanie powierzchni maski, warstwy z `P:\dz\surfaces*` |
| Stawianie (4) | Klik lub przeciągnięcie modelu z biblioteki |
| Drogi (5) | Linia punktów budowana z segmentów P3D, obiekty wzdłuż linii |
| Generatory (6) | Teren, erozja, rzeki, malowanie po regułach, rozrzut, las, osada |

Kamera 3D: prawy przycisk i WASD/QE, Shift przyspiesza, kółko przy prawym przycisku zmienia prędkość.
Widok 2D: kółko przybliża, środkowy lub prawy przycisk przesuwa.

## Import

`Plik > Importuj` przyjmuje folder Terrain Buildera (ASC, maska, satelita, `layers.cfg`, listy obiektów, TV4P),
gotową mapę z `P:\` (WRP źródłowe z kaflami), projekty DayZForests i `.dzroad`.

## Budowanie mapy

Przycisk **Zbuduj** eksportuje pliki do `P:\<ID mapy>` i binaryzuje je AddonBuilderem do
`DayZ\@<ID mapy>\addons`. **Uruchom** dodatkowo tworzy misję testową i startuje serwer oraz klienta
`DayZDiag`. Navmesh przygotowuje się raz: `Zbuduj > Generuj navmesh`.

## Dodawanie języka

Plik `%LOCALAPPDATA%\DZWorldEditor\lang\<kod>.json` w postaci
`{"__name__": "Nazwa języka", "File": "Tłumaczenie"}` pojawia się w menu Język bez przebudowy programu.

## Wydania

Tag `vX.Y.Z` uruchamia workflow `.github/workflows/release.yml`, który publikuje `DZWorldEditor.exe`.
Program przy starcie sprawdza najnowsze wydanie i może je zainstalować.

Plan i szczegóły formatów: [docs/PLAN.md](docs/PLAN.md).
