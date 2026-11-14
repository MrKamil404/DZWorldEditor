//! Proxies of P3D models in the MLOD form Object Builder saves, before binarising.
//!
//! In MLOD a proxy is a named selection `proxy:<path>.<number>` holding one triangle with a right
//! angle. Binarising turns it into a matrix: the right-angle corner is where the proxy goes, the
//! longer side points up, the shorter one forward, and right is up × forward. The triangle's
//! size does not count.

/// Proxies of the most detailed visual LOD, in the coordinates of the model as saved (not less
/// its centre, as in ODOL): the model path, and the axes right, up, forward then the position.
pub fn visual_proxies(b: &[u8]) -> Result<Vec<(String, [f32; 12])>, String> {
    let mut r = Reader { b, p: 0 };
    if r.bytes(4)? != b"MLOD" {
        return Err("Not an MLOD model".into());
    }
    r.u32()?;
    let lods = r.u32()?;
    if lods > 1000 {
        return Err("Too many LODs in the MLOD model".into());
    }
    let mut best: Option<(f32, Vec<(String, [f32; 12])>)> = None;
    for _ in 0..lods {
        if r.bytes(4)? != b"P3DM" {
            return Err("Unsupported LOD in the MLOD model".into());
        }
        r.bytes(8)?;
        let (nv, nn, nf) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        r.u32()?;
        if nv > 4_000_000 || nn > 4_000_000 || nf > 4_000_000 {
            return Err("The MLOD model is too large".into());
        }
        let mut points = Vec::with_capacity(nv);
        for _ in 0..nv {
            points.push([r.f32()?, r.f32()?, r.f32()?]);
            r.u32()?;
        }
        r.bytes(nn * 12)?;
        let mut faces = Vec::with_capacity(nf);
        for _ in 0..nf {
            let n = r.u32()? as usize;
            let mut face = [0usize; 4];
            for v in &mut face {
                *v = r.u32()? as usize;
                r.bytes(12)?;
            }
            r.u32()?;
            r.string()?;
            r.string()?;
            faces.push((n, face));
        }
        if r.bytes(4)? != b"TAGG" {
            return Err("No tags in the MLOD model".into());
        }
        let mut found = Vec::new();
        loop {
            r.bytes(1)?;
            let name = r.string()?;
            let len = r.u32()? as usize;
            let data = r.bytes(len)?;
            if name == "#EndOfFile#" {
                break;
            }
            let Some(path) = name.get(..6).filter(|p| p.eq_ignore_ascii_case("proxy:")).map(|_| &name[6..]) else { continue };
            // The selection weights: one byte per point, then one per face.
            let face = (0..nf).find(|&i| data.get(nv + i).is_some_and(|w| *w != 0)).map(|i| faces[i]);
            let corners: Vec<usize> = match face {
                Some((n, f)) if n >= 3 => f[..3].to_vec(),
                _ => (0..nv).filter(|&i| data.get(i).is_some_and(|w| *w != 0)).take(3).collect(),
            };
            let Some(tri) = corners.iter().map(|&i| points.get(i).copied()).collect::<Option<Vec<[f32; 3]>>>().filter(|t| t.len() == 3) else { continue };
            if let Some(m) = matrix(&tri) {
                found.push((model_of(path), m));
            }
        }
        let resolution = r.f32()?;
        if resolution < 1000.0 && best.as_ref().is_none_or(|b| resolution < b.0) {
            best = Some((resolution, found));
        }
    }
    Ok(best.map(|b| b.1).unwrap_or_default())
}

/// `\dz\...\desk.001` names the model `\dz\...\desk`.
fn model_of(path: &str) -> String {
    match path.rsplit_once('.') {
        Some((model, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => model.to_string(),
        _ => path.to_string(),
    }
}

/// The matrix of a proxy triangle: right, up, forward and the position of its right angle.
fn matrix(t: &[[f32; 3]]) -> Option<[f32; 12]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    // The corner opposite the longest side.
    let side = |i: usize| len(sub(t[(i + 1) % 3], t[(i + 2) % 3]));
    let o = (0..3).max_by(|&a, &b| side(a).total_cmp(&side(b)))?;
    let (a, b) = (sub(t[(o + 1) % 3], t[o]), sub(t[(o + 2) % 3], t[o]));
    let (up, fwd) = if len(a) >= len(b) { (a, b) } else { (b, a) };
    let (lu, lf) = (len(up), len(fwd));
    if lu < 1e-6 || lf < 1e-6 {
        return None;
    }
    let up = [up[0] / lu, up[1] / lu, up[2] / lu];
    let fwd = [fwd[0] / lf, fwd[1] / lf, fwd[2] / lf];
    let right = [up[1] * fwd[2] - up[2] * fwd[1], up[2] * fwd[0] - up[0] * fwd[2], up[0] * fwd[1] - up[1] * fwd[0]];
    let p = t[o];
    Some([right[0], right[1], right[2], up[0], up[1], up[2], fwd[0], fwd[1], fwd[2], p[0], p[1], p[2]])
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.p..self.p + n).ok_or("The MLOD model ends too early")?;
        self.p += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, String> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let rest = self.b.get(self.p..).ok_or("The MLOD model ends too early")?;
        let n = rest.iter().position(|&c| c == 0).ok_or("An unended name in the MLOD model")?;
        self.p += n + 1;
        Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn triangles_turn_into_the_matrices_binarising_makes() {
        // Proxies of one building as Object Builder saved them, and as the binarised model has
        // them (less its centre 0.4514, 2.8068, 1.1893).
        let desk = matrix(&[[7.413648, 0.7045008, 0.96827954], [7.413648, 2.7045007, 0.96827954], [6.413648, 0.7045008, 0.96827954]]).unwrap();
        assert!(close(&desk[..9], &[0.0, 0.0, 1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 0.0]), "{desk:?}");
        assert!(close(&desk[9..], &[7.413648, 0.7045008, 0.96827954]));
        // A triangle drawn smaller and with its corners in another order.
        let table = matrix(&[[6.949223, 2.5392962, 2.5585294], [6.949223, 0.7254521, 2.5585294], [6.949223, 0.7254521, 3.3380277]]).unwrap();
        assert!(close(&table[..9], &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]), "{table:?}");
        assert!(close(&table[9..], &[6.949223, 0.7254521, 2.5585294]));
    }

    #[test]
    fn proxy_names_lose_their_number() {
        assert_eq!(model_of("\\DZ\\structures\\furniture\\beds\\postel_panelak1.001"), "\\DZ\\structures\\furniture\\beds\\postel_panelak1");
        assert_eq!(model_of("\\a\\b.p3d"), "\\a\\b.p3d");
    }
}
