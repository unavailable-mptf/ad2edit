//! The gallery: every dupe the editor knows about, what it remembers about
//! each (a title, a star, archived or not, when it was last opened) and what
//! it keeps for each in its own data folder: the file as it was first seen,
//! the versions each save replaced, and a thumbnail. The original is never
//! overwritten, so whatever happens to a dupe in the editor, the file as it
//! arrived can be brought back. Nothing here writes inside the AdvDupe2
//! folder except `restore` and `duplicate`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::dupe::Dupe;
use crate::dupefile;

pub const INDEX_FILE: &str = "gallery.toml";
/// Saved versions kept per dupe, newest first; the original is on top of these.
pub const VERSIONS_KEPT: usize = 30;

/// What is in a dupe, read once and remembered until the file changes.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Summary {
    pub entities: usize,
    pub constraints: usize,
    pub acf_parts: usize,
    pub chips: usize,
    /// The file's modified time when this was read; a different one means read again.
    pub modified: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Entry {
    /// Shown instead of the file name when not empty. The file keeps its name.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    pub starred: bool,
    pub archived: bool,
    /// Unix seconds; 0 is never.
    pub opened: u64,
    pub times_opened: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(default)]
pub struct Gallery {
    pub dupes: BTreeMap<String, Entry>,
}

/// One dupe as the gallery shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub path: PathBuf,
    pub title: String,
    /// The folder it sits in, relative to the scanned root; empty at the root.
    pub folder: String,
    pub entry: Entry,
    pub modified: u64,
    pub bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SortBy {
    #[default]
    LastOpened,
    Modified,
    Title,
    Size,
}

impl SortBy {
    pub const ALL: [SortBy; 4] = [SortBy::LastOpened, SortBy::Modified, SortBy::Title, SortBy::Size];

    pub fn name(self) -> &'static str {
        match self {
            SortBy::LastOpened => "last opened",
            SortBy::Modified => "last changed",
            SortBy::Title => "title",
            SortBy::Size => "size",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub path: PathBuf,
    /// Unix seconds it was put aside; for the original, when it was first seen.
    pub kept: u64,
    pub bytes: u64,
    pub original: bool,
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|since| since.as_secs()).unwrap_or(0)
}

fn modified_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok()).map(|since| since.as_secs()).unwrap_or(0)
}

/// The name a file goes by in the index: one spelling per file, so the same
/// dupe reached through a different case or slash is the same entry.
pub fn key_of(file: &Path) -> String {
    let text = file.to_string_lossy().replace('\\', "/");
    if cfg!(windows) { text.to_lowercase() } else { text }
}

fn hash_of(text: &str) -> u64 {
    text.bytes().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x100000001b3))
}

/// Where everything kept for one dupe lives: `dupes/<stem>-<hash of its path>`.
pub fn folder_of(data: &Path, file: &Path) -> PathBuf {
    let stem: String = file
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    data.join("dupes").join(format!("{stem}-{:08x}", hash_of(&key_of(file)) as u32))
}

pub fn thumbnail_path(data: &Path, file: &Path) -> PathBuf {
    folder_of(data, file).join("thumbnail.png")
}

/// Whether the thumbnail is missing or older than the dupe.
pub fn thumbnail_is_stale(data: &Path, file: &Path) -> bool {
    match (std::fs::metadata(thumbnail_path(data, file)), std::fs::metadata(file)) {
        (Ok(thumbnail), Ok(dupe)) => modified_of(&thumbnail) < modified_of(&dupe),
        _ => true,
    }
}

impl Gallery {
    /// The index in `data`, or an empty one. A file that does not parse is
    /// moved aside rather than overwritten.
    pub fn load(data: &Path) -> Gallery {
        let path = data.join(INDEX_FILE);
        let Ok(text) = std::fs::read_to_string(&path) else { return Gallery::default() };
        match toml::from_str(&text) {
            Ok(gallery) => gallery,
            Err(_) => {
                let _ = std::fs::rename(&path, data.join(format!("{INDEX_FILE}.broken")));
                Gallery::default()
            }
        }
    }

    pub fn save(&self, data: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
        let text = toml::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(data.join(INDEX_FILE), text).map_err(|e| format!("{INDEX_FILE}: {e}"))
    }

    pub fn entry(&self, file: &Path) -> Entry {
        self.dupes.get(&key_of(file)).cloned().unwrap_or_default()
    }

