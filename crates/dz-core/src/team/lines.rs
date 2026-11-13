//! Three-way merge of text files line by line, as `diff3` does it, for the files of the mission.
use std::collections::HashMap;

/// The result of merging two changed copies of a text.
#[derive(Debug, PartialEq)]
pub struct Merged3 {
    /// The merge, taking your lines where both changed the same lines.
    pub mine: Vec<u8>,
    /// The merge, taking the other person's lines there.
    pub theirs: Vec<u8>,
    /// Places where both changed the same lines differently.
    pub conflicts: usize,
}

/// More differences than this and the files are treated as rewritten.
const MAX_EDITS: usize = 6000;

/// Merges what changed from `base` to `mine` and from `base` to `theirs`.
pub fn merge<'a>(base: &'a [u8], mine: &'a [u8], theirs: &'a [u8]) -> Merged3 {
    let (b, m, t) = (split(base), split(mine), split(theirs));
    let mut ids: HashMap<&'a [u8], u32> = HashMap::new();
    let mut number = |lines: &[&'a [u8]]| -> Vec<u32> {
        lines.iter().map(|l| {
            let n = ids.len() as u32;
            *ids.entry(*l).or_insert(n)
        }).collect()
    };
    let bi = number(&b);
    let mi = number(&m);
    let ti = number(&t);
    let mut to_m = vec![None; b.len()];
    for (i, j) in common(&bi, &mi) {
        to_m[i] = Some(j);
    }
    let mut to_t = vec![None; b.len()];
    for (i, j) in common(&bi, &ti) {
        to_t[i] = Some(j);
    }
    let mut out = Merged3 { mine: Vec::new(), theirs: Vec::new(), conflicts: 0 };
    let both = |out: &mut Merged3, lines: &[&[u8]]| {
        for l in lines {
            out.mine.extend_from_slice(l);
            out.theirs.extend_from_slice(l);
        }
    };
    let (mut i, mut a, mut c) = (0, 0, 0);
    loop {
        if i < b.len() && to_m[i] == Some(a) && to_t[i] == Some(c) {
            both(&mut out, &[b[i]]);
            (i, a, c) = (i + 1, a + 1, c + 1);
            continue;
        }
        // The next line of the base both copies kept.
        let mut j = i;
        while j < b.len() && (to_m[j].is_none() || to_t[j].is_none()) {
            j += 1;
        }
        let (ae, ce) = if j < b.len() { (to_m[j].unwrap(), to_t[j].unwrap()) } else { (m.len(), t.len()) };
        let (bs, ms, ts) = (&b[i..j], &m[a..ae], &t[c..ce]);
        if ms == bs {
            both(&mut out, ts);
        } else if ts == bs || ms == ts {
            both(&mut out, ms);
        } else if bs.len() == ms.len() && bs.len() == ts.len() {
            // Lines changed in place, like two values of one entry: line by line.
            let mut clash = false;
            for k in 0..bs.len() {
                let (bl, ml, tl) = (bs[k], ms[k], ts[k]);
                if ml == bl {
                    both(&mut out, &[tl]);
                } else if tl == bl || ml == tl {
                    both(&mut out, &[ml]);
                } else {
                    clash = true;
                    out.mine.extend_from_slice(ml);
                    out.theirs.extend_from_slice(tl);
                }
            }
            out.conflicts += clash as usize;
        } else {
            out.conflicts += 1;
            for l in ms {
                out.mine.extend_from_slice(l);
            }
            for l in ts {
                out.theirs.extend_from_slice(l);
            }
        }
        if j >= b.len() {
            break;
        }
        (i, a, c) = (j, ae, ce);
    }
    out
}

/// Lines with their line ends, so `\r\n` stays as it was.
fn split(b: &[u8]) -> Vec<&[u8]> {
    b.split_inclusive(|c| *c == b'\n').collect()
}

