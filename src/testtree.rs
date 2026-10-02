// Copyright (c) 2026 Mikko Tanner. All rights reserved.

/*!
Synthetic directory trees for tests and benchmarks.

A [TreeSpec] describes a tree level by level: how many subdirectories
each directory gets, and how many files. [TreeSpec::plan] lists every
entry without touching the disk, which makes it the expectation to test
a scanner against; [TreeSpec::create] writes the tree out, in parallel.

```
use miniutils::{EntryKind, TreeSpec};
use std::path::Path;

// 2 top-level dirs with 1 file each, each holding 3 subdirs with 4 files
let spec = TreeSpec::new().level(2, 1).level(3, 4);
let counts = spec.counts();
assert_eq!((counts.dirs, counts.files), (2 + 2 * 3, 2 + 2 * 3 * 4));

let plan: Vec<_> = spec.plan(Path::new("/tmp/t")).collect();
assert_eq!(plan[0].path, Path::new("/tmp/t/level_1_0"));
assert_eq!(plan[0].kind, EntryKind::Dir);
```
*/

use std::{
    fmt::{self, Debug, Formatter},
    fs::{self, File},
    io::{self, Write},
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
        Arc, Mutex,
    },
    thread::{self, available_parallelism},
};

/// Zeros written into files with a size, one chunk at a time.
static ZEROS: [u8; ZERO_CHUNK] = [0; ZERO_CHUNK];
const ZERO_CHUNK: usize = 64 * 1024;
/// Work units per thread to split the tree into, so that threads finishing early find more work.
const UNITS_PER_THREAD: u64 = 4;
/// Entries a thread creates between two progress reports.
const PROGRESS_BATCH: u64 = 256;

/**
Names an entry from its index path: the 0-based index of each directory
from the top level down, and for a file, its own index last. A directory
at depth 2 gets e.g. `[0, 3]`, the second file in it `[0, 3, 1]`, the
first file in the root `[0]`.
*/
pub type NameFn = Arc<dyn Fn(&[u64]) -> String + Send + Sync>;

/// File sizes of a [TreeSpec].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileSize {
    /// Empty files.
    #[default]
    Zero,
    /// Every file this many bytes.
    Fixed(u64),
    /**
    Each file `min..=max` bytes, pseudo-random but determined by `seed`
    and the file's index path: the same spec gives the same sizes on
    every run, whatever the thread scheduling.
    */
    Range { min: u64, max: u64, seed: u64 },
}

/// The kind of a [PlannedEntry].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    Dir,
    File,
}

/// One entry of a [TreeSpec], as listed by [TreeSpec::plan].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlannedEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    /// The file size; 0 for a directory.
    pub size: u64,
}

/// Entry totals of a tree: planned ([TreeSpec::counts]) or created ([TreeSpec::create]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
}

/**
Options for [TreeSpec::create_with]. `threads: 0` (the default) uses
all available CPUs. `progress` is called from the creating threads with
the number of entries created since its last call, every few hundred
entries and once more at the end.
*/
#[derive(Clone, Copy, Default)]
pub struct CreateOpts<'a> {
    pub threads: usize,
    pub progress: Option<&'a (dyn Fn(u64) + Sync)>,
}

/// One directory depth: subdirectories per parent directory, files per directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Level {
    dirs: u64,
    files: u64,
}

/**
A directory tree described level by level. The root itself is the
directory the tree is created in; it gets `root_files` files, and each
[TreeSpec::level] adds one depth below the previous one.

Default names are `level_{depth}_{index}` for directories and
`file-{index path, joined by _}.bin` for files; see [NameFn] for
the index paths, and [TreeSpec::dir_names] / [TreeSpec::file_names].
*/
#[derive(Clone)]
pub struct TreeSpec {
    root_files: u64,
    levels: Vec<Level>,
    file_size: FileSize,
    dir_name: NameFn,
    file_name: NameFn,
}

