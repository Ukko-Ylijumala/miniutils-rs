// Copyright (c) 2026 Mikko Tanner. All rights reserved.

/*!
Synthetic directory trees for tests and benchmarks.

A [TreeSpec] describes a tree level by level: how many subdirectories
each directory gets, how many files, and how many [Special] entries
(symlinks, hardlinks, FIFOs, sockets). [TreeSpec::plan] lists every
entry without touching the disk, which makes it the expectation to test
a scanner against; [TreeSpec::create] writes the tree out, in parallel.

```
use miniutils::{EntryKind, Special, TreeSpec};
use std::path::Path;

// 2 top-level dirs with 1 file each, each holding 3 subdirs with 4 files and a FIFO
let spec = TreeSpec::new().level(2, 1).level(3, 4).with(Special::Fifo, 1);
let counts = spec.counts();
assert_eq!((counts.dirs, counts.files), (2 + 2 * 3, 2 + 2 * 3 * 4));
assert_eq!(counts.special(Special::Fifo), 2 * 3);

let plan: Vec<_> = spec.plan(Path::new("/tmp/t")).collect();
assert_eq!(plan[0].path, Path::new("/tmp/t/level_1_0"));
assert_eq!(plan[0].kind, EntryKind::Dir);
```
*/

use std::{
    array,
    ffi::OsString,
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

#[cfg(unix)]
use std::{
    ffi::CString,
    os::unix::{ffi::OsStrExt, fs::symlink, net::UnixListener},
};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;

/// Zeros written into files with a size, one chunk at a time.
static ZEROS: [u8; ZERO_CHUNK] = [0; ZERO_CHUNK];
const ZERO_CHUNK: usize = 64 * 1024;
/// Work units per thread to split the tree into, so that threads finishing early find more work.
const UNITS_PER_THREAD: u64 = 4;
/// Entries a thread creates between two progress reports.
const PROGRESS_BATCH: u64 = 256;
/// What a [Special::SymlinkDangling] points to; no default name looks like it.
const DANGLING_TARGET: &str = ".dangling-target";
/// Permissions of a created FIFO (before the umask).
#[cfg(unix)]
const FIFO_MODE: libc::mode_t = 0o644;
/// Longest path a socket can be bound to: `sockaddr_un.sun_path` holds 108 bytes, NUL included.
#[cfg(unix)]
const SUN_PATH_MAX: usize = 107;

/**
Names an entry from its index path: the 0-based index of each directory
from the top level down, and for a file or special entry its own index
last. A directory at depth 2 gets e.g. `[0, 3]`, the second file in it
`[0, 3, 1]`, the first file in the root `[0]`. Special entries are
numbered per [Special] kind.

Names are [OsString]s, so they need not be UTF-8; the builder methods
take any closure returning something [`Into<OsString>`], a [String] too.
*/
pub type NameFn = Arc<dyn Fn(&[u64]) -> OsString + Send + Sync>;

/// Names a [Special] entry from its kind and index path, see [NameFn].
pub type SpecialNameFn = Arc<dyn Fn(Special, &[u64]) -> OsString + Send + Sync>;

/**
Entries other than regular files and directories, added to a level with
[TreeSpec::with]. Links point within their own directory, so a subtree
stays self-contained. All of them need a Unix platform.
*/
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Special {
    /// A symlink to the first regular file of its directory.
    SymlinkFile,
    /// A symlink to `..`: a loop for anything following symlinks.
    SymlinkDir,
    /// A symlink to a name that does not exist.
    SymlinkDangling,
    /// A symlink to itself: `ELOOP` when followed.
    SymlinkSelf,
    /// A hardlink to the first regular file of its directory.
    Hardlink,
    /// A named pipe.
    Fifo,
    /// A Unix domain socket, left behind by a listener that is gone.
    Socket,
}

/// The kind of a [PlannedEntry].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EntryKind {
    Dir,
    File,
    Special(Special),
}

/// One entry of a [TreeSpec], as listed by [TreeSpec::plan].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct PlannedEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    /// The size of a file, or of the file a hardlink shares; 0 for anything else.
    pub size: u64,
    /**
    What a link points to: a symlink's target as stored in the link
    (relative to its directory), a hardlink's file by its full path.
    */
    pub target: Option<PathBuf>,
}

