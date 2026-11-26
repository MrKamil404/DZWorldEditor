//! Keeps the number of surfaces on each texture tile within what the terrain material allows.
use crate::export::TileLayout;
use crate::terrain::MaskMap;
use crate::world::World;
use rayon::prelude::*;
use std::collections::HashMap;

/// Pixel span of the tiles along one axis.
#[derive(Clone, Copy, Debug)]
struct Grid {
    step: f32,
    margin: f32,
    tiles: i32,
}

impl Grid {
    fn of(w: &World) -> Self {
        let layout = TileLayout::of(w);
        let px = w.mask.width as f32 / layout.size;
        Self { step: layout.step * px, margin: layout.margin.max(0.0) * px, tiles: layout.tiles as i32 }
    }
    /// Tiles along an axis whose texture, overlap included, shows this pixel.
    fn range(&self, p: f32) -> (i32, i32) {
        let a = ((p - self.margin) / self.step).floor() as i32;
        let b = ((p + self.margin) / self.step).floor() as i32;
        (a.clamp(0, self.tiles - 1), b.clamp(0, self.tiles - 1))
    }
    fn span(&self, t: i32, side: usize) -> (usize, usize) {
        let a = (t as f32 * self.step - self.margin).floor().max(0.0) as usize;
        let b = (((t + 1) as f32 * self.step + self.margin).ceil() as usize).min(side);
        (a, b)
    }
}

fn histogram(mask: &MaskMap, g: &Grid, col: i32, row: i32) -> [u32; 256] {
    let (x0, x1) = g.span(col, mask.width);
    let (y0, y1) = g.span(row, mask.height);
    let mut n = [0u32; 256];
    for y in y0..y1 {
        for v in &mask.data[y * mask.width + x0..y * mask.width + x1] {
            n[*v as usize] += 1;
        }
    }
    n
}

/// One horizontal span `x0..x1` of excess-surface pixels in row `y` of the mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PixelRun {
    pub y: u32,
    pub x0: u32,
    pub x1: u32,
    pub layer: u8,
}

/// Detailed report for one tile that exceeds the project's surface limit.
#[derive(Clone, Debug)]
pub struct TileIssue {
    pub col: u32,
    pub row: u32,
    /// Surfaces present on the tile (overlap included), sorted most used first: `(layer, pixels)`.
    pub surfaces: Vec<(u8, u32)>,
    /// Surfaces that would be removed under the current fix options.
    pub excess_layers: Vec<u8>,
    /// Total number of pixels belonging to `excess_layers` on this tile.
    pub excess_pixels: u32,
    /// Horizontal runs of pixels belonging to `excess_layers`, for highlighting in the viewport.
    pub runs: Vec<PixelRun>,
    /// Mean mask pixel coordinate `[x, y]` of the excess pixels (or tile centre when none).
    pub focus_px: [f32; 2],
}

/// How pixels of removed surfaces are replaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplaceMode {
    /// Each removed pixel takes the nearest kept surface on the tile.
    #[default]
    Nearest,
    /// Every removed pixel becomes the tile's most used kept surface.
    MostUsed,
    /// Every removed pixel becomes one specific surface chosen by the user.
    Layer(u8),
}

/// Which tiles over the limit to fix.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FixScope {
    #[default]
    AllOverLimit,
    SingleTile(u32, u32),
}

/// User-controlled options for checking and fixing tiles over the surface limit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixOptions {
    pub replace: ReplaceMode,
    /// Surfaces to keep on tiles whenever possible (up to the tile limit).
    pub protected: Vec<u8>,
    /// Surfaces to remove before other surfaces when a tile is over the limit.
    pub remove_first: Vec<u8>,
    pub scope: FixScope,
}

/// Summary of what a fix run changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FixReport {
    pub tiles_fixed: usize,
    pub pixels_changed: usize,
    pub remaining_over: usize,
}

/// Refuses paint that would bring a tile over the limit. Lives for one stroke.
pub struct TileGuard {
    limit: usize,
    grid: Grid,
    seen: HashMap<(i32, i32), ([bool; 256], usize)>,
}

