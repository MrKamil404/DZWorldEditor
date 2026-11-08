# DZ World Editor

A DayZ map editor written in Rust. It combines DayZForests and DayZ Road Studio in one program:
terrain creation, mask painting, object placement, height modifiers, generators, 2D and 3D views,
a model library, PBO export and running the map in the game.

## Requirements

- Windows 10 or 11 and a graphics card with OpenGL 3.3.
- For the model library and building: DayZ Tools with the `P:\` work drive mounted, and DayZ.

## Building

```bash
cargo build --release -p dz-editor
```

The program is `target/release/DZWorldEditor.exe`.

## Working on a map

| Mode (key) | What it does |
| --- | --- |
| Select (1) | Select objects, W/E/R gizmo, terrain modifier under the selection |
| Sculpt (2) | Height brushes; Ctrl inverts, Shift smooths |
| Paint (3) | Paint mask surfaces, layers from `P:\dz\surfaces*` |
| Place (4) | Click, or drag a model from the library |
| Roads (5) | A line of points built from P3D road parts, objects along a line |
| Generate (6) | Terrain, erosion, rivers, rule-based painting, scatter, forest, settlement |

3D camera: right mouse button with WASD/QE, Shift goes faster, the wheel changes speed while the right button is held.
2D view: the wheel zooms, the middle or right button pans.

## Import

`File > Import` accepts a Terrain Builder folder (ASC, mask, satellite image, `layers.cfg`, object lists, TV4P),
a built map on `P:\` (source WRP with its tiles), and DayZForests and `.dzroad` projects.
Binarised worlds (`OPRW`) cannot be opened.

## Building a map

**Build** exports the files to `P:\<map id>` and binarises them with AddonBuilder into
`DayZ\@<map id>\addons`. **Play** also creates a test mission and starts a `DayZDiag` server and client.

The navmesh is prepared separately with `Build > Generate navmesh`: the editor starts the data server
and NavMeshGenerator, and the mesh is generated and saved as `navmesh.nm` in that tool.
Without a navmesh the map runs, but AI does not walk.

## Adding a language

A file `%LOCALAPPDATA%\DZWorldEditor\lang\<code>.json` of the form
`{"__name__": "Language name", "File": "Translation"}` appears in the Language menu without rebuilding the program.

## Releases

A `vX.Y.Z` tag runs `.github/workflows/release.yml`, which publishes `DZWorldEditor.exe`.
The program checks for the latest release at start and can install it.
