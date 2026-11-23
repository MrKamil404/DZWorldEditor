# Maps of real places

Choose **File > New project from a real place** (or **Real place** in the New project window) to
make a map of a real square of the Earth, of one of the DayZ map sizes. Nothing needs an account
or a key: the data comes from free map services.

* Find the place by name or type its latitude and longitude (`49.27, 19.98`). On the picture,
  drag to move, scroll to zoom and double-click to put the map there. The blue square is the map.
* **Satellite picture**, from:
  * Esri World Imagery: sharp, the whole world, but its terms do not allow publishing it in a
    map, so use it to trace and for tests;
  * Sentinel-2 cloudless 2016: the whole world, 10 m pixels, CC BY 4.0 (may be published with
    its attribution);
  * national orthophotos, sharp and open, each only for its own country: Poland (Geoportal,
    GUGiK), Czechia (ČÚZK), Austria (basemap.at) and the USA (USGS);
  * your own tile server: an address with `{z}`, `{x}` and `{y}`.

  The window shows the licence of each source and the attribution to give. Choose the pixel
  size; the window says when the picture needs a lot of memory.
* **Surface mask**, made in two steps:
  * **From the satellite picture**, with the same settings as **Surfaces from the satellite** in
    the Paint tab: natural colours (dense or leafy forest, grass, pale grass, soil, fields,
    rock, roads and roofs), your own picked colours, or colour groups, each with its surface.
    The preview is made from the picture you choose the place on, so it is coarser than the
    result. The Paint tab of the new project keeps these settings, to read the mask again.
  * **From the OpenStreetMap shapes, over the satellite**: each kind of shape (fields, meadows,
    scrub, built-up land, sand, rock, leafy and conifer forests, water, main and minor roads,
    tracks, paths, railways) can be given a surface, and lines a width. They are painted in
    that order over what the satellite gave, so forests and roads come out with clean edges.
    OpenStreetMap tells conifer from leafy forests where its `leaf_type` is set.

  Fix the mask later with the Paint tab.
* **Heights** come from the AWS Terrain Tiles: about 30 m detail (SRTM) in most places, finer in
  some countries. The sea is at 0 m and the sea floor stops at -100 m.
* **OpenStreetMap shapes**: roads, railways, rivers and streams, lakes, forests, land use and
  buildings, each kind in its own shape layer with the OpenStreetMap tags as attributes. Forests
  and land use are split further: `OSM leafy forests`, `OSM conifer forests`, `OSM fields`,
  `OSM meadows`, `OSM scrub`, `OSM built-up land`, `OSM sand` and `OSM rock`, and waterways into
  `OSM rivers` (rivers, canals) and `OSM streams` (streams, ditches). They are cut to
  the map. Use them for generators, roads and masks. The data is © OpenStreetMap contributors
  (ODbL). Clearings inside a forest or a lake inside a meadow come in as holes of their shape:
  the mask and the forest generator leave them out. In the mask, smaller areas lie over larger
  ones, so a meadow inside a forest stays a meadow.
* **Buildings**: each building of OpenStreetMap gets the building of the game nearest its size,
  from the folders of its kind (houses, blocks of flats, shops and offices, industry, farm
  buildings, sheds and garages, churches, schools and services), standing in its outline and
  turned along it, on the object layer `OSM buildings`. Buildings much smaller or larger than any
  model of their kind are left out. **Level the ground under them** adds a terrain modifier
  under each, as Terrain under objects does.
* **Roads**: the roads (and railways, if you choose a track type) of OpenStreetMap are built of
  the game's road pieces once the map opens, one after another, with the ground graded under
  them. Choose the road type for main roads, minor roads, tracks and paths. Each road is laid
  along its line without searching the terrain. Pieces of one road are joined first, points too
  close for a bend are left out, the same road drawn twice is built once, and roundabouts stay as
  shapes. A road that still cannot be fitted stays a shape too: the Output Log says how many.
  Crossings get no crossroad pieces, so fix them by hand with the Roads tab.
* **Generators**: with **Add generators for the OpenStreetMap shapes**, the new map gets a forest
  generator on the leafy forests, the conifer forests and the scrub (each with a forest preset you
  choose), the Fields generator on the fields, the River generator on the rivers and canals, the
  Stream generator on the streams and, if you want, the Ponds and lakes generator on the lakes
  (it digs a basin, which the real heights mostly have already). Forests keep away from the
  buildings and roads. Each works on its layer of shapes, so it can be changed and run again from
  the outliner. **Run them when the map opens** runs them one after another, after the roads;
  stopping one stops the rest.
* The map's latitude and longitude are set to the place, so the sun and stars are right in the
  game. Downloaded tiles are kept in `%LOCALAPPDATA%\DZWorldEditor\tiles`, so the same place
  downloads quickly the second time.

[Back to the guide](guide.md)