impl TileGuard {
    pub fn new(w: &World) -> Self {
        Self { limit: w.meta.max_tile_surfaces.max(1) as usize, grid: Grid::of(w), seen: HashMap::new() }
    }
    pub fn allow(&mut self, mask: &MaskMap, x: usize, y: usize, layer: u8) -> bool {
        let (c0, c1) = self.grid.range(x as f32 + 0.5);
        let (r0, r1) = self.grid.range(y as f32 + 0.5);
        for row in r0..=r1 {
            for col in c0..=c1 {
                let grid = self.grid;
                let entry = self.seen.entry((col, row)).or_insert_with(|| {
                    let n = histogram(mask, &grid, col, row);
                    let mut present = [false; 256];
                    for (i, v) in n.iter().enumerate() {
                        present[i] = *v > 0;
                    }
                    (present, present.iter().filter(|p| **p).count())
                });
                if !entry.0[layer as usize] && entry.1 >= self.limit {
                    return false;
                }
            }
        }
        for row in r0..=r1 {
            for col in c0..=c1 {
                if let Some(e) = self.seen.get_mut(&(col, row)) {
                    if !e.0[layer as usize] {
                        e.0[layer as usize] = true;
                        e.1 += 1;
                    }
                }
            }
        }
        true
    }
}

/// Decides which surfaces of a tile are kept (up to `limit`), which are removed, and what fallback
/// surface receives removed pixels.
fn plan_tile(n: &[u32; 256], limit: usize, opts: &FixOptions) -> ([bool; 256], Vec<u8>, u8) {
    let limit = limit.max(1);
    let mut present: Vec<u8> = (0..256usize).filter(|i| n[*i] > 0).map(|i| i as u8).collect();
    let mut keep = [false; 256];
    if present.len() <= limit {
        for &l in &present {
            keep[l as usize] = true;
        }
        let main = present.iter().copied().max_by_key(|&l| n[l as usize]).unwrap_or(0);
        return (keep, Vec::new(), main);
    }

    let forced_target = match opts.replace {
        ReplaceMode::Layer(target) => Some(target),
        ReplaceMode::Nearest | ReplaceMode::MostUsed => None,
    };

    // Priority tier: 0 = protected, 1 = normal, 2 = remove first. Within a tier, keep the most used.
    let tier = |l: u8| -> u8 {
        if opts.remove_first.contains(&l) {
            2
        } else if opts.protected.contains(&l) {
            0
        } else {
            1
        }
    };
    present.sort_by_key(|&l| (tier(l), std::cmp::Reverse(n[l as usize]), l));

    let mut kept_count = 0usize;
    if let Some(t) = forced_target {
        keep[t as usize] = true;
        kept_count = 1;
    }
    for &l in &present {
        if kept_count >= limit {
            break;
        }
        if !keep[l as usize] {
            keep[l as usize] = true;
            kept_count += 1;
        }
    }

    let excess: Vec<u8> = present.into_iter().filter(|&l| !keep[l as usize]).collect();
    let fallback = match forced_target {
        Some(t) => t,
        None => (0..256usize)
            .filter(|&i| keep[i] && n[i] > 0)
            .max_by_key(|&i| n[i])
            .unwrap_or(0) as u8,
    };
    (keep, excess, fallback)
}

