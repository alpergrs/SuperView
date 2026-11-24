//! Thumbnails: making them quickly and remembering them on disk.

use crate::decode::{self, jpeg, Bitmap, DecodeError, Format};
use image::metadata::Orientation;
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// Longest side of a stored thumbnail, in pixels.
pub const THUMB_SIDE: u32 = 256;

fn shrink(bitmap: Bitmap) -> Bitmap {
    if bitmap.width.max(bitmap.height) <= THUMB_SIDE {
        return bitmap;
    }
    let scale = THUMB_SIDE as f64 / bitmap.width.max(bitmap.height) as f64;
    let w = ((bitmap.width as f64 * scale).round() as u32).max(1);
    let h = ((bitmap.height as f64 * scale).round() as u32).max(1);
    let (bw, bh) = (bitmap.width, bitmap.height);
    let Some(src) = image::RgbaImage::from_raw(bw, bh, bitmap.rgba) else {
        return Bitmap { width: 1, height: 1, rgba: vec![0, 0, 0, 255] };
    };
    let out = image::imageops::thumbnail(&src, w, h);
    Bitmap { width: w, height: h, rgba: out.into_raw() }
}

/// JPEGs can be decoded straight to 1/2, 1/4 or 1/8 size, which is many times faster
/// than decoding everything and shrinking afterwards.
fn jpeg_thumb(data: &[u8]) -> Option<Bitmap> {
    let mut decompressor = turbojpeg::Decompressor::new().ok()?;
    let header = decompressor.read_header(data).ok()?;
    let longest = header.width.max(header.height);
    let mut factor = turbojpeg::ScalingFactor::ONE;
    for f in [
        turbojpeg::ScalingFactor::ONE_EIGHTH,
        turbojpeg::ScalingFactor::ONE_QUARTER,
        turbojpeg::ScalingFactor::ONE_HALF,
    ] {
        if f.scale(longest) >= THUMB_SIDE as usize {
            factor = f;
            break;
        }
    }
    decompressor.set_scaling_factor(factor).ok()?;
    let (w, h) = (factor.scale(header.width), factor.scale(header.height));
    let mut pixels = vec![0u8; w * h * 4];
    let image = turbojpeg::Image {
        pixels: pixels.as_mut_slice(),
        width: w,
        pitch: w * 4,
        height: h,
        format: turbojpeg::PixelFormat::RGBA,
    };
    decompressor.decompress(data, image).ok()?;
    let mut bitmap = Bitmap { width: w as u32, height: h as u32, rgba: pixels };
    let (turns, flip_h) = match jpeg::exif_orientation(data) {
        Orientation::NoTransforms => (0, false),
        Orientation::FlipHorizontal => (0, true),
        Orientation::Rotate180 => (2, false),
        Orientation::FlipVertical => (2, true),
        Orientation::Rotate90FlipH => (1, true),
        Orientation::Rotate90 => (1, false),
        Orientation::Rotate270FlipH => (3, true),
        Orientation::Rotate270 => (3, false),
    };
    if turns != 0 || flip_h {
        bitmap = bitmap.turned(turns, flip_h, false);
    }
    Some(bitmap)
}

/// Make a thumbnail from the file itself.
pub fn make(path: &Path) -> Result<Bitmap, DecodeError> {
    let data = std::fs::read(path)?;
    let format = Format::detect(&data, path).ok_or(DecodeError::Unsupported)?;
    if format == Format::Jpeg {
        if let Some(bitmap) = jpeg_thumb(&data) {
            return Ok(shrink(bitmap));
        }
    }
    if format == Format::Heif {
        return decode::guarded(|| decode::heif::thumbnail(&data)).map(shrink);
    }
    if format == Format::Raw {
        // Cameras store a small ready-made picture next to the sensor data.
        return decode::guarded(|| decode::raw::thumbnail(&data)).map(shrink);
    }
    let mut loaded = decode::load_bytes(&data, format, 0)?;
    Ok(shrink(loaded.frames.swap_remove(0).bitmap))
}

