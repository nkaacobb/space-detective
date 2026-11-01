//! Walks a folder in parallel and builds a flat tree of every file and folder with its size.

use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::cmp::Reverse;
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

    pub fn is_inside(&self, idx: u32, ancestor: u32) -> bool {
        let mut i = idx;
        loop {
            if i == ancestor {
                return true;
            }
            match self.node(i).parent {
                NO_PARENT => return false,
                p => i = p,
            }
        }
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
    let pool = rayon::ThreadPoolBuilder::new()
        // Folder reads mostly wait on the disk, so more threads than cores keeps it busy.
        .num_threads((std::thread::available_parallelism().map_or(4, |n| n.get()) * 2).min(32))
        .stack_size(16 << 20)
        .build()
        .expect("thread pool");
    let raw = pool.install(|| scan_dir(root, root.display().to_string(), progress));
    let mut tree = flatten(raw, root.to_path_buf());
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
}

fn scan_dir(path: &Path, name: String, progress: &Progress) -> RawNode {
    let mut node = RawNode { name, size: 0, is_dir: true, files: 0, dirs: 0, children: Vec::new() };
    if progress.cancel.load(Relaxed) {
        return node;
    }
    progress.dirs.fetch_add(1, Relaxed);
    if let Ok(mut current) = progress.current.try_lock() {
        current.clear();
        current.push_str(&path.to_string_lossy());
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        progress.errors.fetch_add(1, Relaxed);
        return node;
    };
    let mut subdirs = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            progress.errors.fetch_add(1, Relaxed);
            continue;
        };
        let Ok(file_type) = entry.file_type() else { continue };
        // Symlinks and junctions point at things counted elsewhere (or loop back on themselves).
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() {
            let path = entry.path();
            if !SKIP_DIRS.iter().any(|skip| path == Path::new(skip)) {
                subdirs.push((path, name));
            }
        } else {
            let size = entry.metadata().map_or(0, |m| m.len());
            progress.files.fetch_add(1, Relaxed);
            progress.bytes.fetch_add(size, Relaxed);
            node.children.push(RawNode { name, size, is_dir: false, files: 1, dirs: 0, children: Vec::new() });
        }
    }

    let sub: Vec<RawNode> = subdirs.into_par_iter().map(|(p, n)| scan_dir(&p, n, progress)).collect();
    node.children.extend(sub);
    for child in &node.children {
        node.size += child.size;
        node.files += if child.is_dir { child.files } else { 1 };
        node.dirs += if child.is_dir { child.dirs + 1 } else { 0 };
    }
    node.children.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    node
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
    exts[folder_ext as usize].color = Color32::from_rgb(96, 100, 112);

    Tree { root_path, nodes, exts, ext_rank, errors: 0, elapsed: Duration::ZERO }
}

/// The biggest file types get strong, distinct colours; the long tail gets muted ones.
const PALETTE: [Color32; 18] = [
    Color32::from_rgb(78, 121, 167),
    Color32::from_rgb(242, 142, 43),
    Color32::from_rgb(225, 87, 89),
    Color32::from_rgb(118, 183, 178),
    Color32::from_rgb(89, 161, 79),
    Color32::from_rgb(237, 201, 72),
    Color32::from_rgb(176, 122, 161),
    Color32::from_rgb(255, 157, 167),
    Color32::from_rgb(156, 117, 95),
    Color32::from_rgb(134, 188, 182),
    Color32::from_rgb(211, 114, 149),
    Color32::from_rgb(140, 209, 125),
    Color32::from_rgb(182, 153, 45),
    Color32::from_rgb(73, 152, 148),
    Color32::from_rgb(241, 206, 99),
    Color32::from_rgb(160, 203, 232),
    Color32::from_rgb(255, 190, 125),
    Color32::from_rgb(190, 160, 220),
];

pub const RANKED_COLORS: usize = PALETTE.len();

fn color_for_rank(rank: usize, label: &str) -> Color32 {
    if rank < PALETTE.len() {
        return PALETTE[rank];
    }
    let hash = label.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    let hue = (hash % 360) as f32 / 360.0;
    Hsva::new(hue, 0.22, 0.62, 1.0).into()
}
