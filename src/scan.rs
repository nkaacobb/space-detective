//! Walks a folder in parallel and builds a flat tree of every file and folder with its size.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

use eframe::egui::Color32;
use eframe::egui::ecolor::Hsva;
use rayon::prelude::*;

/// Live counters the GUI reads while a scan runs.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
}

pub const NO_PARENT: u32 = u32::MAX;

/// Folders never entered while scanning, unless scanned directly. On macOS the data volume is
/// already visible through "/" (Users, Applications, …) and /Volumes holds other disks; on Linux
/// these are virtual filesystems with made-up sizes.
#[cfg(target_os = "macos")]
const SKIP_DIRS: &[&str] = &["/System/Volumes/Data", "/Volumes", "/dev"];
#[cfg(all(unix, not(target_os = "macos")))]
const SKIP_DIRS: &[&str] = &["/proc", "/sys", "/dev", "/run"];
#[cfg(not(unix))]
const SKIP_DIRS: &[&str] = &[];

pub struct Node {
    pub name: Box<str>,
    pub size: u64,
    pub parent: u32,
    /// Children are stored next to each other, largest first.
    pub first_child: u32,
    pub child_count: u32,
    /// Files and folders inside this one (whole subtree). A file counts itself once.
    pub files: u64,
    pub dirs: u64,
    pub is_dir: bool,
    pub ext: u32,
}

pub struct ExtStat {
    pub label: String,
    pub bytes: u64,
    pub count: u64,
    pub color: Color32,
}

pub struct Tree {
    pub root_path: PathBuf,
    pub nodes: Vec<Node>,
    pub exts: Vec<ExtStat>,
    /// Extension ids, largest total size first.
    pub ext_rank: Vec<u32>,
    pub errors: u64,
    pub elapsed: Duration,
}

impl Tree {
    pub fn children(&self, idx: u32) -> std::ops::Range<u32> {
        let n = &self.nodes[idx as usize];
        n.first_child..n.first_child + n.child_count
    }

    pub fn node(&self, idx: u32) -> &Node {
        &self.nodes[idx as usize]
    }

    pub fn display_name(&self, idx: u32) -> String {
        if idx == 0 {
            self.root_path.display().to_string()
        } else {
            self.node(idx).name.to_string()
        }
    }