fn encode_jpeg(bitmap: &Bitmap) -> Option<Vec<u8>> {
    // JPEG has no transparency: put transparent pictures on white.
    let mut flat = bitmap.rgba.clone();
    for p in flat.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 255 {
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
            }
            p[3] = 255;
        }
    }
    let image = turbojpeg::Image {
        pixels: flat.as_slice(),
        width: bitmap.width as usize,
        pitch: bitmap.width as usize * 4,
        height: bitmap.height as usize,
        format: turbojpeg::PixelFormat::RGBA,
    };
    turbojpeg::compress(image, 85, turbojpeg::Subsamp::Sub2x2).ok().map(|d| d.to_vec())
}

fn decode_jpeg(data: &[u8]) -> Option<Bitmap> {
    let img = turbojpeg::decompress(data, turbojpeg::PixelFormat::RGBA).ok()?;
    if img.pitch != img.width * 4 {
        return None;
    }
    Some(Bitmap { width: img.width as u32, height: img.height as u32, rgba: img.pixels })
}

fn fnv(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn stamp(time: SystemTime) -> u64 {
    time.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

const MAGIC: &[u8; 4] = b"SVC1";

struct Record {
    size: u64,
    stamp: u64,
    offset: u64,
    len: u32,
}

/// The thumbnails of one folder, kept in a single file:
///
/// ```text
/// "SVC1"  u32 path length  folder path (UTF-8)
/// then any number of records, newest wins:
///   u16 name length  file name  u64 file size  u64 modified (ns)  u32 JPEG length  JPEG
/// ```
///
/// A JPEG length of zero means "forget this picture". Records are only ever appended,
/// so a crash can at worst leave a damaged tail, which is cut off on the next load.
struct FolderCache {
    file: PathBuf,
    index: HashMap<String, Record>,
    /// Bytes taken by records that a newer record replaced.
    stale: u64,
}

fn read_exact_or_none<R: Read, const N: usize>(r: &mut R) -> Option<[u8; N]> {
    let mut buf = [0u8; N];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// The folder a cache file belongs to, from its header.
fn header_folder(file: &Path) -> Option<PathBuf> {
    let mut r = BufReader::new(std::fs::File::open(file).ok()?);
    if &read_exact_or_none::<_, 4>(&mut r)? != MAGIC {
        return None;
    }
    let len = u32::from_le_bytes(read_exact_or_none(&mut r)?) as usize;
    if len > 65_536 {
        return None;
    }
    let mut path = vec![0u8; len];
    r.read_exact(&mut path).ok()?;
    Some(PathBuf::from(String::from_utf8(path).ok()?))
}

impl FolderCache {
    fn open(file: PathBuf, folder: &Path) -> FolderCache {
        let mut cache = FolderCache { file, index: HashMap::new(), stale: 0 };
        if cache.load().is_none() {
            // Missing or unreadable: start a fresh file.
            cache.index.clear();
            cache.stale = 0;
            let _ = cache.write_header(folder);
        }
        cache
    }

    fn write_header(&self, folder: &Path) -> std::io::Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let path = folder.to_string_lossy();
        let mut out = Vec::with_capacity(8 + path.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(path.len() as u32).to_le_bytes());
        out.extend_from_slice(path.as_bytes());
        std::fs::write(&self.file, out)
    }

    fn load(&mut self) -> Option<()> {
        let handle = std::fs::File::open(&self.file).ok()?;
        let total = handle.metadata().ok()?.len();
        let mut r = BufReader::new(handle);
        if &read_exact_or_none::<_, 4>(&mut r)? != MAGIC {
            return None;
        }
        let path_len = u32::from_le_bytes(read_exact_or_none(&mut r)?) as u64;
        r.seek_relative(path_len as i64).ok()?;
        let mut pos = 8 + path_len;
        let mut good = pos;
        while pos < total {
            let record = (|| -> Option<(String, Record)> {
                let name_len = u16::from_le_bytes(read_exact_or_none(&mut r)?) as usize;
                let mut name = vec![0u8; name_len];
                r.read_exact(&mut name).ok()?;
                let size = u64::from_le_bytes(read_exact_or_none(&mut r)?);
                let stamp = u64::from_le_bytes(read_exact_or_none(&mut r)?);
                let len = u32::from_le_bytes(read_exact_or_none(&mut r)?);
                let offset = pos + 2 + name_len as u64 + 20;
                if offset + len as u64 > total {
                    return None;
                }
                r.seek_relative(len as i64).ok()?;
                Some((String::from_utf8(name).ok()?, Record { size, stamp, offset, len }))
            })();
            let Some((name, record)) = record else { break };
            pos = record.offset + record.len as u64;
            good = pos;
            let replaced = if record.len == 0 { self.index.remove(&name) } else { self.index.insert(name, record) };
            if let Some(old) = replaced {
                self.stale += old.len as u64;
            }
        }
        if good < total {
            // Cut off a half-written record left by a crash.
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&self.file) {
                let _ = f.set_len(good);
            }
        }
        Some(())
    }

    fn get(&self, name: &str, size: u64, stamp: u64) -> Option<Bitmap> {
        let record = self.index.get(name).filter(|r| r.size == size && r.stamp == stamp)?;
        let mut f = std::fs::File::open(&self.file).ok()?;
        f.seek(SeekFrom::Start(record.offset)).ok()?;
        let mut data = vec![0u8; record.len as usize];
        f.read_exact(&mut data).ok()?;
        decode_jpeg(&data)
    }

    fn append(&mut self, name: &str, size: u64, stamp: u64, jpeg: &[u8]) -> Option<()> {
        if name.len() > u16::MAX as usize {
            return None;
        }
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.file).ok()?;
        let start = f.metadata().ok()?.len();
        let mut out = Vec::with_capacity(22 + name.len() + jpeg.len());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&stamp.to_le_bytes());
        out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        out.extend_from_slice(jpeg);
        f.write_all(&out).ok()?;
        let record = Record { size, stamp, offset: start + 22 + name.len() as u64, len: jpeg.len() as u32 };
        let replaced = if jpeg.is_empty() { self.index.remove(name) } else { self.index.insert(name.to_string(), record) };
        if let Some(old) = replaced {
            self.stale += old.len as u64;
        }
        Some(())
    }
}