/**
The entries of a [TreeSpec], parents before their contents: in each
directory its files first, then each subdirectory followed by everything
below it. Paths are built as the iterator goes, so a plan of millions of
entries takes no more memory than one path per depth.
*/
pub struct Plan<'s> {
    spec: &'s TreeSpec,
    stack: Vec<Frame>,
    /// Deepest directory depth to list; files are listed in every directory listed.
    max_depth: usize,
    /// An entry to return before the stack, the top directory of a subtree plan.
    first: Option<PlannedEntry>,
}

/// A directory of a [Plan] being listed.
struct Frame {
    path: PathBuf,
    idx: Vec<u64>,
    files_next: u64,
    dirs_next: u64,
}

/// State shared by the threads of [TreeSpec::create_with].
struct CreateCtx<'a> {
    stop: AtomicBool,
    error: Mutex<Option<io::Error>>,
    dirs: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
    progress: Option<&'a (dyn Fn(u64) + Sync)>,
}

/* ######################################################################### */

impl TreeSpec {
    /// An empty spec: just the root, no files.
    pub fn new() -> Self {
        Self {
            root_files: 0,
            levels: Vec::new(),
            file_size: FileSize::Zero,
            dir_name: Arc::new(default_dir_name),
            file_name: Arc::new(default_file_name),
        }
    }

    /// Files directly in the root directory.
    pub fn root_files(mut self, files: u64) -> Self {
        self.root_files = files;
        self
    }

    /// Add a depth: `dirs` subdirectories in each directory of the previous depth, `files` files in each.
    pub fn level(mut self, dirs: u64, files: u64) -> Self {
        self.levels.push(Level { dirs, files });
        self
    }

    /// The sizes of the files.
    pub fn file_size(mut self, size: FileSize) -> Self {
        self.file_size = size;
        self
    }

    /// Name directories with `f` instead of `level_{depth}_{index}`.
    pub fn dir_names<F>(mut self, f: F) -> Self
    where
        F: Fn(&[u64]) -> String + Send + Sync + 'static,
    {
        self.dir_name = Arc::new(f);
        self
    }

    /// Name files with `f` instead of `file-{index path}.bin`.
    pub fn file_names<F>(mut self, f: F) -> Self
    where
        F: Fn(&[u64]) -> String + Send + Sync + 'static,
    {
        self.file_name = Arc::new(f);
        self
    }

    /// The number of directory depths below the root.
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /**
    The entry totals of the tree, the root itself not included. Computed
    without listing the entries, except the bytes of [FileSize::Range],
    which take one pass over the files.
    */
    pub fn counts(&self) -> Counts {
        let mut counts = Counts { files: self.root_files, ..Counts::default() };
        let mut dirs_at_depth: u64 = 1;
        for level in &self.levels {
            dirs_at_depth = dirs_at_depth.saturating_mul(level.dirs);
            counts.dirs = counts.dirs.saturating_add(dirs_at_depth);
            counts.files = counts.files.saturating_add(dirs_at_depth.saturating_mul(level.files));
        }
        counts.bytes = match self.file_size {
            FileSize::Zero => 0,
            FileSize::Fixed(size) => counts.files.saturating_mul(size),
            FileSize::Range { .. } => self.plan(Path::new("")).map(|e: PlannedEntry| e.size).sum(),
        };
        counts
    }