    pub fn path_of(&self, idx: u32) -> PathBuf {
        let mut parts = Vec::new();
        let mut i = idx;
        while i != 0 && i != NO_PARENT {
            parts.push(&*self.node(i).name);
            i = self.node(i).parent;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    /// From the scan root down to `idx`, inclusive.
    pub fn ancestry(&self, idx: u32) -> Vec<u32> {
        let mut chain = vec![idx];
        let mut i = idx;
        while self.node(i).parent != NO_PARENT {
            i = self.node(i).parent;
            chain.push(i);
        }
        chain.reverse();
        chain
    }

    pub fn color_of(&self, idx: u32) -> Color32 {
        self.exts[self.node(idx).ext as usize].color
    }

    /// The `count` biggest files anywhere under `root`, biggest first.
    pub fn largest_files(&self, root: u32, count: usize) -> Vec<u32> {
        let mut heap: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::with_capacity(count + 1);
        let mut stack = vec![root];
        while let Some(i) = stack.pop() {
            let n = self.node(i);
            if n.is_dir {
                stack.extend(self.children(i));
            } else if heap.len() < count {
                heap.push(Reverse((n.size, i)));
            } else if heap.peek().is_some_and(|Reverse((s, _))| n.size > *s) {
                heap.pop();
                heap.push(Reverse((n.size, i)));
            }
        }
        let mut out: Vec<(u64, u32)> = heap.into_iter().map(|Reverse(x)| x).collect();
        out.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().map(|(_, i)| i).collect()
    }
}

/// Scans `root` on a worker pool. Returns a partial tree if `progress.cancel` is set.
pub fn scan(root: &Path, progress: &Progress) -> Tree {
    let started = std::time::Instant::now();
    let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    let pool = rayon::ThreadPoolBuilder::new()
        // Folder reads mostly wait on the disk, so more threads than cores keeps it busy.
        .num_threads((std::thread::available_parallelism().map_or(4, |n| n.get()) * 2).min(32))
        .stack_size(16 << 20)
        .build()
        .expect("thread pool");
    let seen = Seen::new();
    let mut raw = pool.install(|| scan_dir(&root, root.display().to_string(), 0, progress, &seen));
    dedupe(&mut raw);
    let mut tree = flatten(raw, root);
    tree.errors = progress.errors.load(Relaxed);
    tree.elapsed = started.elapsed();
    tree
}

struct RawNode {
    name: String,
    size: u64,
    is_dir: bool,
    files: u64,
    dirs: u64,
    children: Vec<RawNode>,
    /// On-disk identity (0 if unknown): the same folder or file reached by two paths has the same id.
    id: u64,
    depth: u16,
    /// A folder already scanned through a shorter path, left empty.
    skipped: bool,
}

/// One entry of a folder listing. Symlinks, junctions and mount points are left out: they lead to
/// data that's counted where it really lives (or loop back on themselves).
struct Entry {
    name: OsString,
    is_dir: bool,
    size: u64,
    /// Files only; folders get theirs when they're listed.
    id: u64,
}

/// Folders visited so far, by on-disk identity, with the shallowest depth each was reached at.
/// Antivirus sandboxes (Norton's, for one) show the whole disk again inside themselves; this stops
/// them being counted twice, or sixty times.
struct Seen {
    shards: Vec<Mutex<HashMap<u64, u16>>>,
}

impl Seen {
    fn new() -> Self {
        Self { shards: (0..64).map(|_| Mutex::new(HashMap::new())).collect() }
    }

    /// Whether to scan a folder reached at `depth`: true unless it was reached by a path at least as short.
    fn claim(&self, id: u64, depth: u16) -> bool {
        if id == 0 {
            return true;
        }
        let mut shard = self.shards[(id ^ (id >> 29)) as usize % self.shards.len()].lock().unwrap();
        match shard.get(&id) {
            Some(&best) if best <= depth => false,
            _ => {
                shard.insert(id, depth);
                true
            }
        }
    }
}

/// Mixes a volume serial and a file index into one identity.
fn identity(volume: u64, index: u64) -> u64 {
    index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ volume
}

fn scan_dir(path: &Path, name: String, depth: u16, progress: &Progress, seen: &Seen) -> RawNode {
    let mut node = RawNode {
        name,
        size: 0,
        is_dir: true,
        files: 0,
        dirs: 0,
        children: Vec::new(),
        id: 0,
        depth,
        skipped: false,
    };
    if progress.cancel.load(Relaxed) {
        return node;
    }
    let Ok((id, entries)) = list::read(path) else {
        progress.errors.fetch_add(1, Relaxed);
        return node;
    };
    node.id = id;
    if !seen.claim(id, depth) {
        node.skipped = true;
        return node;
    }
    progress.dirs.fetch_add(1, Relaxed);
    if let Ok(mut current) = progress.current.try_lock() {
        current.clear();
        current.push_str(&path.to_string_lossy());
    }

    let mut subdirs = Vec::new();
    for entry in entries {
        let display = entry.name.to_string_lossy().into_owned();
        if entry.is_dir {
            let child = path.join(&entry.name);
            if !SKIP_DIRS.iter().any(|skip| child == Path::new(skip)) {
                subdirs.push((child, display));
            }
        } else {
            progress.files.fetch_add(1, Relaxed);
            progress.bytes.fetch_add(entry.size, Relaxed);
            node.children.push(RawNode {
                name: display,
                size: entry.size,
                is_dir: false,
                files: 1,
                dirs: 0,
                children: Vec::new(),
                id: entry.id,
                depth: depth + 1,
                skipped: false,
            });
        }
    }

    let sub: Vec<RawNode> = subdirs.into_par_iter().map(|(p, n)| scan_dir(&p, n, depth + 1, progress, seen)).collect();
    node.children.extend(sub);
    total(&mut node);
    node
}

fn total(node: &mut RawNode) {
    node.size = 0;
    node.files = 0;
    node.dirs = 0;
    for child in &node.children {
        node.size += child.size;
        node.files += if child.is_dir { child.files } else { 1 };
        node.dirs += if child.is_dir { child.dirs + 1 } else { 0 };
    }
    node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
}

/// Files smaller than this aren't de-duplicated; it keeps the bookkeeping small and they barely move the totals.
const DEDUPE_MIN: u64 = 1 << 20;

/// Keeps each folder and big file once, at the shortest path it was found at. A sandbox scanned
/// before the real folder, and files hard-linked into several folders, are dropped here.
fn dedupe(root: &mut RawNode) {
    let mut best: HashMap<(bool, u64), (u16, Vec<u32>)> = HashMap::new();
    collect(root, &mut Vec::new(), &mut best);
    prune(root, &mut Vec::new(), &best);
}

fn dedupe_key(node: &RawNode) -> Option<(bool, u64)> {
    let counts = node.is_dir || node.size >= DEDUPE_MIN;
    (counts && node.id != 0 && !node.skipped).then_some((node.is_dir, node.id))
}

fn collect(node: &RawNode, address: &mut Vec<u32>, best: &mut HashMap<(bool, u64), (u16, Vec<u32>)>) {
    if let Some(key) = dedupe_key(node)
        && best.get(&key).is_none_or(|(depth, _)| node.depth < *depth)
    {
        best.insert(key, (node.depth, address.clone()));
    }
    for (i, child) in node.children.iter().enumerate() {
        address.push(i as u32);
        collect(child, address, best);
        address.pop();
    }
}

/// Returns false when `node` is a duplicate that should go.
fn prune(node: &mut RawNode, address: &mut Vec<u32>, best: &HashMap<(bool, u64), (u16, Vec<u32>)>) -> bool {
    if node.skipped {
        return false;
    }
    if let Some(key) = dedupe_key(node)
        && best.get(&key).is_some_and(|(_, winner)| winner != address)
    {
        return false;
    }
    if node.is_dir {
        let children = std::mem::take(&mut node.children);
        for (i, mut child) in children.into_iter().enumerate() {
            address.push(i as u32);
            if prune(&mut child, address, best) {
                node.children.push(child);
            }
            address.pop();
        }
        total(node);
    }
    true
}

#[cfg(windows)]
mod list {
    //! Reads a folder with one call per 64 KiB of entries, getting each file's id and size along
    //! the way (the standard library's listing doesn't report ids).

    use std::ffi::{OsString, c_void};
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, OPEN_EXISTING,
    };

    use super::{Entry, identity};

    /// Reparse tags with this bit set are links to somewhere else (symlinks, junctions, mount points).
    const NAME_SURROGATE: u32 = 0x2000_0000;

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    /// `\\?\` form, so paths longer than 260 characters still open.
    fn wide(path: &Path) -> Vec<u16> {
        let text = path.as_os_str().to_string_lossy();
        let mut out: Vec<u16> = if text.starts_with(r"\\?\") {
            Vec::new()
        } else if text.starts_with(r"\\") {
            r"\\?\UNC".encode_utf16().collect()
        } else {
            r"\\?\".encode_utf16().collect()
        };
        let mut chars = path.as_os_str().encode_wide().map(|c| if c == '/' as u16 { '\\' as u16 } else { c }).peekable();
        if text.starts_with(r"\\") && !text.starts_with(r"\\?\") {
            chars.next(); // "\\server" becomes "\\?\UNC\server"
        }
        out.extend(chars);
        out.push(0);
        out
    }

    pub fn read(path: &Path) -> io::Result<(u64, Vec<Entry>)> {
        let name = wide(path);
        let raw = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                std::ptr::null_mut(),
            )
        };
        if raw == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let handle = Handle(raw);

        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        let (volume, id) = if unsafe { GetFileInformationByHandle(handle.0, &mut info) } != 0 {
            let volume = info.dwVolumeSerialNumber as u64;
            (volume, identity(volume, ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64))
        } else {
            (0, 0)
        };