/// Replaces excess pixels in the tile rectangle `(x0..x1, y0..y1)` according to `opts.replace`.
/// Returns how many pixels changed.
fn replace_tile_pixels(mask: &mut MaskMap, x0: usize, x1: usize, y0: usize, y1: usize, keep: &[bool; 256], fallback: u8, mode: ReplaceMode) -> usize {
    let width = mask.width;
    let w = x1.saturating_sub(x0);
    let h = y1.saturating_sub(y0);
    if w == 0 || h == 0 {
        return 0;
    }
    match mode {
        ReplaceMode::MostUsed | ReplaceMode::Layer(_) => {
            let mut changed = 0usize;
            for y in y0..y1 {
                for v in &mut mask.data[y * width + x0..y * width + x1] {
                    if !keep[*v as usize] {
                        *v = fallback;
                        changed += 1;
                    }
                }
            }
            changed
        }
        ReplaceMode::Nearest => {
            let total = w * h;
            let mut fill = vec![u8::MAX; total];
            let mut visited = vec![false; total];
            let mut queue: Vec<usize> = Vec::with_capacity(total / 4);
            let mut excess_count = 0usize;
            for ly in 0..h {
                let row_off = (y0 + ly) * width + x0;
                for lx in 0..w {
                    let v = mask.data[row_off + lx];
                    let idx = ly * w + lx;
                    if keep[v as usize] {
                        fill[idx] = v;
                        visited[idx] = true;
                        queue.push(idx);
                    } else {
                        excess_count += 1;
                    }
                }
            }
            if excess_count == 0 {
                return 0;
            }
            if queue.is_empty() {
                // No kept pixel exists inside the tile: fall back cleanly.
                for y in y0..y1 {
                    for v in &mut mask.data[y * width + x0..y * width + x1] {
                        *v = fallback;
                    }
                }
                return excess_count;
            }
            // Multi-source BFS (8-neighbourhood) from kept pixels into removed pixels.
            let mut head = 0usize;
            while head < queue.len() {
                let idx = queue[head];
                head += 1;
                let src = fill[idx];
                let (lx, ly) = (idx % w, idx / w);
                let ny0 = ly.saturating_sub(1);
                let ny1 = (ly + 1).min(h - 1);
                let nx0 = lx.saturating_sub(1);
                let nx1 = (lx + 1).min(w - 1);
                for ny in ny0..=ny1 {
                    let base = ny * w;
                    for nx in nx0..=nx1 {
                        let ni = base + nx;
                        if !visited[ni] {
                            visited[ni] = true;
                            fill[ni] = src;
                            queue.push(ni);
                        }
                    }
                }
            }
            let mut changed = 0usize;
            for ly in 0..h {
                let row_off = (y0 + ly) * width + x0;
                let f_off = ly * w;
                for lx in 0..w {
                    let v = &mut mask.data[row_off + lx];
                    if !keep[*v as usize] {
                        let next = fill[f_off + lx];
                        *v = if next == u8::MAX { fallback } else { next };
                        changed += 1;
                    }
                }
            }
            changed
        }
    }
}

/// How many tiles use more surfaces than `limit`.
pub fn count_over(w: &World, limit: usize) -> usize {
    let g = Grid::of(w);
    let mask = &w.mask;
    (0..g.tiles).into_par_iter().map(|row| (0..g.tiles).filter(|col| histogram(mask, &g, *col, row).iter().filter(|v| **v > 0).count() > limit).count()).sum()
}

/// Tiles with too many surfaces: (column, row counted from the north, surfaces).
pub fn scan(w: &World) -> Vec<(u32, u32, u32)> {
    let g = Grid::of(w);
    let limit = w.meta.max_tile_surfaces.max(1) as u32;
    let mask = &w.mask;
    (0..g.tiles).into_par_iter().flat_map_iter(|row| (0..g.tiles).filter_map(move |col| {
        let n = histogram(mask, &g, col, row).iter().filter(|v| **v > 0).count() as u32;
        (n > limit).then_some((col as u32, row as u32, n))
    })).collect()
}