    /// Every entry of the tree under `root` (not `root` itself), parents first. See [Plan].
    pub fn plan(&self, root: &Path) -> Plan<'_> {
        Plan::new(self, root.to_path_buf(), Vec::new(), usize::MAX, None)
    }

    /// [TreeSpec::create_with] with the default [CreateOpts].
    pub fn create<P: AsRef<Path>>(&self, root: P) -> io::Result<Counts> {
        self.create_with(root, &CreateOpts::default())
    }

    /**
    Create the tree in `root`, which must be an existing, empty directory.
    Returns the totals created. Stops at the first error, which names the
    path it occurred on; the entries created until then are left in place.

    Each thread creates whole subtrees, so a subtree's directories are
    always created before their contents.
    */
    pub fn create_with<P: AsRef<Path>>(&self, root: P, opts: &CreateOpts) -> io::Result<Counts> {
        let root: &Path = root.as_ref();
        if fs::read_dir(root).map_err(|e| with_path(e, root))?.next().is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{}: target directory is not empty", root.display()),
            ));
        }
        let threads: usize = match opts.threads {
            0 => available_parallelism().map_or(1, NonZeroUsize::get),
            n => n,
        };
        let ctx = CreateCtx {
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
            dirs: AtomicU64::new(0),
            files: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            progress: opts.progress,
        };

        match self.split_depth(threads) {
            None => ctx.create(self.plan(root)),
            Some(depth) => {
                // everything above the split depth, then its subtrees in parallel
                ctx.create(Plan::new(self, root.to_path_buf(), Vec::new(), depth - 1, None));
                let units: u64 = self.levels[..depth].iter().map(|l: &Level| l.dirs).product();
                let next: AtomicU64 = AtomicU64::new(0);
                thread::scope(|s| {
                    for _ in 0..threads {
                        s.spawn(|| loop {
                            let unit: u64 = next.fetch_add(1, Relaxed);
                            if unit >= units || ctx.stop.load(Relaxed) {
                                break;
                            }
                            ctx.create(self.subtree_plan(root, self.unit_idx(depth, unit)));
                        });
                    }
                });
            }
        }

        match ctx.error.into_inner().unwrap_or_else(|e| e.into_inner()) {
            Some(e) => Err(e),
            None => Ok(Counts {
                dirs: ctx.dirs.into_inner(),
                files: ctx.files.into_inner(),
                bytes: ctx.bytes.into_inner(),
            }),
        }
    }

    /* --------------------------------- */

    /// Files in each directory at `depth` (the root is depth 0).
    fn files_at(&self, depth: usize) -> u64 {
        match depth {
            0 => self.root_files,
            d => self.levels.get(d - 1).map_or(0, |l: &Level| l.files),
        }
    }

    /// Subdirectories in each directory at `depth`.
    fn dirs_below(&self, depth: usize) -> u64 {
        self.levels.get(depth).map_or(0, |l: &Level| l.dirs)
    }

    /// The size of the file at `idx`.
    fn size_of(&self, idx: &[u64]) -> u64 {
        match self.file_size {
            FileSize::Zero => 0,
            FileSize::Fixed(size) => size,
            FileSize::Range { min, max, seed } => {
                let (lo, hi) = (min.min(max), min.max(max));
                match (hi - lo).checked_add(1) {
                    Some(span) => lo + idx_hash(seed, idx) % span,
                    None => idx_hash(seed, idx), // the full u64 range
                }
            }
        }
    }

    /**
    The depth whose directories are the units of parallel creation: the
    shallowest with enough of them to keep `threads` busy, else the
    deepest. [None] for a single thread or no directories to split by.
    */
    fn split_depth(&self, threads: usize) -> Option<usize> {
        if threads <= 1 {
            return None;
        }
        let target: u64 = (threads as u64).saturating_mul(UNITS_PER_THREAD);
        let mut units: u64 = 1;
        for (i, level) in self.levels.iter().enumerate() {
            units = units.saturating_mul(level.dirs);
            if units == 0 {
                return None;
            }
            if units >= target {
                return Some(i + 1);
            }
        }
        match units > 1 {
            true => Some(self.levels.len()),
            false => None,
        }
    }

    /// The index path of the `unit`th directory at `depth`, in plan order.
    fn unit_idx(&self, depth: usize, mut unit: u64) -> Vec<u64> {
        let mut idx: Vec<u64> = vec![0; depth];
        for d in (0..depth).rev() {
            let dirs: u64 = self.levels[d].dirs;
            idx[d] = unit % dirs;
            unit /= dirs;
        }
        idx
    }

    /// The directory at `idx` and everything below it, that directory first.
    fn subtree_plan(&self, root: &Path, idx: Vec<u64>) -> Plan<'_> {
        let mut path: PathBuf = root.to_path_buf();
        for depth in 1..=idx.len() {
            path.push((self.dir_name)(&idx[..depth]));
        }
        let first = PlannedEntry { path: path.clone(), kind: EntryKind::Dir, size: 0 };
        Plan::new(self, path, idx, usize::MAX, Some(first))
    }
}

