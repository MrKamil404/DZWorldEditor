//! Parser heightmapy ESRI ASCII Grid (.asc) + próbkowanie wysokości i spadku.

use std::path::Path;

#[derive(Clone, Debug)]
pub struct AscHeightmap {
    pub ncols: usize,
    pub nrows: usize,
    pub xllcorner: f64,
    pub yllcorner: f64,
    pub cellsize: f64,
    pub nodata: f64,
    /// Wiersz 0 = północ (góra pliku ASC).
    pub values: Vec<f32>,
    /// Przesunięcie dodawane do współrzędnych świata przed porównaniem z nagłówkiem.
    /// Ustawiane automatycznie: jeśli xllcorner >= 100000 przyjmuje się, że ASC
    /// używa współrzędnych z offsetem easting (np. 200000 jak w Terrain Builderze).
    pub x_origin_offset: f64,
    pub y_origin_offset: f64,
}

impl AscHeightmap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut ncols = None;
        let mut nrows = None;
        let mut xll = 0.0;
        let mut yll = 0.0;
        let mut cellsize = None;
        let mut nodata = -9999.0;
        let mut values: Vec<f32> = Vec::new();

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let lower = line.to_lowercase();
            if lower.starts_with("ncols") {
                ncols = Some(parse_hdr(&lower, "ncols")?);
            } else if lower.starts_with("nrows") {
                nrows = Some(parse_hdr(&lower, "nrows")?);
            } else if lower.starts_with("xllcorner") {
                xll = parse_hdr_f(&lower, "xllcorner")?;
            } else if lower.starts_with("yllcorner") {
                yll = parse_hdr_f(&lower, "yllcorner")?;
            } else if lower.starts_with("cellsize") {
                cellsize = Some(parse_hdr_f(&lower, "cellsize")?);
            } else if lower.starts_with("nodata_value") {
                nodata = parse_hdr_f(&lower, "nodata_value")?;
            } else {
                for tok in line.split_whitespace() {
                    let v: f32 = tok
                        .parse()
                        .map_err(|_| format!("ASC: nieprawidłowa wartość '{tok}'"))?;
                    values.push(v);
                }
            }
        }

        let ncols = ncols.ok_or("ASC: brak 'ncols'")?;
        let nrows = nrows.ok_or("ASC: brak 'nrows'")?;
        let cellsize = cellsize.ok_or("ASC: brak 'cellsize'")?;
        if ncols == 0 || nrows == 0 || cellsize <= 0.0 {
            return Err("ASC: nieprawidłowy nagłówek".into());
        }
        if values.len() != ncols * nrows {
            return Err(format!(
                "ASC: oczekiwano {} wartości, wczytano {}",
                ncols * nrows,
                values.len()
            ));
        }

        // Heurystyka: duże xll => ASC w układzie z offsetem easting (TB).
        let (xo, yo) = if xll >= 100_000.0 || yll >= 100_000.0 {
            (xll, yll)
        } else {
            (0.0, 0.0)
        };

        Ok(Self {
            ncols,
            nrows,
            xllcorner: xll,
            yllcorner: yll,
            cellsize,
            nodata,
            values,
            x_origin_offset: xo,
            y_origin_offset: yo,
        })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("Nie udało się wczytać ASC: {e}"))?;
        Self::parse(&text)
    }

    /// Wartości są w f32, a NODATA w f64 — porównanie musi być w precyzji f32,
    /// inaczej ułamkowe NODATA (np. -9999.9) udają prawdziwą wysokość.
    #[inline]
    fn is_nodata(&self, v: f32) -> bool {
        let nodata = self.nodata as f32;
        !v.is_finite() || v == nodata || (v - nodata).abs() <= nodata.abs() * f32::EPSILON
    }

    #[inline]
    fn at(&self, col: i64, row: i64) -> Option<f32> {
        if col < 0 || row < 0 || col >= self.ncols as i64 || row >= self.nrows as i64 {
            return None;
        }
        Some(self.values[(row as usize) * self.ncols + (col as usize)])
    }

    /// Próbkowanie dwuliniowe (wartości ASC traktowane jako środki komórek).
    /// `wx`, `wy` — współrzędne mapy w metrach (origin SW).
    pub fn sample(&self, wx: f64, wy: f64) -> Option<f64> {
        let fx = wx + self.x_origin_offset - self.xllcorner;
        let fy = wy + self.y_origin_offset - self.yllcorner;
        let size_x = f64::from(self.ncols as u32) * self.cellsize;
        let size_y = f64::from(self.nrows as u32) * self.cellsize;
        if !(0.0..=size_x).contains(&fx) || !(0.0..=size_y).contains(&fy) {
            return None;
        }
        if self.ncols < 2 || self.nrows < 2 {
            return None; // degeneracja — wymagane >= 2x2
        }

        // Indeksy węzłów (środków komórek)
        let gx = fx / self.cellsize - 0.5;
        let gy = fy / self.cellsize - 0.5;
        let mut col = gx.floor() as i64;
        let mut rfb = gy.floor() as i64; // row-from-bottom
        let mut tx = gx - col as f64;
        let mut ty = gy - rfb as f64;
        if col < 0 {
            col = 0;
            tx = 0.0;
        }
        if col > self.ncols as i64 - 2 {
            col = self.ncols as i64 - 2;
            tx = 1.0;
        }
        if rfb < 0 {
            rfb = 0;
            ty = 0.0;
        }
        if rfb > self.nrows as i64 - 2 {
            rfb = self.nrows as i64 - 2;
            ty = 1.0;
        }
        // wiersz 0 pliku = północ = góra siatki
        let row = self.nrows as i64 - 1 - rfb;

        let h00 = self.at(col, row)?; // lewy-dolny
        let h10 = self.at(col + 1, row)?;
        let h01 = self.at(col, row - 1)?; // lewy-górny
        let h11 = self.at(col + 1, row - 1)?;

        if [h00, h10, h01, h11].into_iter().any(|h| self.is_nodata(h)) {
            return None;
        }

        let bottom = h00 + (h10 - h00) * tx as f32;
        let top = h01 + (h11 - h01) * tx as f32;
        Some(f64::from(bottom + (top - bottom) * ty as f32))
    }

    /// Spadek w stopniach (centralna różnica o rozmiar komórki).
    pub fn slope_deg(&self, wx: f64, wy: f64) -> Option<f64> {
        let d = self.cellsize;
        let hx1 = self.sample(wx + d * 0.5, wy)?;
        let hx0 = self.sample(wx - d * 0.5, wy)?;
        let hy1 = self.sample(wx, wy + d * 0.5)?;
        let hy0 = self.sample(wx, wy - d * 0.5)?;
        let gx = (hx1 - hx0) / d;
        let gy = (hy1 - hy0) / d;
        Some(gx.hypot(gy).atan().to_degrees())
    }

    pub fn min_max(&self) -> (f32, f32) {
        let mut mn = f32::INFINITY;
        let mut mx = f32::NEG_INFINITY;
        for &v in &self.values {
            if self.is_nodata(v) {
                continue;
            }
            mn = mn.min(v);
            mx = mx.max(v);
        }
        if !mn.is_finite() {
            (0.0, 0.0)
        } else {
            (mn, mx)
        }
    }
}

