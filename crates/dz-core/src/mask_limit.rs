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

/// Replaces the rarest surfaces of tiles over the limit with the tile's main surface. Returns
/// how many tiles had to change.
pub fn enforce(w: &mut World) -> usize {
    let g = Grid::of(w);
    let limit = w.meta.max_tile_surfaces.max(1) as usize;
    let mut changed = 0;
    // Overlaps belong to two tiles, so fixing one can touch the next.
    for _ in 0..6 {
        let issues = scan(w);
        if issues.is_empty() {
            break;
        }
        changed += issues.len();
        for (col, row, _) in issues {
            let n = histogram(&w.mask, &g, col as i32, row as i32);
            let mut order: Vec<usize> = (0..256).filter(|i| n[*i] > 0).collect();
            order.sort_by_key(|i| std::cmp::Reverse(n[*i]));
            let mut keep = [false; 256];
            for i in order.iter().take(limit) {
                keep[*i] = true;
            }
            let main = order[0] as u8;
            let (x0, x1) = g.span(col as i32, w.mask.width);
            let (y0, y1) = g.span(row as i32, w.mask.height);
            let width = w.mask.width;
            for y in y0..y1 {
                for v in &mut w.mask.data[y * width + x0..y * width + x1] {
                    if !keep[*v as usize] {
                        *v = main;
                    }
                }
            }
        }
    }
    changed
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
}
