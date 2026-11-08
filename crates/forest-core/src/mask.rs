//! Wczytywanie maski satelitarnej (PNG/BMP/TGA/JPG) i mapowanie kolorów na strefy.

use std::path::Path;

/// Kolor RGB 8-bit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Rgb8(pub [u8; 3]);

impl Rgb8 {
    pub fn r(&self) -> u8 {
        self.0[0]
    }
    pub fn g(&self) -> u8 {
        self.0[1]
    }
    pub fn b(&self) -> u8 {
        self.0[2]
    }

    /// Sumaryczna różnica kanałów (metryka Manhattan).
    pub fn dist(&self, other: &Rgb8) -> u32 {
        (i32::from(self.0[0]) - i32::from(other.0[0])).unsigned_abs()
            + (i32::from(self.0[1]) - i32::from(other.0[1])).unsigned_abs()
            + (i32::from(self.0[2]) - i32::from(other.0[2])).unsigned_abs()
    }
}

/// Surowa maska w RGBA8.
#[derive(Clone)]
pub struct MaskImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl MaskImage {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        // Maski mapowe bywają >500 MB po dekodowaniu (np. 15360x15360 RGBA) —
        // domyślny limit pamięci dekodera trzeba zdjąć.
        let mut reader = image::ImageReader::open(path)
            .map_err(|e| format!("Nie udało się otworzyć maski: {e}"))?;
        reader.no_limits();
        let img = reader
            .decode()
            .map_err(|e| format!("Nie udało się wczytać maski: {e}"))?;
        Ok(Self::from_dynamic(img))
    }

    pub fn from_dynamic(img: image::DynamicImage) -> Self {
        let rgba = img.to_rgba8();
        Self {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        }
    }

    #[inline]
    pub fn pixel(&self, x: u32, y: u32) -> Rgb8 {
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Rgb8([self.rgba[idx], self.rgba[idx + 1], self.rgba[idx + 2]])
    }

    #[inline]
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        let idx = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        self.rgba[idx + 3]
    }

    /// Histogram kolorów (bez alfa < 128), sortowany malejąco po liczbie pikseli.
    pub fn color_histogram(&self, min_pixels: usize) -> Vec<(Rgb8, usize)> {
        use std::collections::HashMap;
        let mut hist: HashMap<Rgb8, usize> = HashMap::new();
        for y in 0..self.height {
            for x in 0..self.width {
                if self.alpha(x, y) < 128 {
                    continue;
                }
                *hist.entry(self.pixel(x, y)).or_insert(0) += 1;
            }
        }
        let mut v: Vec<(Rgb8, usize)> = hist.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.retain(|(_, c)| *c >= min_pixels);
        v
    }

    #[inline]
    pub fn pixel_size_m(&self, map_size_m: f64) -> f64 {
        map_size_m / f64::from(self.width.max(1))
    }

    /// Świat (m, origin SW) -> piksel maski. Zwraca None poza zakresem.
    pub fn world_to_pixel_f(&self, wx: f64, wy: f64, map_size_m: f64) -> Option<(f64, f64)> {
        if wx < 0.0 || wy < 0.0 || wx >= map_size_m || wy >= map_size_m {
            return None;
        }
        let s = f64::from(self.width) / map_size_m;
        let px = wx * s;
        // wiersz 0 maski = północ = max Y świata
        let py = (map_size_m - wy) * (f64::from(self.height) / map_size_m);
        Some((px, py))
    }

    /// Dopasowuje piksel do palety stref z tolerancją. Zwraca indeks strefy.
    /// `palette[i]` to kolor strefy `i`. Alpha < 128 => None.
    pub fn match_palette(
        &self,
        px: f64,
        py: f64,
        palette: &[Rgb8],
        tolerance: u32,
    ) -> Option<usize> {
        if palette.is_empty() {
            return None;
        }
        let xi = px.floor().clamp(0.0, f64::from(self.width - 1)) as u32;
        let yi = py.floor().clamp(0.0, f64::from(self.height - 1)) as u32;
        if self.alpha(xi, yi) < 128 {
            return None;
        }
        let c = self.pixel(xi, yi);
        let mut best: Option<(usize, u32)> = None;
        for (i, p) in palette.iter().enumerate() {
            let d = c.dist(p);
            if d <= tolerance && best.map_or(true, |(_, bd)| d < bd) {
                best = Some((i, d));
            }
        }
        best.map(|(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_mask(w: u32, h: u32, colors: &[(Rgb8, u32, u32, u32, u32)]) -> MaskImage {
        // prostokąty (color, x0, y0, w, h)
        let mut img = image::RgbaImage::new(w, h);
        for (c, x0, y0, rw, rh) in colors {
            for y in *y0..(*y0 + *rh) {
                for x in *x0..(*x0 + *rw) {
                    img.put_pixel(x, y, image::Rgba([c.r(), c.g(), c.b(), 255]));
                }
            }
        }
        MaskImage::from_dynamic(image::DynamicImage::ImageRgba8(img))
    }

    #[test]
    fn histogram_and_palette() {
        let red = Rgb8([255, 0, 0]);
        let green = Rgb8([0, 255, 0]);
        let m = solid_mask(16, 16, &[(red, 0, 0, 8, 16), (green, 8, 0, 8, 16)]);
        let hist = m.color_histogram(1);
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].1, 128);

        let zone = m.match_palette(4.0, 4.0, &[green, red], 0);
        assert_eq!(zone, Some(1));
        let zone = m.match_palette(12.0, 12.0, &[green, red], 0);
        assert_eq!(zone, Some(0));
    }

    #[test]
    fn tolerance_matching() {
        let base = Rgb8([10, 200, 30]);
        let noisy = Rgb8([13, 197, 33]); // dist = 9
        let far = Rgb8([250, 250, 250]);
        let m = solid_mask(8, 8, &[(noisy, 0, 0, 8, 8)]);
        assert_eq!(m.match_palette(4.0, 4.0, &[base], 10), Some(0));
        assert_eq!(m.match_palette(4.0, 4.0, &[far], 10), None);
    }

    #[test]
    fn world_to_pixel_north_is_row_zero() {
        let m = solid_mask(10, 10, &[]);
        // świat 100m; punkt przy północy (y=95) powinien trafić w wiersz ~0
        let (px, py) = m.world_to_pixel_f(50.0, 95.0, 100.0).unwrap();
        assert!((py - 0.5).abs() < 0.6, "py={py}");
        assert!((px - 5.0).abs() < 0.6);
        assert!(m.world_to_pixel_f(-1.0, 5.0, 100.0).is_none());
        assert!(m.world_to_pixel_f(5.0, 101.0, 100.0).is_none());
    }

    #[test]
    fn png_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.png");
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255]));
        img.save(&path).unwrap();
        let m = MaskImage::load(&path).unwrap();
        assert_eq!((m.width, m.height), (4, 4));
        assert_eq!(m.pixel(0, 0), Rgb8([1, 2, 3]));
    }

    #[test]
    fn bmp_24bit_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.bmp");
        let img = image::RgbaImage::from_pixel(5, 3, image::Rgba([9, 77, 200, 255]));
        img.save(&path).unwrap();
        let m = MaskImage::load(&path).unwrap();
        assert_eq!((m.width, m.height), (5, 3));
        assert_eq!(m.pixel(2, 1), Rgb8([9, 77, 200]));
    }

    /// Ręcznie buduje 8-bitowy paletowy BMP 2x2 (typowy eksport masek
    /// z edytorów grafiki) i sprawdza dekodowanie.
    #[test]
    fn bmp_paletted_8bit() {
        const RED: [u8; 3] = [255, 0, 0];
        const GREEN: [u8; 3] = [0, 255, 0];
        const BLACK: [u8; 3] = [0, 0, 0];

        let palette_entries = 256usize;
        let row_padded = 4usize; // wyrównanie do 4 B
        let data_size = row_padded * 2;
        let pixel_offset = (14 + 40 + palette_entries * 4) as u32;
        let file_size = pixel_offset + data_size as u32;

        let mut b: Vec<u8> = Vec::new();
        b.extend_from_slice(b"BM");
        b.extend_from_slice(&file_size.to_le_bytes());
        b.extend_from_slice(&[0u8; 4]);
        b.extend_from_slice(&pixel_offset.to_le_bytes());
        b.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
        b.extend_from_slice(&2i32.to_le_bytes()); // width
        b.extend_from_slice(&2i32.to_le_bytes()); // height
        b.extend_from_slice(&1u16.to_le_bytes()); // planes
        b.extend_from_slice(&8u16.to_le_bytes()); // bpp
        b.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        b.extend_from_slice(&(data_size as u32).to_le_bytes());
        b.extend_from_slice(&[0u8; 8]); // ppm
        b.extend_from_slice(&(palette_entries as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        for i in 0..palette_entries {
            let c = match i {
                0 => RED,
                1 => GREEN,
                _ => BLACK,
            };
            b.extend_from_slice(&[c[2], c[1], c[0], 0]); // BGRA
        }
        // wiersze od dołu: dolny rząd [zielony, czarny], górny [czerwony, zielony]
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&[0, 1, 0, 0]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pal8.bmp");
        std::fs::write(&path, &b).unwrap();

        let m = MaskImage::load(&path).unwrap();
        assert_eq!((m.width, m.height), (2, 2));
        assert_eq!(m.pixel(0, 0), Rgb8(RED)); // górny rząd = ostatni w pliku
        assert_eq!(m.pixel(1, 0), Rgb8(GREEN));
        assert_eq!(m.pixel(0, 1), Rgb8(GREEN)); // dolny rząd = pierwszy w pliku
        assert_eq!(m.pixel(1, 1), Rgb8(RED));
    }
}