/**
Entry totals of a tree: planned ([TreeSpec::counts]) or created
([TreeSpec::create]). `files` and `bytes` are regular files only: a
hardlink is a [Special], and its data is that of the file it shares.
*/
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Counts {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
    /// Per [Special] kind, in [Special::ALL] order; see [Counts::special].
    pub specials: [u64; Special::COUNT],
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

/// One directory depth: subdirectories per parent directory, files and specials per directory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Level {
    dirs: u64,
    files: u64,
    specials: [u64; Special::COUNT],
}

/**
A directory tree described level by level. The root itself is the
directory the tree is created in; it gets `root_files` files, and each
[TreeSpec::level] adds one depth below the previous one.

Default names are `level_{depth}_{index}` for directories,
`file-{index path, joined by _}.bin` for files and
`{special name}-{index path}` for [Special] entries (e.g. `fifo-0_2_0`);
see [NameFn] for the index paths, and the `*_names()` methods to change them.
*/
#[derive(Clone)]
pub struct TreeSpec {
    root: Level,
    levels: Vec<Level>,
    file_size: FileSize,
    dir_name: NameFn,
    file_name: NameFn,
    special_name: SpecialNameFn,
}

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

/**
The entries of a [TreeSpec], parents before their contents: in each
directory its files first, then its special entries (so that links find
their targets), then each subdirectory followed by everything below it.
Paths are built as the iterator goes, so a plan of millions of entries
takes no more memory than one path per depth.
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
    /// The [Special] kind being listed, as an index into [Special::ALL], and the next of it.
    special_kind: usize,
    special_next: u64,
    dirs_next: u64,
}

/// State shared by the threads of [TreeSpec::create_with].
struct CreateCtx<'a> {
    stop: AtomicBool,
    error: Mutex<Option<io::Error>>,
    dirs: AtomicU64,
    files: AtomicU64,
    bytes: AtomicU64,
    specials: [AtomicU64; Special::COUNT],
    progress: Option<&'a (dyn Fn(u64) + Sync)>,
}

/* ######################################################################### */

impl Special {
    /// Every kind, in the order a directory's special entries are listed and counted.
    pub const ALL: [Special; 7] = [
        Self::SymlinkFile,
        Self::SymlinkDir,
        Self::SymlinkDangling,
        Self::SymlinkSelf,
        Self::Hardlink,
        Self::Fifo,
        Self::Socket,
    ];
    /// The number of kinds.
    pub const COUNT: usize = Self::ALL.len();

    /// A short name, used for the default entry names.
    pub fn name(self) -> &'static str {
        match self {
            Self::SymlinkFile => "symlink_file",
            Self::SymlinkDir => "symlink_dir",
            Self::SymlinkDangling => "symlink_dangling",
            Self::SymlinkSelf => "symlink_self",
            Self::Hardlink => "hardlink",
            Self::Fifo => "fifo",
            Self::Socket => "socket",
        }
    }

    /// Whether this kind is a symbolic link.
    pub fn is_symlink(self) -> bool {
        matches!(
            self,
            Self::SymlinkFile | Self::SymlinkDir | Self::SymlinkDangling | Self::SymlinkSelf
        )
    }

    /// Whether this kind links to the first regular file of its directory.
    fn needs_file(self) -> bool {
        matches!(self, Self::SymlinkFile | Self::Hardlink)
    }

    /// The position in [Special::ALL].
    fn index(self) -> usize {
        Self::ALL.iter().position(|&k: &Special| k == self).unwrap_or(0)
    }
}

impl Counts {
    /// The number of [Special] entries of `kind`.
    pub fn special(&self, kind: Special) -> u64 {
        self.specials[kind.index()]
    }
}

impl TreeSpec {
    /// An empty spec: just the root, no files.
    pub fn new() -> Self {
        Self {
            root: Level::default(),
            levels: Vec::new(),
            file_size: FileSize::Zero,
            dir_name: Arc::new(default_dir_name),
            file_name: Arc::new(default_file_name),
            special_name: Arc::new(default_special_name),
        }
    }

