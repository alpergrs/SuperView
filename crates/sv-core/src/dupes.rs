//! Finding duplicate pictures: files that are byte for byte the same, and pictures that
//! look the same although the files differ (resized, re-saved or converted copies).

use crate::decode::Bitmap;
use crate::folder::Entry;
use crate::thumbs;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Comparing every picture with every other gets slow beyond this many.
pub const MAX_SIMILAR: usize = 30_000;

fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// Largest file first, so the first of a group is the natural one to keep; equal sizes
/// keep the order they were listed in.
fn tidy(mut groups: Vec<Vec<Entry>>, order: &HashMap<PathBuf, usize>) -> Vec<Vec<PathBuf>> {
    for group in &mut groups {
        group.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| order[&a.path].cmp(&order[&b.path])));
    }
    groups.sort_by_key(|g| g.iter().map(|e| order[&e.path]).min());
    groups.into_iter().map(|g| g.into_iter().map(|e| e.path).collect()).collect()
}

/// Groups of files with exactly the same contents. `progress` counts files examined.
pub fn find_exact(entries: &[Entry], progress: &AtomicUsize, cancel: &AtomicBool) -> Vec<Vec<PathBuf>> {
    let order: HashMap<PathBuf, usize> = entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
    // Files of different sizes cannot be the same, so only same-size files are read.
    let mut by_size: HashMap<u64, Vec<&Entry>> = HashMap::new();
    for entry in entries {
        by_size.entry(entry.size).or_default().push(entry);
    }
    let (alone, candidates): (Vec<_>, Vec<_>) = by_size.into_values().partition(|g| g.len() < 2);
    progress.fetch_add(alone.iter().map(Vec::len).sum(), Ordering::Relaxed);

    let hashed: Vec<(&Entry, Option<(u64, u64)>)> = candidates
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|entry| {
            let hash = if cancel.load(Ordering::Relaxed) {
                None
            } else {
                std::fs::read(&entry.path).ok().map(|data| (fnv(0xCBF2_9CE4_8422_2325, &data), fnv(0x8422_2325_CBF2_9CE4, &data)))
            };
            progress.fetch_add(1, Ordering::Relaxed);
            (entry, hash)
        })
        .collect();

    let mut by_hash: HashMap<(u64, u64, u64), Vec<&Entry>> = HashMap::new();
    for (entry, hash) in hashed {
        if let Some((a, b)) = hash {
            by_hash.entry((entry.size, a, b)).or_default().push(entry);
        }
    }
    let mut groups = Vec::new();
    for candidates in by_hash.into_values().filter(|g| g.len() > 1) {
        // Matching fingerprints are confirmed by comparing the actual bytes.
        let Ok(first) = std::fs::read(&candidates[0].path) else { continue };
        let same: Vec<Entry> = candidates
            .iter()
            .filter(|e| e.path == candidates[0].path || std::fs::read(&e.path).is_ok_and(|data| data == first))
            .map(|e| (*e).clone())
            .collect();
        if same.len() > 1 {
            groups.push(same);
        }
    }
    tidy(groups, &order)
}

/// A 64-bit fingerprint of how a picture looks: whether each pixel of a 9x8 greyscale
/// miniature is brighter than its right neighbour. Resizing, re-saving and mild colour
/// changes leave almost all of the bits unchanged.
pub fn look(bitmap: &Bitmap) -> u64 {
    let (w, h) = (bitmap.width.max(1) as usize, bitmap.height.max(1) as usize);
    let mut grey = [[0u32; 9]; 8];
    for (row, line) in grey.iter_mut().enumerate() {
        for (col, cell) in line.iter_mut().enumerate() {
            // Average over the block of the picture this cell covers.
            let (x0, x1) = (col * w / 9, ((col + 1) * w / 9).max(col * w / 9 + 1).min(w));
            let (y0, y1) = (row * h / 8, ((row + 1) * h / 8).max(row * h / 8 + 1).min(h));
            let (mut sum, mut count) = (0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    sum += bitmap.rgba[i] as u64 * 77 + bitmap.rgba[i + 1] as u64 * 150 + bitmap.rgba[i + 2] as u64 * 29;
                    count += 1;
                }
            }
            *cell = (sum / count.max(1)) as u32;
        }
    }
    let mut bits = 0u64;
    for line in &grey {
        for col in 0..8 {
            bits = (bits << 1) | (line[col] > line[col + 1]) as u64;
        }
    }
    bits
}

/// The average colour of a picture. The fingerprint only sees light and dark, so two
/// pictures with the same layout in different colours need this to be told apart.
pub fn average_colour(bitmap: &Bitmap) -> [u8; 3] {
    let mut sum = [0u64; 3];
    for p in bitmap.rgba.chunks_exact(4) {
        for c in 0..3 {
            sum[c] += p[c] as u64;
        }
    }
    let count = (bitmap.rgba.len() / 4).max(1) as u64;
    [(sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8]
}

/// Groups of pictures that look alike. `tolerance` is how many of the 64 fingerprint
/// bits may differ: 0 for near-identical only, around 6 for resized and re-saved copies,
/// more finds looser matches and more false ones. Pictures must also have nearly the
/// same proportions. Returns `None` when there are too many pictures to compare.
pub fn find_similar(entries: &[Entry], tolerance: u32, progress: &AtomicUsize, cancel: &AtomicBool) -> Option<Vec<Vec<PathBuf>>> {
    if entries.len() > MAX_SIMILAR {
        return None;
    }
    let order: HashMap<PathBuf, usize> = entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
    let prints: Vec<(&Entry, u64, f32, [u8; 3])> = entries
        .par_iter()
        .filter_map(|entry| {
            let print = if cancel.load(Ordering::Relaxed) {
                None
            } else {
                std::panic::catch_unwind(|| thumbs::make(&entry.path)).ok().and_then(Result::ok).map(|b| (entry, look(&b), b.width as f32 / b.height.max(1) as f32, average_colour(&b)))
            };
            progress.fetch_add(1, Ordering::Relaxed);
            print
        })
        .collect();

    // Union-find: pictures linked directly or through others end up in one group.
    let mut parent: Vec<usize> = (0..prints.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..prints.len() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        for j in i + 1..prints.len() {
            let alike = (prints[i].1 ^ prints[j].1).count_ones() <= tolerance;
            let same_shape = (prints[i].2 / prints[j].2 - 1.0).abs() < 0.05;
            let colour_gap = (0..3).map(|c| (prints[i].3[c] as i32 - prints[j].3[c] as i32).abs()).max().unwrap_or(0);
            let same_colours = colour_gap <= 10 + tolerance as i32;
            if alike && same_shape && same_colours {
                let (a, b) = (root(&mut parent, i), root(&mut parent, j));
                parent[a] = b;
            }
        }
    }
    let mut groups: HashMap<usize, Vec<Entry>> = HashMap::new();
    for (i, print) in prints.iter().enumerate() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(print.0.clone());
    }
    Some(tidy(groups.into_values().filter(|g| g.len() > 1).collect(), &order))
}