        let mut buffer = vec![0u64; 8 * 1024];
        let mut entries = Vec::new();
        let mut class = FileIdBothDirectoryRestartInfo;
        loop {
            let ok = unsafe {
                GetFileInformationByHandleEx(handle.0, class, buffer.as_mut_ptr().cast::<c_void>(), (buffer.len() * 8) as u32)
            };
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if error == ERROR_NO_MORE_FILES {
                    break;
                }
                return Err(io::Error::from_raw_os_error(error as i32));
            }
            class = FileIdBothDirectoryInfo;
            let base = buffer.as_ptr().cast::<u8>();
            let mut offset = 0usize;
            loop {
                // Records are 8-byte aligned within the u64 buffer.
                let record = unsafe { &*base.add(offset).cast::<FILE_ID_BOTH_DIR_INFO>() };
                let len = record.FileNameLength as usize / 2;
                let name = unsafe { std::slice::from_raw_parts(std::ptr::addr_of!(record.FileName).cast::<u16>(), len) };
                let dots = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
                let attributes = record.FileAttributes;
                // For reparse points, EaSize holds the reparse tag.
                let link = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 && record.EaSize & NAME_SURROGATE != 0;
                if !dots && !link {
                    let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                    entries.push(Entry {
                        name: OsString::from_wide(name),
                        is_dir,
                        size: if is_dir { 0 } else { record.EndOfFile.max(0) as u64 },
                        id: if is_dir { 0 } else { identity(volume, record.FileId as u64) },
                    });
                }
                if record.NextEntryOffset == 0 {
                    break;
                }
                offset += record.NextEntryOffset as usize;
            }
        }
        Ok((id, entries))
    }
}

#[cfg(not(windows))]
mod list {
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    use super::{Entry, identity};