    /// Files directly in the root directory.
    pub fn root_files(mut self, files: u64) -> Self {
        self.root.files = files;
        self
    }

    /// Add a depth: `dirs` subdirectories in each directory of the previous depth, `files` files in each.
    pub fn level(mut self, dirs: u64, files: u64) -> Self {
        self.levels.push(Level { dirs, files, ..Level::default() });
        self
    }

    /**
    `count` [Special] entries of `kind` in each directory of the level
    added last, or in the root before any [TreeSpec::level]. Replaces an
    earlier count of the same kind there.
    */
    pub fn with(mut self, kind: Special, count: u64) -> Self {
        let level: &mut Level = self.levels.last_mut().unwrap_or(&mut self.root);
        level.specials[kind.index()] = count;
        self
    }

    /// The sizes of the files.
    pub fn file_size(mut self, size: FileSize) -> Self {
        self.file_size = size;
        self
    }

    /// Name directories with `f` instead of `level_{depth}_{index}`.
    pub fn dir_names<F, S>(mut self, f: F) -> Self
    where
        F: Fn(&[u64]) -> S + Send + Sync + 'static,
        S: Into<OsString>,
    {
        self.dir_name = Arc::new(move |idx: &[u64]| f(idx).into());
        self
    }

    /// Name files with `f` instead of `file-{index path}.bin`.
    pub fn file_names<F, S>(mut self, f: F) -> Self
    where
        F: Fn(&[u64]) -> S + Send + Sync + 'static,
        S: Into<OsString>,
    {
        self.file_name = Arc::new(move |idx: &[u64]| f(idx).into());
        self
    }

    /// Name [Special] entries with `f` instead of `{special name}-{index path}`.
    pub fn special_names<F, S>(mut self, f: F) -> Self
    where
        F: Fn(Special, &[u64]) -> S + Send + Sync + 'static,
        S: Into<OsString>,
    {
        self.special_name = Arc::new(move |kind: Special, idx: &[u64]| f(kind, idx).into());
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
        let mut counts = Counts {
            files: self.root.files,
            specials: self.root.specials,
            ..Counts::default()
        };
        let mut dirs_at_depth: u64 = 1;
        for level in &self.levels {
            dirs_at_depth = dirs_at_depth.saturating_mul(level.dirs);
            counts.dirs = counts.dirs.saturating_add(dirs_at_depth);
            counts.files = counts.files.saturating_add(dirs_at_depth.saturating_mul(level.files));
            for (total, &each) in counts.specials.iter_mut().zip(&level.specials) {
                *total = total.saturating_add(dirs_at_depth.saturating_mul(each));
            }
        }
        counts.bytes = match self.file_size {
            FileSize::Zero => 0,
            FileSize::Fixed(size) => counts.files.saturating_mul(size),
            FileSize::Range { .. } => self
                .plan(Path::new(""))
                .filter(|e: &PlannedEntry| e.kind == EntryKind::File)
                .map(|e: PlannedEntry| e.size)
                .sum(),
        };
        counts
    }