fn parse_hdr(line: &str, key: &str) -> Result<usize, String> {
    let v = parse_hdr_f(line, key)?;
    if v < 0.0 || v.fract() != 0.0 {
        return Err(format!("ASC: {key} musi być nieujemną liczbą całkowitą"));
    }
    Ok(v as usize)
}

fn parse_hdr_f(line: &str, key: &str) -> Result<f64, String> {
    line[key.len()..]
        .trim()
        .parse()
        .map_err(|_| format!("ASC: nieprawidłowa wartość dla '{key}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "ncols 3\nnrows 3\nxllcorner 0\nyllcorner 0\ncellsize 10\nNODATA_value -9999\n\
        100 110 120\n\
        50 60 70\n\
        0 10 20\n";

    #[test]
    fn parses_header_and_values() {
        let hm = AscHeightmap::parse(SAMPLE).unwrap();
        assert_eq!((hm.ncols, hm.nrows), (3, 3));
        assert_eq!(hm.cellsize, 10.0);
        assert_eq!(hm.values.len(), 9);
        assert_eq!(hm.x_origin_offset, 0.0);
    }

    #[test]
    fn samples_cell_centers_and_bilinear() {
        let hm = AscHeightmap::parse(SAMPLE).unwrap();
        // środek mapy = (15,15) -> dokładnie wartość centralna 60
        let h = hm.sample(15.0, 15.0).unwrap();
        assert!((h - 60.0).abs() < 1e-4, "h={h}");
        // róg SW (0,0) -> 0
        let h = hm.sample(0.0, 0.0).unwrap();
        assert!(h.abs() < 1e-4);
        // róg NE (30,30) -> 120
        let h = hm.sample(30.0, 30.0).unwrap();
        assert!((h - 120.0).abs() < 1e-4, "h={h}");
        // poza mapą
        assert!(hm.sample(-1.0, 5.0).is_none());
        assert!(hm.sample(31.0, 5.0).is_none());
    }

    #[test]
    fn slope_on_flat_is_zero_and_on_ramp_positive() {
        let flat = AscHeightmap::parse(
            "ncols 3\nnrows 3\nxllcorner 0\nyllcorner 0\ncellsize 10\nNODATA_value -9999\n\
             5 5 5\n5 5 5\n5 5 5\n",
        )
        .unwrap();
        assert!(flat.slope_deg(15.0, 15.0).unwrap() < 0.01);

        let hm = AscHeightmap::parse(SAMPLE).unwrap();
        // gradient pionowy 50m/10m dominuje => stromy zbocz ~79 stopni
        let s = hm.slope_deg(15.0, 15.0).unwrap();
        assert!(s > 70.0 && s < 85.0, "s={s}");
    }

    #[test]
    fn offset_heuristic_for_tb_style_asc() {
        let tb = AscHeightmap::parse(
            "ncols 2\nnrows 2\nxllcorner 200000\nyllcorner 0\ncellsize 100\nNODATA_value -9999\n\
             1 2\n3 4\n",
        )
        .unwrap();
        assert_eq!(tb.x_origin_offset, 200000.0);
        // świat (0,0) powinien czytać komórkę SW
        let h = tb.sample(0.0, 0.0).unwrap();
        assert!((h - 3.0).abs() < 1e-4, "h={h}");
    }

    #[test]
    fn fractional_nodata_is_not_a_height() {
        let hm = AscHeightmap::parse(
            "ncols 2\nnrows 2\nxllcorner 0\nyllcorner 0\ncellsize 10\nNODATA_value -9999.9\n\
             -9999.9 -9999.9\n-9999.9 12.5\n",
        )
        .unwrap();
        assert!(hm.sample(10.0, 10.0).is_none());
        assert_eq!(hm.min_max(), (12.5, 12.5));
    }

    #[test]
    fn rejects_bad_files() {
        assert!(AscHeightmap::parse("ncols 2\n").is_err());
        assert!(AscHeightmap::parse("ncols 2\nnrows 2\ncellsize 1\n1 2 3\n").is_err());
    }
}