    pub fn entry_mut(&mut self, file: &Path) -> &mut Entry {
        self.dupes.entry(key_of(file)).or_default()
    }

    pub fn mark_opened(&mut self, file: &Path, at: u64) {
        let entry = self.entry_mut(file);
        entry.opened = at;
        entry.times_opened += 1;
    }

    /// The files as cards. `roots` are the folders that were scanned, so a
    /// card can say which subfolder it is in.
    pub fn cards(&self, files: &[PathBuf], roots: &[PathBuf]) -> Vec<Card> {
        files
            .iter()
            .filter_map(|file| {
                let meta = std::fs::metadata(file).ok()?;
                let entry = self.entry(file);
                let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = roots
                    .iter()
                    .find_map(|root| file.parent()?.strip_prefix(root).ok())
                    .map(|inside| inside.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|| file.parent().map(|parent| parent.display().to_string()).unwrap_or_default());
                Some(Card {
                    path: file.clone(),
                    title: if entry.title.is_empty() { stem } else { entry.title.clone() },
                    folder,
                    entry,
                    modified: modified_of(&meta),
                    bytes: meta.len(),
                })
            })
            .collect()
    }
}

/// Every `.txt` under the folders, deepest last, leaving out the editor's
/// own autosaves. Whether each really is a dupe is found out on opening.
pub fn scan(folders: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue: Vec<PathBuf> = folders.to_vec();
    while let Some(folder) = queue.pop() {
        let Ok(listing) = std::fs::read_dir(&folder) else { continue };
        for item in listing.flatten() {
            let path = item.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            let name = item.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".txt") && !name.ends_with(".autosave.txt") {
                found.push(path);
            }
        }
    }
    found.sort();
    found.dedup_by_key(|path| key_of(path));
    found
}

/// Starred cards first, then by `by`; the usual direction for each (newest,
/// largest, A to Z), the other way round when `reversed`.
pub fn sort(cards: &mut [Card], by: SortBy, reversed: bool) {
    cards.sort_by(|a, b| {
        let order = match by {
            SortBy::LastOpened => b.entry.opened.cmp(&a.entry.opened).then(b.modified.cmp(&a.modified)),
            SortBy::Modified => b.modified.cmp(&a.modified),
            SortBy::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            SortBy::Size => b.bytes.cmp(&a.bytes),
        };
        let order = if reversed { order.reverse() } else { order };
        b.entry.starred.cmp(&a.entry.starred).then(order).then_with(|| a.path.cmp(&b.path))
    });
}

/// Whether a card is shown for what was typed in the search box: every word
/// must be somewhere in its title, file name, folder or note.
pub fn matches(card: &Card, query: &str) -> bool {
    let hay = format!(
        "{} {} {} {}",
        card.title,
        card.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
        card.folder,
        card.entry.note
    )
    .to_lowercase();
    query.to_lowercase().split_whitespace().all(|word| hay.contains(word))
}

/// "just now", "3 hours ago", "2 days ago", then the age in weeks, months
/// and years; "never" for 0.
pub fn ago(then: u64, now: u64) -> String {
    if then == 0 {
        return "never".to_owned();
    }
    let seconds = now.saturating_sub(then);
    let said = |count: u64, unit: &str| format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" });
    match seconds {
        0..=89 => "just now".to_owned(),
        90..=3569 => said((seconds + 30) / 60, "minute"),
        3570..=84_599 => said((seconds + 1800) / 3600, "hour"),
        84_600..=1_209_599 => said((seconds + 43_200) / 86_400, "day"),
        1_209_600..=5_183_999 => said(seconds / 604_800, "week"),
        5_184_000..=63_071_999 => said(seconds / 2_592_000, "month"),
        _ => said(seconds / 31_536_000, "year"),
    }
}

fn constraint_count(dupe: &Dupe) -> usize {
    let constraints = dupe.root_table().ok().and_then(|root| dupe.get_table(root, "Constraints"));
    match constraints.map(|table| &dupe.arena[table]) {
        Some(crate::value::Node::Array(items)) => items.len(),
        _ => 0,
    }
}

/// A dupe file's bytes decoded.
pub fn read_dupe(bytes: &[u8]) -> Result<Dupe, String> {
    let file = dupefile::parse(bytes)?;
    let raw = dupefile::decompress(&file.compressed).map_err(|e| format!("decompression failed: {e}"))?;
    Ok(Dupe::from_body(&raw)?.0)
}