/// Detailed scan of every tile over the surface limit, including the exact horizontal pixel runs
/// of the surfaces that would be removed under `opts`.
pub fn scan_detailed(w: &World, opts: &FixOptions) -> Vec<TileIssue> {
    let g = Grid::of(w);
    let limit = w.meta.max_tile_surfaces.max(1) as usize;
    let mask = &w.mask;
    let width = mask.width;
    let mut issues: Vec<TileIssue> = (0..g.tiles)
        .into_par_iter()
        .flat_map_iter(|row| {
            (0..g.tiles).filter_map(move |col| {
                let n = histogram(mask, &g, col, row);
                let mut surfaces: Vec<(u8, u32)> = (0..256usize)
                    .filter(|i| n[*i] > 0)
                    .map(|i| (i as u8, n[i]))
                    .collect();
                if surfaces.len() <= limit {
                    return None;
                }
                surfaces.sort_by_key(|&(l, c)| (std::cmp::Reverse(c), l));
                let (keep, excess_layers, _) = plan_tile(&n, limit, opts);
                let (x0, x1) = g.span(col, mask.width);
                let (y0, y1) = g.span(row, mask.height);
                let mut runs = Vec::new();
                let mut excess_pixels = 0u32;
                let (mut sx, mut sy) = (0.0f64, 0.0f64);
                for y in y0..y1 {
                    let row_slice = &mask.data[y * width..y * width + width];
                    let mut x = x0;
                    while x < x1 {
                        let l = row_slice[x];
                        if !keep[l as usize] {
                            let start = x;
                            while x < x1 && row_slice[x] == l {
                                x += 1;
                            }
                            let len = (x - start) as u32;
                            excess_pixels += len;
                            sx += ((start + x) as f64 * 0.5) * len as f64;
                            sy += (y as f64 + 0.5) * len as f64;
                            runs.push(PixelRun { y: y as u32, x0: start as u32, x1: x as u32, layer: l });
                        } else {
                            x += 1;
                        }
                    }
                }
                let focus_px = if excess_pixels > 0 {
                    [(sx / excess_pixels as f64) as f32, (sy / excess_pixels as f64) as f32]
                } else {
                    [(x0 + x1) as f32 * 0.5, (y0 + y1) as f32 * 0.5]
                };
                Some(TileIssue { col: col as u32, row: row as u32, surfaces, excess_layers, excess_pixels, runs, focus_px })
            })
        })
        .collect();
    issues.sort_by_key(|i| (i.row, i.col));
    issues
}

/// Replaces excess surfaces on tiles over the limit according to `opts`.
pub fn enforce_with(w: &mut World, opts: &FixOptions) -> FixReport {
    let g = Grid::of(w);
    let limit = w.meta.max_tile_surfaces.max(1) as usize;
    let mut tiles_fixed = 0usize;
    let mut pixels_changed = 0usize;

    match opts.scope {
        FixScope::SingleTile(col, row) => {
            if (col as i32) < g.tiles && (row as i32) < g.tiles {
                let n = histogram(&w.mask, &g, col as i32, row as i32);
                let (keep, excess, fallback) = plan_tile(&n, limit, opts);
                if !excess.is_empty() {
                    let (x0, x1) = g.span(col as i32, w.mask.width);
                    let (y0, y1) = g.span(row as i32, w.mask.height);
                    let c = replace_tile_pixels(&mut w.mask, x0, x1, y0, y1, &keep, fallback, opts.replace);
                    if c > 0 {
                        tiles_fixed = 1;
                        pixels_changed = c;
                    }
                }
            }
        }
        FixScope::AllOverLimit => {
            // Overlaps belong to two tiles, so fixing one can touch the next.
            for _ in 0..6 {
                let issues = scan(w);
                if issues.is_empty() {
                    break;
                }
                let mut pass_changed = 0usize;
                for (col, row, _) in issues {
                    let n = histogram(&w.mask, &g, col as i32, row as i32);
                    let (keep, excess, fallback) = plan_tile(&n, limit, opts);
                    if excess.is_empty() {
                        continue;
                    }
                    let (x0, x1) = g.span(col as i32, w.mask.width);
                    let (y0, y1) = g.span(row as i32, w.mask.height);
                    let c = replace_tile_pixels(&mut w.mask, x0, x1, y0, y1, &keep, fallback, opts.replace);
                    if c > 0 {
                        tiles_fixed += 1;
                        pixels_changed += c;
                        pass_changed += c;
                    }
                }
                if pass_changed == 0 {
                    break;
                }
            }
        }
    }

    let remaining_over = count_over(w, limit);
    FixReport { tiles_fixed, pixels_changed, remaining_over }
}