    pub fn read(path: &Path) -> io::Result<(u64, Vec<Entry>)> {
        let meta = std::fs::symlink_metadata(path)?;
        let id = identity(meta.dev(), meta.ino());
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let Ok(entry) = entry else { continue };
            let Ok(meta) = entry.metadata() else { continue };
            let file_type = meta.file_type();
            if file_type.is_symlink() {
                continue;
            }
            let is_dir = file_type.is_dir();
            entries.push(Entry {
                name: entry.file_name(),
                is_dir,
                size: if is_dir { 0 } else { meta.len() },
                // Only hard-linked files can turn up twice.
                id: if !is_dir && meta.nlink() > 1 { identity(meta.dev(), meta.ino()) } else { 0 },
            });
        }
        Ok((id, entries))
    }
}

fn extension_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() && name.len() - i <= 12 && !name[i..].contains(' ') => {
            name[i..].to_lowercase()
        }
        _ => String::new(),
    }
}

/// Turns the nested scan result into one flat array with each folder's children side by side.
fn flatten(root: RawNode, root_path: PathBuf) -> Tree {
    let mut nodes = Vec::new();
    let mut ext_ids: HashMap<String, u32> = HashMap::new();
    let mut exts: Vec<ExtStat> = Vec::new();
    let mut ext_id = |name: &str, is_dir: bool, size: u64| -> u32 {
        let key = if is_dir { "/".to_string() } else { extension_of(name) };
        let id = *ext_ids.entry(key.clone()).or_insert_with(|| {
            let label = match key.as_str() {
                "/" => "Folders".to_string(),
                "" => "(no extension)".to_string(),
                _ => key.clone(),
            };
            exts.push(ExtStat { label, bytes: 0, count: 0, color: Color32::GRAY });
            exts.len() as u32 - 1
        });
        if !is_dir {
            exts[id as usize].bytes += size;
            exts[id as usize].count += 1;
        }
        id
    };

    let make = |raw: &RawNode, parent: u32, ext: u32| Node {
        name: raw.name.clone().into_boxed_str(),
        size: raw.size,
        parent,
        first_child: 0,
        child_count: 0,
        files: raw.files,
        dirs: raw.dirs,
        is_dir: raw.is_dir,
        ext,
    };

    let root_ext = ext_id("", true, 0);
    nodes.push(make(&root, NO_PARENT, root_ext));
    let mut queue = VecDeque::from([(root, 0u32)]);
    while let Some((mut raw, idx)) = queue.pop_front() {
        let children = std::mem::take(&mut raw.children);
        nodes[idx as usize].first_child = nodes.len() as u32;
        nodes[idx as usize].child_count = children.len() as u32;
        for child in children {
            let ext = ext_id(&child.name, child.is_dir, child.size);
            let child_idx = nodes.len() as u32;
            nodes.push(make(&child, idx, ext));
            if child.is_dir {
                queue.push_back((child, child_idx));
            }
        }
    }

    let folder_ext = ext_ids["/"];
    let mut ext_rank: Vec<u32> = (0..exts.len() as u32).filter(|&i| i != folder_ext).collect();
    ext_rank.sort_unstable_by(|&a, &b| exts[b as usize].bytes.cmp(&exts[a as usize].bytes));
    for (rank, &id) in ext_rank.iter().enumerate() {
        exts[id as usize].color = color_for_rank(rank, &exts[id as usize].label);
    }
    exts[folder_ext as usize].color = Color32::from_rgb(232, 182, 84);

    Tree { root_path, nodes, exts, ext_rank, errors: 0, elapsed: Duration::ZERO }
}

/// The biggest file types get strong, distinct colours; the long tail gets muted ones.
const PALETTE: [Color32; 18] = [
    Color32::from_rgb(62, 128, 204),
    Color32::from_rgb(236, 154, 58),
    Color32::from_rgb(226, 86, 86),
    Color32::from_rgb(70, 184, 168),
    Color32::from_rgb(120, 196, 98),
    Color32::from_rgb(152, 120, 214),
    Color32::from_rgb(244, 204, 82),
    Color32::from_rgb(238, 136, 170),
    Color32::from_rgb(96, 168, 228),
    Color32::from_rgb(196, 118, 72),
    Color32::from_rgb(110, 210, 214),
    Color32::from_rgb(176, 204, 88),
    Color32::from_rgb(212, 104, 168),
    Color32::from_rgb(132, 150, 228),
    Color32::from_rgb(230, 182, 122),
    Color32::from_rgb(84, 150, 120),
    Color32::from_rgb(200, 88, 120),
    Color32::from_rgb(160, 186, 216),
];

pub const RANKED_COLORS: usize = PALETTE.len();

fn color_for_rank(rank: usize, label: &str) -> Color32 {
    if rank < PALETTE.len() {
        return PALETTE[rank];
    }
    let hash = label.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (hash % 360) as f32 / 360.0;
    Hsva::new(hue, 0.30, 0.66, 1.0).into()
}