impl Default for TreeSpec {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for TreeSpec {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("TreeSpec")
            .field("root_files", &self.root_files)
            .field("levels", &self.levels)
            .field("file_size", &self.file_size)
            .finish_non_exhaustive()
    }
}

impl Debug for CreateOpts<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateOpts")
            .field("threads", &self.threads)
            .field("progress", &self.progress.is_some())
            .finish()
    }
}

/* ######################################################################### */

impl<'s> Plan<'s> {
    fn new(
        spec: &'s TreeSpec,
        path: PathBuf,
        idx: Vec<u64>,
        max_depth: usize,
        first: Option<PlannedEntry>,
    ) -> Self {
        let top = Frame { path, idx, files_next: 0, dirs_next: 0 };
        Self { spec, stack: vec![top], max_depth, first }
    }
}

impl Iterator for Plan<'_> {
    type Item = PlannedEntry;

    fn next(&mut self) -> Option<PlannedEntry> {
        if let Some(first) = self.first.take() {
            return Some(first);
        }
        let spec: &TreeSpec = self.spec;
        loop {
            let frame: &mut Frame = self.stack.last_mut()?;
            let depth: usize = frame.idx.len();
            if frame.files_next < spec.files_at(depth) {
                let mut idx: Vec<u64> = frame.idx.clone();
                idx.push(frame.files_next);
                frame.files_next += 1;
                let path: PathBuf = frame.path.join((spec.file_name)(&idx));
                return Some(PlannedEntry { path, kind: EntryKind::File, size: spec.size_of(&idx) });
            }
            if depth < self.max_depth && frame.dirs_next < spec.dirs_below(depth) {
                let mut idx: Vec<u64> = frame.idx.clone();
                idx.push(frame.dirs_next);
                frame.dirs_next += 1;
                let path: PathBuf = frame.path.join((spec.dir_name)(&idx));
                let child = Frame { path: path.clone(), idx, files_next: 0, dirs_next: 0 };
                self.stack.push(child);
                return Some(PlannedEntry { path, kind: EntryKind::Dir, size: 0 });
            }
            self.stack.pop();
        }
    }
}

/* ######################################################################### */

impl CreateCtx<'_> {
    /// Create the entries of `plan`, until done or until any thread fails.
    fn create(&self, plan: Plan) {
        let (mut dirs, mut files, mut bytes, mut unreported) = (0u64, 0u64, 0u64, 0u64);
        for entry in plan {
            if self.stop.load(Relaxed) {
                break;
            }
            if let Err(e) = create_entry(&entry) {
                self.fail(with_path(e, &entry.path));
                break;
            }
            match entry.kind {
                EntryKind::Dir => dirs += 1,
                EntryKind::File => {
                    files += 1;
                    bytes += entry.size;
                }
            }
            unreported += 1;
            if unreported == PROGRESS_BATCH {
                self.report(unreported);
                unreported = 0;
            }
        }
        self.dirs.fetch_add(dirs, Relaxed);
        self.files.fetch_add(files, Relaxed);
        self.bytes.fetch_add(bytes, Relaxed);
        if unreported > 0 {
            self.report(unreported);
        }
    }

    fn report(&self, n: u64) {
        if let Some(progress) = self.progress {
            progress(n);
        }
    }

    /// Record the first error and tell all threads to stop.
    fn fail(&self, e: io::Error) {
        self.stop.store(true, Relaxed);
        let mut slot = self.error.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert(e);
    }
}