/// Pairs (index in `a`, index in `b`) of equal lines, in order.
fn common(a: &[u32], b: &[u32]) -> Vec<(usize, usize)> {
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf] {
        suf += 1;
    }
    let mut pairs: Vec<(usize, usize)> = (0..pre).map(|i| (i, i)).collect();
    pairs.extend(myers(&a[pre..a.len() - suf], &b[pre..b.len() - suf]).into_iter().map(|(i, j)| (i + pre, j + pre)));
    pairs.extend((0..suf).map(|k| (a.len() - suf + k, b.len() - suf + k)));
    pairs
}

/// Myers' shortest edit script; the equal lines it keeps, in order.
fn myers(a: &[u32], b: &[u32]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let max = (n + m) as usize;
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let off = max as isize + 1;
    let mut v = vec![0isize; 2 * max + 3];
    // The furthest x on each diagonal before every step, kept only over the diagonals that
    // step can read: -(d+1)..=d+1.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut done = false;
    for d in 0..=max as isize {
        if d as usize > MAX_EDITS {
            return Vec::new();
        }
        trace.push(v[(off - d - 1) as usize..=(off + d + 1) as usize].to_vec());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[(off + k - 1) as usize] < v[(off + k + 1) as usize]) { v[(off + k + 1) as usize] } else { v[(off + k - 1) as usize] + 1 };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[(off + k) as usize] = x;
            if x >= n && y >= m {
                done = true;
                break;
            }
            k += 2;
        }
        if done {
            break;
        }
    }
    let mut pairs = Vec::new();
    let (mut x, mut y) = (n, m);
    for (d, saved) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let at = |k: isize| saved[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) { k + 1 } else { k - 1 };
        let prev_x = if d == 0 { 0 } else { at(prev_k) };
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            pairs.push(((x - 1) as usize, (y - 1) as usize));
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            x = prev_x;
            y = prev_y;
        }
    }
    pairs.reverse();
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<u8> {
        s.replace('|', "\n").into_bytes()
    }

    #[test]
    fn changes_in_different_places_are_both_kept() {
        let base = lines("<a>|<b>|<c>|<d>|<e>|");
        let mine = lines("<a>|<B mine>|<c>|<d>|<e>|");
        let theirs = lines("<a>|<b>|<c>|<d>|<new>|<e>|");
        let r = merge(&base, &mine, &theirs);
        assert_eq!(r.conflicts, 0);
        assert_eq!(r.mine, lines("<a>|<B mine>|<c>|<d>|<new>|<e>|"));
        assert_eq!(r.mine, r.theirs);
    }

    #[test]
    fn the_same_line_changed_twice_is_a_conflict() {
        let base = lines("x|y|z|");
        let r = merge(&base, &lines("x|Y1|z|"), &lines("x|Y2|z|"));
        assert_eq!(r.conflicts, 1);
        assert_eq!((r.mine, r.theirs), (lines("x|Y1|z|"), lines("x|Y2|z|")));
    }

    #[test]
    fn the_same_change_twice_is_no_conflict_and_line_ends_stay() {
        let base = b"a\r\nb\r\nc\r\n".to_vec();
        let both = b"a\r\nB\r\nc\r\n".to_vec();
        let r = merge(&base, &both, &both);
        assert_eq!((r.conflicts, r.mine), (0, both));
        let r = merge(&base, b"b\r\nc\r\n", b"a\r\nb\r\nc\r\nd\r\n");
        assert_eq!((r.conflicts, r.mine), (0, b"b\r\nc\r\nd\r\n".to_vec()));
    }

    #[test]
    fn edit_scripts_find_the_common_lines() {
        let a = [1, 2, 3, 4, 5, 6];
        let b = [1, 9, 3, 4, 7, 6, 8];
        assert_eq!(common(&a, &b), vec![(0, 0), (2, 2), (3, 3), (5, 5)]);
        let a: Vec<u32> = (0..2000).collect();
        let mut b = a.clone();
        b.remove(700);
        b.insert(1500, 99999);
        assert_eq!(common(&a, &b).len(), 1999);
    }
}