fn summary_of(dupe: &Dupe, modified: u64) -> Summary {
    let entities = dupe.list_entities();
    Summary {
        entities: entities.len(),
        constraints: constraint_count(dupe),
        acf_parts: entities.iter().filter(|(_, class, _)| class.trim_matches('"').starts_with("acf_")).count(),
        chips: entities.iter().filter(|(_, class, _)| class.trim_matches('"') == "gmod_wire_expression2").count(),
        modified,
    }
}

/// What is in a dupe file's bytes.
pub fn summarise(bytes: &[u8], modified: u64) -> Result<Summary, String> {
    Ok(summary_of(&read_dupe(bytes)?, modified))
}

pub const THUMBNAIL_SIZE: (usize, usize) = (320, 200);

/// Draws the dupe's thumbnail into its folder in `data` and says what is
/// in it. An empty dupe gets a summary and no picture.
pub fn render_thumbnail(lib: &mut crate::models::ModelLibrary, data: &Path, file: &Path) -> Result<Summary, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let modified = std::fs::metadata(file).map(|meta| modified_of(&meta)).unwrap_or(0);
    let dupe = read_dupe(&bytes)?;
    let summary = summary_of(&dupe, modified);
    if summary.entities > 0 {
        let (picture, _) = crate::render::picture(&dupe, lib, THUMBNAIL_SIZE.0, THUMBNAIL_SIZE.1, -35.0, 25.0, 1.4);
        let target = thumbnail_path(data, file);
        if let Some(folder) = target.parent() {
            std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        }
        std::fs::write(&target, crate::render::png(&picture)).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    Ok(summary)
}

/// Puts the file as it is now aside before it is overwritten: as the
/// original the first time, as a version every time its content is new.
/// Returns where the version went, or None when there was nothing to keep.
pub fn keep(data: &Path, file: &Path, at: u64) -> Result<Option<PathBuf>, String> {
    let Ok(current) = std::fs::read(file) else { return Ok(None) };
    let folder = folder_of(data, file);
    let versions = folder.join("versions");
    std::fs::create_dir_all(&versions).map_err(|e| format!("{}: {e}", versions.display()))?;
    let original = folder.join("original.txt");
    if !original.exists() {
        std::fs::write(&original, &current).map_err(|e| format!("{}: {e}", original.display()))?;
        std::fs::write(folder.join("path.txt"), file.display().to_string()).map_err(|e| e.to_string())?;
        return Ok(Some(original));
    }
    let kept = self::versions(data, file);
    if kept.first().is_some_and(|newest| std::fs::read(&newest.path).is_ok_and(|bytes| bytes == current)) {
        return Ok(None);
    }
    let mut stamp = at;
    while versions.join(format!("{stamp}.txt")).exists() {
        stamp += 1;
    }
    let target = versions.join(format!("{stamp}.txt"));
    std::fs::write(&target, &current).map_err(|e| format!("{}: {e}", target.display()))?;
    for old in kept.iter().filter(|version| !version.original).skip(VERSIONS_KEPT - 1) {
        let _ = std::fs::remove_file(&old.path);
    }
    Ok(Some(target))
}

/// What is kept for a dupe, newest first, the original last.
pub fn versions(data: &Path, file: &Path) -> Vec<Version> {
    let folder = folder_of(data, file);
    let mut found: Vec<Version> = std::fs::read_dir(folder.join("versions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|item| {
            let path = item.path();
            let kept = path.file_stem()?.to_string_lossy().parse().ok()?;
            Some(Version { bytes: item.metadata().ok()?.len(), path, kept, original: false })
        })
        .collect();
    found.sort_by(|a, b| b.kept.cmp(&a.kept));
    let original = folder.join("original.txt");
    if let Ok(meta) = std::fs::metadata(&original) {
        found.push(Version { path: original, kept: modified_of(&meta), bytes: meta.len(), original: true });
    }
    found
}

/// Brings a kept version back as the dupe, putting what it replaces aside
/// first, so a restore can itself be undone.
pub fn restore(data: &Path, file: &Path, version: &Path, at: u64) -> Result<(), String> {
    let bytes = std::fs::read(version).map_err(|e| format!("{}: {e}", version.display()))?;
    keep(data, file, at)?;
    std::fs::write(file, bytes).map_err(|e| format!("{}: {e}", file.display()))
}