    /**
    Check that the spec can be created: every level with
    [Special::SymlinkFile] or [Special::Hardlink] entries needs a regular
    file in each directory to link to. [TreeSpec::create] checks this
    first; [TreeSpec::plan] lists such links regardless.
    */
    pub fn validate(&self) -> io::Result<()> {
        for (depth, level) in std::iter::once(&self.root).chain(&self.levels).enumerate() {
            if level.files > 0 {
                continue;
            }
            for kind in Special::ALL.into_iter().filter(|k: &Special| k.needs_file()) {
                if level.specials[kind.index()] > 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("depth {depth}: {} needs a file to link to", kind.name()),
                    ));
                }
            }
        }
        Ok(())
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
    [Special] entries need a Unix platform.

    Each thread creates whole subtrees, so a subtree's directories are
    always created before their contents.
    */
    pub fn create_with<P: AsRef<Path>>(&self, root: P, opts: &CreateOpts) -> io::Result<Counts> {
        let root: &Path = root.as_ref();
        self.validate()?;
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
            specials: array::from_fn(|_| AtomicU64::new(0)),
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
                specials: ctx.specials.map(AtomicU64::into_inner),
            }),
        }
    }

    /* --------------------------------- */

    /// The level of the directories at `depth` (the root is depth 0).
    fn level_at(&self, depth: usize) -> Option<&Level> {
        match depth {
            0 => Some(&self.root),
            d => self.levels.get(d - 1),
        }
    }

    /// Files in each directory at `depth`.
    fn files_at(&self, depth: usize) -> u64 {
        self.level_at(depth).map_or(0, |l: &Level| l.files)
    }

    /// Special entries of the kind at `kind_idx` in each directory at `depth`.
    fn specials_at(&self, depth: usize, kind_idx: usize) -> u64 {
        self.level_at(depth).map_or(0, |l: &Level| l.specials[kind_idx])
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

    /// The planned [Special] entry at `idx` in the directory `dir` (index path `dir_idx`).
    fn special_entry(&self, dir: &Path, dir_idx: &[u64], kind: Special, idx: &[u64]) -> PlannedEntry {
        let name: OsString = (self.special_name)(kind, idx);
        let path: PathBuf = dir.join(&name);
        let mut file_idx: Vec<u64> = dir_idx.to_vec();
        file_idx.push(0); // the first regular file, the one links point to
        let (target, size): (Option<PathBuf>, u64) = match kind {
            Special::SymlinkFile => (Some((self.file_name)(&file_idx).into()), 0),
            Special::SymlinkDir => (Some(PathBuf::from("..")), 0),
            Special::SymlinkDangling => (Some(PathBuf::from(DANGLING_TARGET)), 0),
            Special::SymlinkSelf => (Some(name.into()), 0),
            Special::Hardlink => {
                let file: PathBuf = dir.join((self.file_name)(&file_idx));
                (Some(file), self.size_of(&file_idx))
            }
            Special::Fifo | Special::Socket => (None, 0),
        };
        PlannedEntry { path, kind: EntryKind::Special(kind), size, target }
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
        let first = PlannedEntry { path: path.clone(), kind: EntryKind::Dir, size: 0, target: None };
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
            .field("root", &self.root)
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
        Self { spec, stack: vec![Frame::new(path, idx)], max_depth, first }
    }
}

impl Frame {
    fn new(path: PathBuf, idx: Vec<u64>) -> Self {
        Self { path, idx, files_next: 0, special_kind: 0, special_next: 0, dirs_next: 0 }
    }

    /// The index path of the next entry in this directory, numbered by `next`.
    fn child_idx(&self, next: u64) -> Vec<u64> {
        let mut idx: Vec<u64> = Vec::with_capacity(self.idx.len() + 1);
        idx.extend_from_slice(&self.idx);
        idx.push(next);
        idx
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
                let idx: Vec<u64> = frame.child_idx(frame.files_next);
                frame.files_next += 1;
                return Some(PlannedEntry {
                    path: frame.path.join((spec.file_name)(&idx)),
                    kind: EntryKind::File,
                    size: spec.size_of(&idx),
                    target: None,
                });
            }
            while frame.special_kind < Special::COUNT {
                if frame.special_next < spec.specials_at(depth, frame.special_kind) {
                    let idx: Vec<u64> = frame.child_idx(frame.special_next);
                    frame.special_next += 1;
                    let kind: Special = Special::ALL[frame.special_kind];
                    return Some(spec.special_entry(&frame.path, &frame.idx, kind, &idx));
                }
                frame.special_kind += 1;
                frame.special_next = 0;
            }
            if depth < self.max_depth && frame.dirs_next < spec.dirs_below(depth) {
                let idx: Vec<u64> = frame.child_idx(frame.dirs_next);
                frame.dirs_next += 1;
                let path: PathBuf = frame.path.join((spec.dir_name)(&idx));
                self.stack.push(Frame::new(path.clone(), idx));
                return Some(PlannedEntry { path, kind: EntryKind::Dir, size: 0, target: None });
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
        let mut specials: [u64; Special::COUNT] = [0; Special::COUNT];
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
                EntryKind::Special(kind) => specials[kind.index()] += 1,
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
        for (total, n) in self.specials.iter().zip(specials) {
            total.fetch_add(n, Relaxed);
        }
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
        EntryKind::Special(kind) => create_special(kind, entry),
    }
}