/// All folder caches, kept in one place (`%LOCALAPPDATA%\SuperView\cache`). Nothing is
/// ever written into the picture folders themselves.
pub struct CacheStore {
    root: PathBuf,
    folders: Mutex<HashMap<PathBuf, Arc<Mutex<FolderCache>>>>,
}

fn file_name_key(path: &Path) -> Option<String> {
    Some(path.file_name()?.to_string_lossy().to_lowercase())
}

impl CacheStore {
    pub fn new(root: PathBuf) -> CacheStore {
        CacheStore { root, folders: Mutex::new(HashMap::new()) }
    }

    pub fn default_location() -> Option<CacheStore> {
        Some(CacheStore::new(app_cache_dir()?.join("cache")))
    }

    fn file_for(&self, folder: &Path) -> PathBuf {
        let text = folder.to_string_lossy().to_lowercase();
        let a = fnv(0xCBF2_9CE4_8422_2325, text.as_bytes());
        let b = fnv(0x8422_2325_CBF2_9CE4, text.as_bytes());
        self.root.join(format!("{a:016x}{b:016x}.svc"))
    }

    fn folder(&self, folder: &Path) -> Arc<Mutex<FolderCache>> {
        let mut folders = self.folders.lock().unwrap();
        if folders.len() > 512 && !folders.contains_key(folder) {
            // Only the indexes are dropped; they are read again when needed.
            folders.clear();
        }
        folders
            .entry(folder.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(FolderCache::open(self.file_for(folder), folder))))
            .clone()
    }

    pub fn get(&self, path: &Path, size: u64, modified: SystemTime) -> Option<Bitmap> {
        let cache = self.folder(path.parent()?);
        let cache = cache.lock().unwrap();
        cache.get(&file_name_key(path)?, size, stamp(modified))
    }

    pub fn put(&self, path: &Path, size: u64, modified: SystemTime, bitmap: &Bitmap) {
        let (Some(folder), Some(name), Some(jpeg)) = (path.parent(), file_name_key(path), encode_jpeg(bitmap)) else {
            return;
        };
        let cache = self.folder(folder);
        let _ = cache.lock().unwrap().append(&name, size, stamp(modified), &jpeg);
    }

    /// Drop the thumbnail of a picture whose contents changed. Does nothing for a
    /// folder that has no cache file.
    pub fn forget(&self, path: &Path) {
        let (Some(folder), Some(name)) = (path.parent(), file_name_key(path)) else { return };
        if !self.file_for(folder).exists() {
            return;
        }
        let cache = self.folder(folder);
        let _ = cache.lock().unwrap().append(&name, 0, 0, &[]);
    }

    fn files(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(read) = std::fs::read_dir(&self.root) else { return Vec::new() };
        read.filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "svc"))
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                Some((e.path(), meta.len(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
            })
            .collect()
    }

    /// Bytes on disk taken by all folder caches.
    pub fn total_size(&self) -> u64 {
        self.files().iter().map(|f| f.1).sum()
    }

    fn delete(&self, file: &Path) {
        let _ = std::fs::remove_file(file);
        self.folders.lock().unwrap().retain(|_, c| c.lock().unwrap().file != file);
    }

    /// Delete every cache file.
    pub fn clear(&self) {
        for (file, _, _) in self.files() {
            let _ = std::fs::remove_file(file);
        }
        self.folders.lock().unwrap().clear();
    }

    /// Delete the caches of folders that no longer exist or whose pictures now add up
    /// to less than `threshold` bytes. Returns how many files were removed.
    pub fn remove_small(&self, threshold: u64) -> usize {
        let mut removed = 0;
        for (file, _, _) in self.files() {
            // A folder that cannot be reached right now (network drive away) is left alone:
            // there is no telling how large it is.
            let keep = header_folder(&file).is_some_and(|folder| {
                if folder.is_dir() {
                    picture_bytes(&folder) >= threshold
                } else {
                    !folder_is_gone(&folder)
                }
            });
            if !keep {
                self.delete(&file);
                removed += 1;
            }
        }
        removed
    }

    /// Housekeeping for start-up: drop caches of folders that are gone, then the least
    /// recently written ones while the total is above `cap` bytes.
    pub fn tidy(&self, cap: u64) {
        let mut files = self.files();
        files.retain(|(file, _, _)| {
            let alive = header_folder(file).is_some_and(|folder| !folder_is_gone(&folder));
            if !alive {
                self.delete(file);
            }
            alive
        });
        files.sort_by_key(|f| f.2);
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        for (file, len, _) in files {
            if total <= cap {
                break;
            }
            self.delete(&file);
            total -= len;
        }
    }
}