/// A copy beside the file, named "<stem> copy.txt", "<stem> copy 2.txt", ...
pub fn duplicate(file: &Path) -> Result<PathBuf, String> {
    let stem = file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "dupe".to_owned());
    let target = (1..1000)
        .map(|n| file.with_file_name(if n == 1 { format!("{stem} copy.txt") } else { format!("{stem} copy {n}.txt") }))
        .find(|candidate| !candidate.exists())
        .ok_or("too many copies already")?;
    std::fs::copy(file, &target).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("ad2read-gallery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn the_original_is_kept_once_and_every_new_content_after_it_is_a_version() {
        let root = scratch("keep");
        let (data, file) = (root.join("data"), root.join("tank.txt"));
        assert_eq!(keep(&data, &file, 100).unwrap(), None, "nothing on disk yet, nothing to keep");

        std::fs::write(&file, b"as it arrived").unwrap();
        let first = keep(&data, &file, 100).unwrap().unwrap();
        assert!(first.ends_with("original.txt"));
        std::fs::write(&file, b"first edit").unwrap();
        assert!(keep(&data, &file, 200).unwrap().unwrap().ends_with("200.txt"));
        assert_eq!(keep(&data, &file, 300).unwrap(), None, "the same content is not kept twice");
        std::fs::write(&file, b"second edit").unwrap();
        keep(&data, &file, 200).unwrap();

        let kept = versions(&data, &file);
        assert_eq!(kept.len(), 3);
        assert_eq!((kept[0].kept, kept[1].kept), (201, 200), "a taken second moves on rather than overwrite");
        assert!(kept[2].original);
        assert_eq!(std::fs::read(&kept[2].path).unwrap(), b"as it arrived");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_restore_brings_a_version_back_and_can_itself_be_undone() {
        let root = scratch("restore");
        let (data, file) = (root.join("data"), root.join("tank.txt"));
        std::fs::write(&file, b"original").unwrap();
        keep(&data, &file, 10).unwrap();
        std::fs::write(&file, b"ruined").unwrap();

        let original = versions(&data, &file).pop().unwrap();
        restore(&data, &file, &original.path, 20).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"original");
        let kept = versions(&data, &file);
        assert_eq!(std::fs::read(&kept[0].path).unwrap(), b"ruined", "what the restore replaced was put aside");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn only_the_newest_versions_are_kept_and_never_at_the_originals_cost() {
        let root = scratch("prune");
        let (data, file) = (root.join("data"), root.join("tank.txt"));
        for round in 0..(VERSIONS_KEPT as u64 + 6) {
            std::fs::write(&file, format!("content {round}")).unwrap();
            keep(&data, &file, 1000 + round).unwrap();
        }
        let kept = versions(&data, &file);
        assert_eq!(kept.len(), VERSIONS_KEPT + 1);
        assert!(kept.last().unwrap().original);
        assert_eq!(std::fs::read(&kept.last().unwrap().path).unwrap(), b"content 0");
        assert_eq!(kept[0].kept, 1000 + VERSIONS_KEPT as u64 + 5);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_dupes_with_one_name_in_different_folders_do_not_share_a_folder() {
        let data = Path::new("data");
        let a = folder_of(data, Path::new("C:/dupes/tank.txt"));
        let b = folder_of(data, Path::new("C:/dupes/old/tank.txt"));
        assert_ne!(a, b);
        assert_eq!(a, folder_of(data, Path::new("C:\\dupes\\tank.txt")), "one spelling per file");
        let odd = folder_of(data, Path::new("C:/dupes/p62 recon & airperf.txt"));
        assert!(odd.file_name().unwrap().to_string_lossy().starts_with("p62_recon___airperf-"));
    }

    #[test]
    fn the_index_round_trips_and_a_broken_one_is_moved_aside() {
        let root = scratch("index");
        let mut gallery = Gallery::default();
        let file = root.join("my tank.txt");
        gallery.mark_opened(&file, 1_790_000_000);
        gallery.mark_opened(&file, 1_790_000_500);
        gallery.entry_mut(&file).title = "The good one".to_owned();
        gallery.entry_mut(&file).starred = true;
        gallery.entry_mut(&file).summary = Some(Summary { entities: 54, constraints: 12, acf_parts: 20, chips: 1, modified: 7 });
        gallery.save(&root).unwrap();
        let back = Gallery::load(&root);
        assert_eq!(back, gallery);
        assert_eq!(back.entry(&file).times_opened, 2);
        assert_eq!(back.entry(&file).opened, 1_790_000_500);

        std::fs::write(root.join(INDEX_FILE), "this is [not toml").unwrap();
        assert_eq!(Gallery::load(&root), Gallery::default());
        assert!(root.join(format!("{INDEX_FILE}.broken")).exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_scan_finds_dupes_in_subfolders_and_skips_autosaves() {
        let root = scratch("scan");
        std::fs::create_dir_all(root.join("tanks/old")).unwrap();
        for name in ["a.txt", "a.autosave.txt", "a.nodes.toml", "tanks/b.txt", "tanks/old/c.TXT"] {
            std::fs::write(root.join(name), b"x").unwrap();
        }
        let found = scan(&[root.clone()]);
        let names: Vec<String> = found.iter().map(|path| path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, ["a.txt", "tanks/b.txt", "tanks/old/c.TXT"]);

        let cards = Gallery::default().cards(&found, &[root.clone()]);
        assert_eq!(cards.iter().map(|card| card.folder.as_str()).collect::<Vec<_>>(), ["", "tanks", "tanks/old"]);
        assert_eq!(cards[1].title, "b");
        let _ = std::fs::remove_dir_all(root);
    }

    fn card(title: &str, opened: u64, modified: u64, bytes: u64, starred: bool) -> Card {
        Card {
            path: PathBuf::from(format!("{title}.txt")),
            title: title.to_owned(),
            folder: String::new(),
            entry: Entry { opened, starred, ..Entry::default() },
            modified,
            bytes,
        }
    }

    #[test]
    fn starred_cards_lead_whatever_the_sort() {
        let mut cards = vec![card("beta", 5, 50, 300, false), card("Alpha", 9, 10, 100, false), card("gamma", 1, 99, 200, true)];
        let titles = |cards: &[Card]| cards.iter().map(|card| card.title.clone()).collect::<Vec<_>>();
        sort(&mut cards, SortBy::LastOpened, false);
        assert_eq!(titles(&cards), ["gamma", "Alpha", "beta"]);
        sort(&mut cards, SortBy::Modified, false);
        assert_eq!(titles(&cards), ["gamma", "beta", "Alpha"]);
        sort(&mut cards, SortBy::Title, false);
        assert_eq!(titles(&cards), ["gamma", "Alpha", "beta"]);
        sort(&mut cards, SortBy::Title, true);
        assert_eq!(titles(&cards), ["gamma", "beta", "Alpha"]);
        sort(&mut cards, SortBy::Size, false);
        assert_eq!(titles(&cards), ["gamma", "beta", "Alpha"]);
    }

    #[test]
    fn a_search_needs_every_word_somewhere() {
        let mut found = card("Light tank", 0, 0, 0, false);
        found.folder = "clan/tanks".to_owned();
        found.entry.note = "the one that drives".to_owned();
        assert!(matches(&found, ""));
        assert!(matches(&found, "light CLAN"));
        assert!(matches(&found, "drives tank"));
        assert!(!matches(&found, "light drone"));
    }

    #[test]
    fn ages_read_the_way_people_say_them() {
        let now = 100_000_000;
        assert_eq!(ago(0, now), "never");
        assert_eq!(ago(now - 20, now), "just now");
        assert_eq!(ago(now - 60 * 5, now), "5 minutes ago");
        assert_eq!(ago(now - 3600, now), "1 hour ago");
        assert_eq!(ago(now - 3599, now), "1 hour ago");
        assert_eq!(ago(now - 3600 * 23, now), "23 hours ago");
        assert_eq!(ago(now - 3600 * 30, now), "1 day ago");
        assert_eq!(ago(now - 86_400 * 3, now), "3 days ago");
        assert_eq!(ago(now - 86_400 * 21, now), "3 weeks ago");
        assert_eq!(ago(now - 86_400 * 90, now), "3 months ago");
        assert_eq!(ago(now - 86_400 * 800, now), "2 years ago");
    }

    #[test]
    fn a_preset_is_summarised_from_its_file_bytes() {
        let dupe = crate::buildfile::preset_dupe("light").unwrap();
        let bytes = crate::buildfile::file_bytes(&dupe, "light").unwrap();
        let summary = summarise(&bytes, 42).unwrap();
        assert_eq!(summary.entities, dupe.list_entities().len());
        assert!(summary.acf_parts > 10 && summary.acf_parts < summary.entities);
        assert_eq!(summary.modified, 42);
        assert!(summarise(b"not a dupe", 0).is_err());
    }
}