/// Replaces the rarest surfaces of tiles over the limit with the tile's main surface. Returns
/// how many tiles had to change.
pub fn enforce(w: &mut World) -> usize {
    enforce_with(w, &FixOptions { replace: ReplaceMode::MostUsed, ..Default::default() }).tiles_fixed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::WorldMeta;

    fn world() -> World {
        let mut w = World::new(WorldMeta::new("A", 256, 8.0), 0.0);
        w.meta.max_tile_surfaces = 2;
        w
    }

    #[test]
    fn guard_stops_a_third_surface_on_a_tile() {
        let mut w = world();
        w.mask.data[0] = 1;
        let mut g = TileGuard::new(&w);
        assert!(g.allow(&w.mask, 1, 1, 1));
        assert!(!g.allow(&w.mask, 1, 1, 2));
        assert!(g.allow(&w.mask, w.mask.width - 2, w.mask.height - 2, 3));
    }

    #[test]
    fn enforcing_removes_the_rarest_surfaces() {
        let mut w = world();
        for i in 0..50 { w.mask.data[i] = 1; }
        w.mask.data[60] = 2;
        assert_eq!(scan(&w).len(), 1);
        assert_eq!(enforce(&mut w), 1);
        assert!(scan(&w).is_empty());
        assert_eq!(w.mask.data[60], 0);
        assert_eq!(w.mask.data[10], 1);
    }

    #[test]
    fn scan_detailed_reports_excess_pixel_runs_and_respects_protection() {
        let mut w = world();
        // Tile (0, 0): mostly layer 0, 50 pixels of layer 1, 3 pixels of layer 2 at (10..13, 5).
        for i in 0..50 { w.mask.data[i] = 1; }
        let width = w.mask.width;
        for x in 10..13 { w.mask.data[5 * width + x] = 2; }

        let default_issues = scan_detailed(&w, &FixOptions::default());
        assert_eq!(default_issues.len(), 1);
        assert_eq!(default_issues[0].excess_layers, vec![2]);
        assert_eq!(default_issues[0].excess_pixels, 3);
        assert_eq!(default_issues[0].runs, vec![PixelRun { y: 5, x0: 10, x1: 13, layer: 2 }]);

        // Protecting layer 2 makes layer 1 the excess surface instead.
        let opts = FixOptions { protected: vec![2], ..Default::default() };
        let protected_issues = scan_detailed(&w, &opts);
        assert_eq!(protected_issues[0].excess_layers, vec![1]);
        assert_eq!(protected_issues[0].excess_pixels, 50);
    }

    #[test]
    fn nearest_replace_fills_removed_speck_from_its_surroundings() {
        let mut w = world();
        let width = w.mask.width;
        // Layer 0 dominates the tile, a patch of layer 1 covers (10..30, 10..30),
        // and a 2x2 speck of layer 2 sits inside the layer 1 patch at (18..20, 18..20).
        for y in 10..30 {
            for x in 10..30 {
                w.mask.data[y * width + x] = 1;
            }
        }
        for y in 18..20 {
            for x in 18..20 {
                w.mask.data[y * width + x] = 2;
            }
        }
        let report = enforce_with(&mut w, &FixOptions { replace: ReplaceMode::Nearest, ..Default::default() });
        assert_eq!(report.tiles_fixed, 1);
        assert_eq!(report.pixels_changed, 4);
        assert_eq!(report.remaining_over, 0);
        // Nearest mode absorbs the speck into surrounding layer 1, not the distant dominant layer 0.
        assert_eq!(w.mask.data[18 * width + 18], 1);
        assert_eq!(w.mask.data[19 * width + 19], 1);
    }

    #[test]
    fn protected_and_chosen_layer_and_single_tile_scope_work() {
        let mut w = world();
        let width = w.mask.width;
        // Put 3 surfaces on tile (0, 0) and on the far corner tile.
        for x in 0..40 { w.mask.data[x] = 1; }
        w.mask.data[5 * width + 5] = 2;
        let last = w.mask.data.len() - 1;
        for i in 0..40 { w.mask.data[last - i] = 1; }
        w.mask.data[last - width * 5] = 2;
        assert_eq!(scan(&w).len(), 2);

        // Fix only tile (0, 0), replacing removed surface with layer 3 (which stays within limit=2).
        let opts = FixOptions {
            replace: ReplaceMode::Layer(3),
            protected: vec![2],
            remove_first: vec![],
            scope: FixScope::SingleTile(0, 0),
        };
        let r = enforce_with(&mut w, &opts);
        assert_eq!(r.tiles_fixed, 1);
        assert_eq!(r.remaining_over, 1, "far tile was left untouched");
        // On tile (0, 0), layer 2 was protected and layer 3 became the replacement, so layer 2 stayed.
        assert_eq!(w.mask.data[5 * width + 5], 2);
        assert_eq!(w.mask.data[0], 3);
    }
}