/// Whether a folder has really been removed. A folder on a drive or network share that
/// cannot be reached at the moment is not gone, only out of sight.
fn folder_is_gone(folder: &Path) -> bool {
    if folder.is_dir() {
        return false;
    }
    // The top of the path: a drive (`D:\`) or a share (`\\server\share\`).
    let top = folder.ancestors().last().unwrap_or(folder);
    top.is_dir()
}

/// Total size of the pictures directly inside `folder`.
pub fn picture_bytes(folder: &Path) -> u64 {
    crate::folder::list(folder, false).iter().map(|e| e.size).sum()
}

fn app_cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("SuperView"))
}

/// Remove the thumbnail files written by SuperView versions before the per-folder cache.
pub fn remove_legacy_cache() {
    if let Some(dir) = app_cache_dir() {
        let _ = std::fs::remove_dir_all(dir.join("thumbs"));
    }
}

/// The thumbnail for a picture. With a store it comes from the folder's cache file when
/// present and is added to it otherwise; without one nothing touches the disk.
pub fn load_or_make(
    store: Option<&CacheStore>,
    path: &Path,
    size: u64,
    modified: SystemTime,
) -> Result<Bitmap, DecodeError> {
    if let Some(bitmap) = store.and_then(|c| c.get(path, size, modified)) {
        return Ok(bitmap);
    }
    let bitmap = make(path)?;
    if let Some(store) = store {
        store.put(path, size, modified, &bitmap);
    }
    Ok(bitmap)
}