/// Create one planned entry; never overwrites an existing one.
fn create_entry(entry: &PlannedEntry) -> io::Result<()> {
    match entry.kind {
        EntryKind::Dir => fs::create_dir(&entry.path),
        EntryKind::File => {
            let mut file: File = File::create_new(&entry.path)?;
            let mut left: u64 = entry.size;
            while left > 0 {
                let n: usize = left.min(ZERO_CHUNK as u64) as usize;
                file.write_all(&ZEROS[..n])?;
                left -= n as u64;
            }
            Ok(())
        }
    }
}

/// `e` with the path it occurred on in its message.
fn with_path(e: io::Error, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

fn default_dir_name(idx: &[u64]) -> String {
    format!("level_{}_{}", idx.len(), idx.last().copied().unwrap_or(0))
}

fn default_file_name(idx: &[u64]) -> String {
    let parts: Vec<String> = idx.iter().map(u64::to_string).collect();
    format!("file-{}.bin", parts.join("_"))
}

/// A well-mixed hash of `seed` and `idx` (splitmix64 steps), for [FileSize::Range].
fn idx_hash(seed: u64, idx: &[u64]) -> u64 {
    let mix = |mut z: u64| -> u64 {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    idx.iter().fold(mix(seed), |h: u64, &i: &u64| mix(h ^ i))
}

/* ######################################################################### */

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    /// What is on disk under `root`, as plan entries.
    fn on_disk(root: &Path) -> HashSet<PlannedEntry> {
        let mut found: HashSet<PlannedEntry> = HashSet::new();
        let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let meta = entry.metadata().unwrap();
                let (kind, size) = match meta.is_dir() {
                    true => (EntryKind::Dir, 0),
                    false => (EntryKind::File, meta.len()),
                };
                if meta.is_dir() {
                    dirs.push(entry.path());
                }
                found.insert(PlannedEntry { path: entry.path(), kind, size });
            }
        }
        found
    }

    fn spec() -> TreeSpec {
        TreeSpec::new().root_files(2).level(3, 1).level(4, 5).level(2, 0)
    }

    #[test]
    fn counts_match_plan() {
        let spec = spec().file_size(FileSize::Range { min: 1, max: 100, seed: 7 });
        let plan: Vec<PlannedEntry> = spec.plan(Path::new("/r")).collect();
        let dirs = plan.iter().filter(|e| e.kind == EntryKind::Dir).count() as u64;
        let bytes: u64 = plan.iter().map(|e| e.size).sum();
        let counts = spec.counts();
        assert_eq!(counts.dirs, 3 + 3 * 4 + 3 * 4 * 2);
        assert_eq!(counts.files, 2 + 3 + 3 * 4 * 5);
        assert_eq!((counts.dirs, counts.files, counts.bytes), (dirs, plan.len() as u64 - dirs, bytes));
        assert!(plan.iter().all(|e| e.kind == EntryKind::Dir || (1..=100).contains(&e.size)));
    }

    #[test]
    fn plan_lists_parents_first() {
        let mut seen: HashSet<PathBuf> = HashSet::from([PathBuf::from("/r")]);
        for e in spec().plan(Path::new("/r")) {
            assert!(seen.contains(e.path.parent().unwrap()), "{} before its parent", e.path.display());
            assert!(seen.insert(e.path.clone()), "{} listed twice", e.path.display());
        }
    }

    #[test]
    fn default_and_custom_names() {
        let plan: Vec<PathBuf> = spec().plan(Path::new("/r")).map(|e| e.path).collect();
        assert_eq!(plan[0], Path::new("/r/file-0.bin"));
        assert!(plan.contains(&PathBuf::from("/r/level_1_2/level_2_3/file-2_3_4.bin")));
        assert!(plan.contains(&PathBuf::from("/r/level_1_2/level_2_3/level_3_1")));

        let spec = spec()
            .dir_names(|idx| format!("d{}", idx.len()))
            .file_names(|idx| format!("f{}", idx.last().unwrap()));
        let plan: Vec<PathBuf> = spec.plan(Path::new("/r")).map(|e| e.path).collect();
        assert!(plan.contains(&PathBuf::from("/r/d1/d2/f4")));
    }

    #[test]
    fn create_matches_plan() {
        let spec = spec().file_size(FileSize::Range { min: 0, max: 70_000, seed: 3 });
        for threads in [1, 2, 8] {
            let temp: TempDir = TempDir::new().unwrap();
            let reported: AtomicU64 = AtomicU64::new(0);
            let progress = |n: u64| {
                reported.fetch_add(n, Relaxed);
            };
            let opts = CreateOpts { threads, progress: Some(&progress) };
            let counts: Counts = spec.create_with(temp.path(), &opts).unwrap();
            assert_eq!(counts, spec.counts(), "threads={threads}");
            assert_eq!(reported.load(Relaxed), counts.dirs + counts.files, "threads={threads}");
            let planned: HashSet<PlannedEntry> = spec.plan(temp.path()).collect();
            assert_eq!(on_disk(temp.path()), planned, "threads={threads}");
        }
    }

    #[test]
    fn create_flat_and_empty() {
        // no levels: root files only, and nothing at all
        for spec in [TreeSpec::new().root_files(5), TreeSpec::new(), TreeSpec::new().level(0, 3)] {
            let temp: TempDir = TempDir::new().unwrap();
            assert_eq!(spec.create(temp.path()).unwrap(), spec.counts(), "{spec:?}");
            assert_eq!(on_disk(temp.path()).len() as u64, spec.counts().files, "{spec:?}");
        }
    }

    #[test]
    fn create_refuses_non_empty_target() {
        let temp: TempDir = TempDir::new().unwrap();
        fs::write(temp.path().join("x"), b"").unwrap();
        let err = spec().create(temp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn create_stops_on_error() {
        // a name collision: two dirs named alike, the second create fails
        let spec = TreeSpec::new().level(2, 0).level(1, 1).dir_names(|_| "same".into());
        let temp: TempDir = TempDir::new().unwrap();
        let err = spec.create_with(temp.path(), &CreateOpts { threads: 1, ..Default::default() });
        let err = err.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(err.to_string().contains("same"), "{err}");
    }

    #[test]
    fn range_sizes_are_deterministic() {
        let spec = spec().file_size(FileSize::Range { min: 10, max: 20, seed: 42 });
        let a: Vec<u64> = spec.plan(Path::new("/r")).map(|e| e.size).collect();
        let b: Vec<u64> = spec.plan(Path::new("/r")).map(|e| e.size).collect();
        assert_eq!(a, b);
        let other = spec.clone().file_size(FileSize::Range { min: 10, max: 20, seed: 43 });
        let c: Vec<u64> = other.plan(Path::new("/r")).map(|e| e.size).collect();
        assert_ne!(a, c, "the seed changes the sizes");
    }

    #[test]
    fn split_units_cover_the_tree() {
        // the units at the split depth, created in index order, are the plan's directories there
        let spec = spec();
        for threads in [2, 3, 16] {
            let depth: usize = spec.split_depth(threads).unwrap();
            let units: u64 = spec.levels[..depth].iter().map(|l| l.dirs).product();
            let from_units: Vec<PathBuf> = (0..units)
                .map(|u| spec.subtree_plan(Path::new("/r"), spec.unit_idx(depth, u)).next().unwrap().path)
                .collect();
            let from_plan: Vec<PathBuf> = spec
                .plan(Path::new("/r"))
                .filter(|e| e.kind == EntryKind::Dir && e.path.components().count() == depth + 2)
                .map(|e| e.path)
                .collect();
            assert_eq!(from_units, from_plan, "threads={threads}");
        }
    }
}