/// Create a [Special] entry as planned.
#[cfg(unix)]
fn create_special(kind: Special, entry: &PlannedEntry) -> io::Result<()> {
    let target = || -> io::Result<&Path> {
        entry.target.as_deref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "a link without a target")
        })
    };
    match kind {
        Special::Hardlink => fs::hard_link(target()?, &entry.path),
        Special::Fifo => mkfifo(&entry.path),
        Special::Socket => bind_socket(&entry.path),
        _ => symlink(target()?, &entry.path), // the symlink kinds
    }
}

#[cfg(not(unix))]
fn create_special(kind: Special, _entry: &PlannedEntry) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{} entries need a Unix platform", kind.name()),
    ))
}

#[cfg(unix)]
fn mkfifo(path: &Path) -> io::Result<()> {
    let c_path: CString = CString::new(path.as_os_str().as_bytes())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    // SAFETY: c_path is a valid NUL-terminated string that outlives the call
    match unsafe { libc::mkfifo(c_path.as_ptr(), FIFO_MODE) } {
        0 => Ok(()),
        _ => Err(io::Error::last_os_error()),
    }
}

/**
Leave a socket file at `path`: bind a listener there and drop it. A path
too long for `sockaddr_un` is bound through `/proc/self/fd/<dir fd>/name`
on Linux, which resolves to the same directory.
*/
#[cfg(unix)]
fn bind_socket(path: &Path) -> io::Result<()> {
    if path.as_os_str().len() <= SUN_PATH_MAX {
        return UnixListener::bind(path).map(drop);
    }
    #[cfg(target_os = "linux")]
    if let (Some(dir), Some(name)) = (path.parent(), path.file_name()) {
        let dir: File = File::open(dir)?;
        let short: PathBuf = PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())).join(name);
        if short.as_os_str().len() <= SUN_PATH_MAX {
            return UnixListener::bind(&short).map(drop);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "socket path too long for sockaddr_un",
    ))
}

/// `e` with the path it occurred on in its message.
fn with_path(e: io::Error, path: &Path) -> io::Error {
    io::Error::new(e.kind(), format!("{}: {e}", path.display()))
}

/// An index path joined by `_`, for the default names.
fn join_idx(idx: &[u64]) -> String {
    let parts: Vec<String> = idx.iter().map(u64::to_string).collect();
    parts.join("_")
}

fn default_dir_name(idx: &[u64]) -> OsString {
    format!("level_{}_{}", idx.len(), idx.last().copied().unwrap_or(0)).into()
}

fn default_file_name(idx: &[u64]) -> OsString {
    format!("file-{}.bin", join_idx(idx)).into()
}

fn default_special_name(kind: Special, idx: &[u64]) -> OsString {
    format!("{}-{}", kind.name(), join_idx(idx)).into()
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

    #[cfg(unix)]
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{FileTypeExt, MetadataExt},
    };

    /// An entry as it can be seen on disk: a hardlink is a file, and symlink kinds look alike.
    #[derive(Debug, PartialEq, Eq, Hash)]
    struct DiskEntry {
        path: PathBuf,
        kind: &'static str,
        size: u64,
        target: Option<PathBuf>,
    }

    impl From<PlannedEntry> for DiskEntry {
        fn from(e: PlannedEntry) -> Self {
            let (kind, size, target) = match e.kind {
                EntryKind::Dir => ("dir", 0, None),
                EntryKind::File | EntryKind::Special(Special::Hardlink) => ("file", e.size, None),
                EntryKind::Special(Special::Fifo) => ("fifo", 0, None),
                EntryKind::Special(Special::Socket) => ("socket", 0, None),
                EntryKind::Special(_) => ("symlink", 0, e.target),
            };
            Self { path: e.path, kind, size, target }
        }
    }

    /// What is on disk under `root`, without following symlinks.
    fn on_disk(root: &Path) -> HashSet<DiskEntry> {
        let mut found: HashSet<DiskEntry> = HashSet::new();
        let mut dirs: Vec<PathBuf> = vec![root.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path: PathBuf = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                let ft = meta.file_type();
                let (kind, size, target) = if ft.is_dir() {
                    dirs.push(path.clone());
                    ("dir", 0, None)
                } else if ft.is_file() {
                    ("file", meta.len(), None)
                } else if ft.is_symlink() {
                    ("symlink", 0, Some(fs::read_link(&path).unwrap()))
                } else {
                    #[cfg(unix)]
                    match (ft.is_fifo(), ft.is_socket()) {
                        (true, _) => ("fifo", 0, None),
                        (_, true) => ("socket", 0, None),
                        _ => ("other", 0, None),
                    }
                    #[cfg(not(unix))]
                    ("other", 0, None)
                };
                found.insert(DiskEntry { path, kind, size, target });
            }
        }
        found
    }

    fn planned(spec: &TreeSpec, root: &Path) -> HashSet<DiskEntry> {
        spec.plan(root).map(DiskEntry::from).collect()
    }

    fn spec() -> TreeSpec {
        TreeSpec::new().root_files(2).level(3, 1).level(4, 5).level(2, 0)
    }

    /// The shape of [spec], with one of every special kind in the root and two at depth 2.
    fn special_spec() -> TreeSpec {
        let with_all = |spec: TreeSpec, n: u64| -> TreeSpec {
            Special::ALL.into_iter().fold(spec, |s: TreeSpec, k: Special| s.with(k, n))
        };
        let spec: TreeSpec = with_all(TreeSpec::new().root_files(2), 1).level(3, 1).level(4, 5);
        with_all(spec, 2).level(2, 0)
    }

    #[test]
    fn counts_match_plan() {
        for spec in [spec().file_size(FileSize::Range { min: 1, max: 100, seed: 7 }), special_spec()] {
            let plan: Vec<PlannedEntry> = spec.plan(Path::new("/r")).collect();
            let of = |k: EntryKind| plan.iter().filter(|e| e.kind == k).count() as u64;
            let counts = spec.counts();
            assert_eq!(counts.dirs, of(EntryKind::Dir), "{spec:?}");
            assert_eq!(counts.files, of(EntryKind::File), "{spec:?}");
            for kind in Special::ALL {
                assert_eq!(counts.special(kind), of(EntryKind::Special(kind)), "{kind:?}");
            }
            let bytes: u64 = plan.iter().filter(|e| e.kind == EntryKind::File).map(|e| e.size).sum();
            assert_eq!(counts.bytes, bytes);
        }
        let counts = spec().counts();
        assert_eq!(counts.dirs, 3 + 3 * 4 + 3 * 4 * 2);
        assert_eq!(counts.files, 2 + 3 + 3 * 4 * 5);
        assert_eq!(special_spec().counts().special(Special::Fifo), 1 + 2 * 3 * 4);
    }

    #[test]
    fn plan_lists_parents_first() {
        let mut seen: HashSet<PathBuf> = HashSet::from([PathBuf::from("/r")]);
        for e in special_spec().plan(Path::new("/r")) {
            assert!(seen.contains(e.path.parent().unwrap()), "{} before its parent", e.path.display());
            assert!(seen.insert(e.path.clone()), "{} listed twice", e.path.display());
            if e.kind == EntryKind::Special(Special::Hardlink) {
                assert!(seen.contains(e.target.as_ref().unwrap()), "{} before its file", e.path.display());
            }
        }
    }

    #[test]
    fn default_and_custom_names() {
        let plan: Vec<PathBuf> = special_spec().plan(Path::new("/r")).map(|e| e.path).collect();
        assert_eq!(plan[0], Path::new("/r/file-0.bin"));
        assert!(plan.contains(&PathBuf::from("/r/level_1_2/level_2_3/file-2_3_4.bin")));
        assert!(plan.contains(&PathBuf::from("/r/level_1_2/level_2_3/level_3_1")));
        assert!(plan.contains(&PathBuf::from("/r/fifo-0")));
        assert!(plan.contains(&PathBuf::from("/r/level_1_0/level_2_1/socket-0_1_1")));

        let spec = special_spec()
            .dir_names(|idx| format!("d{}", idx.len()))
            .file_names(|idx| format!("f{}", idx.last().unwrap()))
            .special_names(|kind, idx| format!("{}{}", &kind.name()[..2], idx.last().unwrap()));
        let plan: Vec<PathBuf> = spec.plan(Path::new("/r")).map(|e| e.path).collect();
        assert!(plan.contains(&PathBuf::from("/r/d1/d2/f4")));
        assert!(plan.contains(&PathBuf::from("/r/d1/d2/fi1")));
    }

    #[test]
    fn link_targets() {
        let plan: Vec<PlannedEntry> = special_spec().plan(Path::new("/r")).collect();
        let find = |p: &str| plan.iter().find(|e| e.path == Path::new(p)).unwrap();
        assert_eq!(find("/r/symlink_file-0").target.as_deref(), Some(Path::new("file-0.bin")));
        assert_eq!(find("/r/symlink_dir-0").target.as_deref(), Some(Path::new("..")));
        assert_eq!(find("/r/symlink_self-0").target.as_deref(), Some(Path::new("symlink_self-0")));
        assert_eq!(find("/r/hardlink-0").target.as_deref(), Some(Path::new("/r/file-0.bin")));
        assert_eq!(find("/r/fifo-0").target, None);
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
            assert_eq!(on_disk(temp.path()), planned(&spec, temp.path()), "threads={threads}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn create_specials() {
        let spec = special_spec().file_size(FileSize::Fixed(3));
        for threads in [1, 8] {
            let temp: TempDir = TempDir::new().unwrap();
            let counts: Counts = spec
                .create_with(temp.path(), &CreateOpts { threads, ..Default::default() })
                .unwrap();
            assert_eq!(counts, spec.counts(), "threads={threads}");
            assert_eq!(on_disk(temp.path()), planned(&spec, temp.path()), "threads={threads}");
        }
        // a hardlink shares its file's inode
        let temp: TempDir = TempDir::new().unwrap();
        spec.create(temp.path()).unwrap();
        let ino = |p: &str| fs::metadata(temp.path().join(p)).unwrap().ino();
        assert_eq!(ino("hardlink-0"), ino("file-0.bin"));
    }

    #[cfg(unix)]
    #[test]
    fn socket_beyond_sun_path() {
        // two 60-byte directory names push the socket path past sockaddr_un
        let spec = TreeSpec::new()
            .level(1, 0)
            .level(1, 0)
            .with(Special::Socket, 1)
            .dir_names(|idx| format!("{}{}", "d".repeat(59), idx.len()));
        let temp: TempDir = TempDir::new().unwrap();
        spec.create(temp.path()).unwrap();
        let socket: PathBuf = spec.plan(temp.path()).last().unwrap().path;
        assert!(socket.as_os_str().len() > SUN_PATH_MAX);
        assert!(fs::symlink_metadata(&socket).unwrap().file_type().is_socket());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_names() {
        let spec = TreeSpec::new()
            .level(2, 2)
            .file_names(|idx| OsString::from_vec(vec![b'f', 0xff, b'0' + *idx.last().unwrap() as u8]));
        let temp: TempDir = TempDir::new().unwrap();
        spec.create(temp.path()).unwrap();
        assert_eq!(on_disk(temp.path()), planned(&spec, temp.path()));
        assert!(on_disk(temp.path()).iter().any(|e| e.path.to_str().is_none()));
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
    fn create_refuses_invalid_specs() {
        let temp: TempDir = TempDir::new().unwrap();
        fs::write(temp.path().join("x"), b"").unwrap();
        let err = spec().create(temp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);

        // a link to the first file of a directory without files
        let temp: TempDir = TempDir::new().unwrap();
        let spec = TreeSpec::new().level(2, 0).with(Special::Hardlink, 1);
        let err = spec.create(temp.path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("hardlink"), "{err}");
        assert!(fs::read_dir(temp.path()).unwrap().next().is_none(), "created despite the error");
    }

    #[test]
    fn create_stops_on_error() {
        // a name collision: two dirs named alike, the second create fails
        let spec = TreeSpec::new().level(2, 0).level(1, 1).dir_names(|_| "same");
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
