//! Finding a model's real size.
//!
//! Every entity in a dupe carries a model path and nothing else — no bounds,
//! no dimensions. So the viewport drew every prop as the same cube. This
//! resolves a path like `models/props_junk/sawblade001a.mdl` to the bounding
//! box in the model's own header.
//!
//! Only the `.mdl` is needed. `vmdl`'s `Model::from_path` wants `.vtx` and
//! `.vvd` alongside it because it builds meshes, but the bounding box lives in
//! the MDL header alone, so `Mdl::read` on those bytes is enough — one file
//! instead of three, and no extraction to disk.
//!
//! Three places a model can live, searched in the order GMod itself would:
//!   1. loose under `garrysmod/models/`
//!   2. inside a `.gma` addon, either in `garrysmod/addons` or the workshop cache
//!   3. inside a `.vpk`
//!
//! The GMA layout below is from Facepunch/gmad's own `AddonReader.h`, not from
//! memory.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Half-extents of a model, in Source units.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub min: (f64, f64, f64),
    pub max: (f64, f64, f64),
}

impl Bounds {
    pub fn centre(&self) -> (f64, f64, f64) {
        (
            (self.min.0 + self.max.0) * 0.5,
            (self.min.1 + self.max.1) * 0.5,
            (self.min.2 + self.max.2) * 0.5,
        )
    }
    pub fn half_extents(&self) -> (f64, f64, f64) {
        (
            (self.max.0 - self.min.0).abs() * 0.5,
            (self.max.1 - self.min.1).abs() * 0.5,
            (self.max.2 - self.min.2).abs() * 0.5,
        )
    }
}

/// A model's geometry, flattened to a triangle list in the model's own space.
/// Positions and normals only — no texture coordinates, because the renderer
/// shades by normal and there is nowhere to get materials from anyway.
#[derive(Debug, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Indices into `positions`, three per triangle.
    pub indices: Vec<u32>,
    /// (first index, index count, material) — one per material. A ragdoll
    /// has a body texture and a face texture and using one for the whole
    /// model is what put the wrong skin on everything.
    pub sections: Vec<(u32, u32, String)>,
    /// For each section, its slot in the model's material list, which is
    /// the number the SubMaterial tool's overrides are keyed by.
    pub section_slots: Vec<usize>,
}

impl MeshData {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

type BoneMatrix = [[f32; 4]; 3];

fn compose(a: &BoneMatrix, b: &BoneMatrix) -> BoneMatrix {
    let mut out = [[0.0f32; 4]; 3];
    for row in 0..3 {
        for col in 0..4 {
            out[row][col] = a[row][0] * b[0][col] + a[row][1] * b[1][col] + a[row][2] * b[2][col];
        }
        out[row][3] += a[row][3];
    }
    out
}

/// Each bone's skinning matrix for frame 0 of the model's first sequence:
/// the bone's pose in model space times its pose-to-bone matrix. A bone the
/// sequence says nothing raw about keeps its reference pose, for which the
/// product is the identity.
fn idle_bone_matrices(model: &vmdl::Model) -> Vec<BoneMatrix> {
    use vmdl::mdl::AnimationFlags;
    let idle = model.animations().next();
    let mut poses: Vec<BoneMatrix> = Vec::new();
    let mut matrices = Vec::new();
    for (index, bone) in model.bones().enumerate() {
        let track = idle.and_then(|idle| idle.animations.iter().find(|track| track.bone as usize == index));
        let additive = track.is_some_and(|track| track.flags.contains(AnimationFlags::STUDIO_ANIM_DELTA));
        let raw = |flag: AnimationFlags| track.filter(|track| !additive && track.flags.contains(flag));
        let q = raw(AnimationFlags::STUDIO_ANIM_RAWROT)
            .or_else(|| raw(AnimationFlags::STUDIO_ANIM_RAWROT2))
            .map(|track| track.rotation(0))
            .unwrap_or(bone.quaternion);
        let at = raw(AnimationFlags::STUDIO_ANIM_RAWPOS).map(|track| track.position(0)).unwrap_or(bone.pos);
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let local: BoneMatrix = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w), at.x],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w), at.y],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y), at.z],
        ];
        let pose = match usize::try_from(bone.parent).ok().and_then(|parent| poses.get(parent)) {
            Some(parent) => compose(parent, &local),
            None => local,
        };
        matrices.push(compose(&pose, &bone.pose_to_bone.rows()));
        poses.push(pose);
    }
    matrices
}

/// Finds the `garrysmod` folder without being told where it is.
///
/// The best hint is a dupe's own path — an AdvDupe2 file lives at
/// `garrysmod/data/advdupe2/whatever.txt`, so walking up from it lands on the
/// game folder exactly. Failing that, the usual Steam locations, then every
/// library listed in `libraryfolders.vdf`, since a big Steam install is very
/// often on a second drive.
pub fn find_garrysmod(hint: Option<&Path>) -> Option<PathBuf> {
    let looks_right = |p: &Path| {
        p.file_name().map(|n| n.eq_ignore_ascii_case("garrysmod")) == Some(true)
            && (p.join("addons").is_dir() || p.join("models").is_dir() || p.join("data").is_dir())
    };

    if let Some(h) = hint {
        for dir in h.ancestors() {
            if looks_right(dir) {
                return Some(dir.to_path_buf());
            }
        }
    }

    let mut steam_roots: Vec<PathBuf> = vec![
        PathBuf::from(r"C:\Program Files (x86)\Steam"),
        PathBuf::from(r"C:\Program Files\Steam"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        steam_roots.push(home.join(".steam/steam"));
        steam_roots.push(home.join(".local/share/Steam"));
        steam_roots.push(home.join("Library/Application Support/Steam"));
    }

    // libraryfolders.vdf lists every drive Steam installs to. It's KeyValues,
    // and the "path" entries are all that matter here.
    let mut libraries: Vec<PathBuf> = Vec::new();
    for root in &steam_roots {
        libraries.push(root.clone());
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        let Ok(text) = fs::read_to_string(&vdf) else {
            continue;
        };
        for line in text.lines() {
            let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
            if quoted.len() == 2 && quoted[0].eq_ignore_ascii_case("path") {
                libraries.push(PathBuf::from(quoted[1].replace("\\\\", "\\")));
            }
        }
    }

    for lib in libraries {
        let candidate = lib
            .join("steamapps")
            .join("common")
            .join("GarrysMod")
            .join("garrysmod");
        if looks_right(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// A decoded diffuse texture, ready to upload.
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Where a model was found, so the log can say something useful.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    Loose(PathBuf),
    Gma(PathBuf),
    Vpk(PathBuf),
    /// Made here from an entity's numbers, not read from a file.
    Generated,
}

pub struct ModelLibrary {
    roots: Vec<PathBuf>,
    /// Every .gma found, mapped to the model paths inside it. Built once,
    /// because a workshop cache can hold hundreds of archives and scanning
    /// them per lookup would be unusable.
    gma_index: HashMap<String, (PathBuf, GmaEntry)>,
    /// Parsed once. VPK::read walks the whole directory tree, and doing that
    /// per lookup was thousands of parses of garrysmod_dir.vpk for one dupe.
    vpks: Vec<(PathBuf, source_vpk::Vpk)>,
    cache: HashMap<String, Option<(Bounds, Source)>>,
    mesh_cache: HashMap<String, Option<std::sync::Arc<MeshData>>>,
    texture_cache: HashMap<String, Option<std::sync::Arc<Texture>>>,
    attachment_cache: HashMap<String, Option<(f64, f64, f64)>>,
    pub scanned_gmas: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct GmaEntry {
    /// Offset of the file block plus the running offset of this entry.
    offset: u64,
    size: u64,
}

fn normalise(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .replace('\\', "/")
        .to_lowercase()
}

/// Reads a null-terminated string, the way Bootil's ReadString does.
fn read_cstr(data: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    while *pos < data.len() && data[*pos] != 0 {
        *pos += 1;
    }
    if *pos >= data.len() {
        return None;
    }
    let s = String::from_utf8_lossy(&data[start..*pos]).into_owned();
    *pos += 1;
    Some(s)
}

fn read_u32(data: &[u8], pos: &mut usize) -> Option<u32> {
    let end = *pos + 4;
    let v = u32::from_le_bytes(data.get(*pos..end)?.try_into().ok()?);
    *pos = end;
    Some(v)
}

fn read_i64(data: &[u8], pos: &mut usize) -> Option<i64> {
    let end = *pos + 8;
    let v = i64::from_le_bytes(data.get(*pos..end)?.try_into().ok()?);
    *pos = end;
    Some(v)
}

/// Parses a GMA's file index. Layout from Facepunch/gmad AddonReader.h:
/// "GMAD", version byte, u64 steamid, u64 timestamp, then (version > 1) a
/// null-terminated list of required content, then name/description/author
/// strings, an i32 addon version, then entries of
/// (u32 filenumber, string name, i64 size, u32 crc) until filenumber == 0.
fn index_gma(data: &[u8]) -> Option<Vec<(String, GmaEntry)>> {
    if data.len() < 5 || &data[0..4] != b"GMAD" {
        return None;
    }
    let version = data[4];
    if version > 3 {
        return None;
    }
    let mut pos = 5 + 8 + 8; // version byte, steamid, timestamp

    if version > 1 {
        // Required content: strings until an empty one.
        loop {
            let s = read_cstr(data, &mut pos)?;
            if s.is_empty() {
                break;
            }
        }
    }
    read_cstr(data, &mut pos)?; // name
    read_cstr(data, &mut pos)?; // description
    read_cstr(data, &mut pos)?; // author
    pos += 4; // addon version, unused

    let mut entries = Vec::new();
    let mut running = 0u64;
    loop {
        let number = read_u32(data, &mut pos)?;
        if number == 0 {
            break;
        }
        let name = read_cstr(data, &mut pos)?;
        let size = read_i64(data, &mut pos)?;
        pos += 4; // crc
        entries.push((
            normalise(&name),
            GmaEntry {
                offset: running,
                size: size.max(0) as u64,
            },
        ));
        running += size.max(0) as u64;
    }

    // Offsets so far are relative to the start of the file block, which begins
    // right after the index.
    let block = pos as u64;
    Some(
        entries
            .into_iter()
            .map(|(n, e)| {
                (
                    n,
                    GmaEntry {
                        offset: block + e.offset,
                        size: e.size,
                    },
                )
            })
            .collect(),
    )
}

/// Extra game paths from `cfg/mount.cfg`, which is a KeyValues file of
/// `"game" "path"` pairs. Only the second string of each pair is a path.
fn read_mount_cfg(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        let quoted: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        if quoted.len() == 2 {
            let p = PathBuf::from(quoted[1]);
            if p.exists() {
                out.push(p);
            }
        }
    }
    out
}

/// Pulls `$basetexture` out of a VMT. A VMT is KeyValues text, and the value
/// may or may not be quoted, so this looks for the key and takes the rest of
/// the line.
/// A `patch` VMT wraps another: `patch { include "materials/x.vmt" ... }`.
/// Returns the included path if this is one.
/// The include path of a `patch` VMT. VBSP's cubemap patches write the key
/// quoted — `"include" "materials/x.vmt"` — so what follows the keyword is
/// the key's closing quote, then whitespace, then the value. Reading the
/// value as "up to the next quote" from there returned the whitespace and
/// every cubemap-patched map material fell through to "no $basetexture".
pub fn include_from_vmt(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find("include")?;
    let after = &text[at + "include".len()..];
    // Step past the key's closing quote (if the key was quoted), then any
    // whitespace, then read a quoted or bare value.
    let value = after.trim_start_matches('"').trim_start();
    let value = if let Some(rest) = value.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else {
        value.split_whitespace().next().unwrap_or("")
    };
    let value = value.trim().trim_matches('"');
    if value.is_empty() {
        return None;
    }
    Some(value.replace('\\', "/").to_lowercase())
}

/// A VMT texture value as a lookup name: forward slashes, lowercase, no
/// leading "materials/", no trailing ".vtf".
pub fn texture_name(value: &str) -> String {
    let n = normalise(value);
    let n = n.trim_start_matches("materials/").to_owned();
    n.strip_suffix(".vtf").map(|x| x.to_owned()).unwrap_or(n)
}

/// `$basetexture2` — the second layer of a WorldVertexTransition.
pub fn basetexture2_from_vmt(text: &str) -> Option<String> {
    vmt_value(text, "$basetexture2")
}

/// Whether the shader is Water (or the VMT looks like water).
pub fn is_water_vmt(text: &str) -> bool {
    let lower = text.to_lowercase();
    let head: String = lower.chars().take(64).collect();
    head.trim_start().trim_start_matches('"').starts_with("water")
        || lower.contains("$fogcolor")
        || lower.contains("$refracttexture")
        || lower.contains("$bottommaterial")
}

/// `$fogcolor "{ 12 33 46 }"` (0-255) or `"[0.05 0.13 0.18]"` (0-1).
pub fn fogcolor_from_vmt(text: &str) -> Option<(u8, u8, u8)> {
    let v = vmt_value(text, "$fogcolor")?;
    let nums: Vec<f64> = v
        .trim_matches(|c| c == '{' || c == '}' || c == '[' || c == ']' || c == '"' || c == ' ')
        .split_whitespace()
        .filter_map(|x| x.parse::<f64>().ok())
        .collect();
    if nums.len() < 3 {
        return None;
    }
    let scale = if v.contains('[') { 255.0 } else { 1.0 };
    Some((
        (nums[0] * scale).clamp(0.0, 255.0) as u8,
        (nums[1] * scale).clamp(0.0, 255.0) as u8,
        (nums[2] * scale).clamp(0.0, 255.0) as u8,
    ))
}

/// A 4×4 solid colour, for materials that have no picture.
pub fn solid_texture(r: u8, g: u8, b: u8) -> Texture {
    Texture {
        width: 4,
        height: 4,
        rgba: [r, g, b, 255].repeat(16),
    }
}

/// The value of a VMT key, quoted or bare, first occurrence. Braced values
/// like `{ 12 33 46 }` come back whole.
pub fn vmt_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        let Some(at) = lower.find(key) else { continue };
        let after = &line[at + key.len()..];
        // Exact key: the next char must not extend the name ($basetexture2).
        if after.chars().next().map(|c| c.is_alphanumeric()).unwrap_or(false) {
            continue;
        }
        let value = after.trim_start_matches('"').trim_start();
        let owned: String = if let Some(rest) = value.strip_prefix('"') {
            rest.split('"').next().unwrap_or("").to_owned()
        } else if value.starts_with('{') || value.starts_with('[') {
            let close = if value.starts_with('{') { '}' } else { ']' };
            value.split(close).next().map(|x| format!("{x}{close}")).unwrap_or_default()
        } else {
            value.split_whitespace().next().unwrap_or("").to_owned()
        };
        let owned = owned.trim().to_owned();
        if !owned.is_empty() {
            return Some(owned.replace('\\', "/"));
        }
    }
    None
}

pub fn basetexture_from_vmt(text: &str) -> Option<String> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        let Some(at) = lower.find("$basetexture") else {
            continue;
        };
        // Skip `$basetexture2`, which is a blend layer rather than the base.
        let after = &line[at + "$basetexture".len()..];
        if after.starts_with('2') {
            continue;
        }
        // The key is usually quoted, so what follows it is that closing
        // quote, not the value. Step past it before reading anything.
        let value = after.trim_start_matches('"').trim_start();
        let value = if let Some(rest) = value.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else {
            value.split_whitespace().next().unwrap_or("")
        };
        let value = value.trim().trim_matches('"');
        if !value.is_empty() {
            return Some(value.replace('\\', "/"));
        }
    }
    None
}

pub fn decode_vtf(bytes: &[u8]) -> Option<Texture> {
    let vtf = vtf::from_bytes(bytes).ok()?;
    let image = vtf.highres_image.decode(0).ok()?;
    let rgba = image.to_rgba8();
    Some(Texture {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Parses a GMA's index by reading only as much of the file as the index
/// needs. Starts at 1 MB, which covers almost every addon, and grows if the
/// index turns out to be longer.
fn read_gma_index(path: &Path) -> Option<Vec<(String, GmaEntry)>> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let total = file.metadata().ok()?.len() as usize;
    // Magic first, so a file that isn't a GMA is rejected on four bytes
    // rather than after growing the buffer to the whole thing.
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).ok()?;
    if &magic != b"GMAD" {
        return None;
    }
    {
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0)).ok()?;
    }
    let mut want = (1 << 20).min(total);
    loop {
        let mut buf = vec![0u8; want];
        file.read_exact(&mut buf).ok()?;
        if let Some(index) = index_gma(&buf) {
            return Some(index);
        }
        if want >= total {
            return None;
        }
        // Index ran past the buffer: read more from the start. Rare enough
        // that re-reading the prefix is fine.
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(0)).ok()?;
        want = (want * 8).min(total);
    }
}

/// Reads one file out of a GMA by seeking straight to it.
fn read_gma_entry(path: &Path, entry: &GmaEntry) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(entry.offset)).ok()?;
    let mut buf = vec![0u8; entry.size as usize];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn gather(dir: &Path, ext: &str, out: &mut Vec<PathBuf>, depth: usize) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            gather(&p, ext, out, depth - 1);
        } else if p.extension().map(|x| x.eq_ignore_ascii_case(ext)) == Some(true) {
            out.push(p);
        }
    }
}

impl ModelLibrary {
    /// `garrysmod` is the game folder — the one containing `addons` and
    /// `models`. Everything else is found relative to it.
    pub fn new(garrysmod: &Path) -> ModelLibrary {
        let mut roots = vec![garrysmod.to_path_buf()];

        // The HL2, episode and CS:S content that ships with Garry's Mod does
        // NOT live under garrysmod/ — it sits in sibling folders like
        // sourceengine/ and hl2/ one level up. Searching only downward from
        // garrysmod/ finds none of it, which is why every stock prop came
        // back missing.
        let game_root = garrysmod.parent().map(|p| p.to_path_buf());
        if let Some(g) = game_root.as_ref() {
            roots.push(g.clone());
        }

        // steamapps/common/GarrysMod/garrysmod -> steamapps/workshop/content/4000
        let workshop = garrysmod
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .map(|steamapps| steamapps.join("workshop").join("content").join("4000"));

        // cfg/mount.cfg is how GMod itself picks up games installed elsewhere.
        for extra in read_mount_cfg(&garrysmod.join("cfg").join("mount.cfg")) {
            roots.push(extra);
        }

        let mut gmas = Vec::new();
        gather(&garrysmod.join("addons"), "gma", &mut gmas, 3);
        if let Some(w) = workshop.as_ref() {
            gather(w, "gma", &mut gmas, 3);
            roots.push(w.clone());
        }

        let mut gma_index = HashMap::new();
        let scanned = gmas.len();
        for gma in &gmas {
            // The index sits at the front of the file. A workshop GMA can be
            // hundreds of megabytes; reading all of it to parse a few hundred
            // kilobytes of index was most of a two-minute load.
            let Some(entries) = read_gma_index(gma) else {
                continue;
            };
            for (name, entry) in entries {
                // Everything a model needs, not just the .mdl. Bounds come
                // from the header alone, but a mesh also wants .dx90.vtx and
                // .vvd, and a texture wants .vmt and .vtf. Indexing only .mdl
                // is why addon models resolved a size and never rendered.
                let useful = name.ends_with(".mdl")
                    || name.ends_with(".vtx")
                    || name.ends_with(".vvd")
                    || name.ends_with(".vmt")
                    || name.ends_with(".vtf");
                if useful {
                    gma_index.entry(name).or_insert((gma.clone(), entry));
                }
            }
        }

        // Collect VPKs from the whole game root, not just garrysmod/.
        let mut raw = Vec::new();
        if let Some(g) = game_root.as_ref() {
            gather(g, "vpk", &mut raw, 3);
        } else {
            gather(garrysmod, "vpk", &mut raw, 3);
        }
        for extra in roots.iter().skip(1) {
            gather(extra, "vpk", &mut raw, 3);
        }

        // A split archive is `name_dir.vpk` plus `name_000.vpk`, `name_001.vpk`
        // and so on. Only the _dir one carries the index; the crate resolves
        // the numbered parts itself, and opening one directly finds nothing.
        let mut vpk_paths: Vec<PathBuf> = Vec::new();
        for p in raw {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let numbered = stem.len() > 4
                && stem.as_bytes()[stem.len() - 4] == b'_'
                && stem[stem.len() - 3..].chars().all(|c| c.is_ascii_digit());
            if numbered {
                continue;
            }
            if !vpk_paths.contains(&p) {
                vpk_paths.push(p);
            }
        }
        let vpks: Vec<(PathBuf, source_vpk::Vpk)> = vpk_paths
            .into_iter()
            .filter_map(|p| source_vpk::Vpk::open(&p).ok().map(|v| (p, v)))
            .collect();

        ModelLibrary {
            roots,
            gma_index,
            vpks,
            cache: HashMap::new(),
            mesh_cache: HashMap::new(),
            texture_cache: HashMap::new(),
            attachment_cache: HashMap::new(),
            scanned_gmas: scanned,
        }
    }

    /// Everything the lookup will search, so a miss can be explained rather
    /// than just reported.
    pub fn describe_search(&self) -> String {
        let roots: Vec<String> = self.roots.iter().map(|p| p.display().to_string()).collect();
        format!(
            "roots: {}\nvpk archives: {}\ngma archives: {} ({} models indexed)",
            roots.join(", "),
            self.vpks.len(),
            self.scanned_gmas,
            self.indexed_models()
        )
    }

    pub fn archives(&self) -> usize {
        self.vpks.len()
    }

    pub fn indexed_models(&self) -> usize {
        self.gma_index.keys().filter(|k| k.ends_with(".mdl")).count()
    }

    /// Every file indexed, models and their companions alike.
    pub fn indexed_files(&self) -> usize {
        self.gma_index.len()
    }

    /// Bounding box for a model path, or None if it can't be found or read.
    /// Takes in a mesh that was generated rather than read (a Primitive
    /// shape), under a name of its own, so bounds, drawing and uploading
    /// treat it like any model. Taking the same name in twice is free.
    pub fn adopt(&mut self, name: &str, mesh: MeshData) {
        let key = normalise(name);
        if self.mesh_cache.contains_key(&key) {
            return;
        }
        let (mut min, mut max) = ((f64::MAX, f64::MAX, f64::MAX), (f64::MIN, f64::MIN, f64::MIN));
        for p in &mesh.positions {
            min = (min.0.min(p[0] as f64), min.1.min(p[1] as f64), min.2.min(p[2] as f64));
            max = (max.0.max(p[0] as f64), max.1.max(p[1] as f64), max.2.max(p[2] as f64));
        }
        if !mesh.positions.is_empty() {
            self.cache.insert(key.clone(), Some((Bounds { min, max }, Source::Generated)));
        }
        self.mesh_cache.insert(key, Some(std::sync::Arc::new(mesh)));
    }

    /// The model an entity is drawn with: its own, or for a Primitive shape
    /// the generated one, taken in on the way.
    pub fn model_of(&mut self, dupe: &crate::dupe::Dupe, index: f64, model: &str) -> String {
        match crate::primitive::of_entity(dupe, index) {
            Some((name, mesh)) => {
                self.adopt(&name, mesh);
                name
            }
            None => model.to_owned(),
        }
    }

    pub fn bounds(&mut self, model: &str) -> Option<(Bounds, Source)> {
        let key = normalise(model);
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let found = self.locate(&key);
        self.cache.insert(key, found.clone());
        found
    }

    /// Raw bytes of any file, from wherever it lives. Meshes need three files
    /// per model, so this is worth having separately from the bounds lookup.
    pub fn read_file(&self, key: &str) -> Option<(Vec<u8>, Source)> {
        let key = normalise(key);

        for root in &self.roots {
            let p = root.join(&key);
            if let Ok(data) = fs::read(&p) {
                return Some((data, Source::Loose(p)));
            }
        }

        if let Some((archive, entry)) = self.gma_index.get(&key) {
            if let Some(bytes) = read_gma_entry(archive, entry) {
                return Some((bytes, Source::Gma(archive.clone())));
            }
        }

        for (pak, vpk) in &self.vpks {
            if !vpk.contains(&key) {
                continue;
            }
            // read() returns preload + archive bytes, seeking, bounds-checked.
            let Ok(data) = vpk.read(&key) else { continue };
            return Some((data, Source::Vpk(pak.clone())));
        }

        None
    }

    /// Full geometry for a model, or None if any of the three files is missing
    /// or the model won't parse. Cached, because a build can use the same prop
    /// forty times.
    pub fn mesh(&mut self, model: &str) -> Option<std::sync::Arc<MeshData>> {
        self.mesh_as(model, 0, &[])
    }

    /// The mesh as one entity wears it: its skin, and for each body part
    /// the bodygroup it chose. An entity that chose nothing gets the same
    /// cached mesh as every other.
    pub fn mesh_as(&mut self, model: &str, skin: usize, bodygroups: &[usize]) -> Option<std::sync::Arc<MeshData>> {
        let model = normalise(model);
        let last_chosen = bodygroups.iter().rposition(|chosen| *chosen != 0).map_or(0, |at| at + 1);
        let bodygroups = &bodygroups[..last_chosen];
        let key = if skin == 0 && bodygroups.is_empty() { model.clone() } else { format!("{model}#{skin}#{bodygroups:?}") };
        if let Some(hit) = self.mesh_cache.get(&key) {
            return hit.clone();
        }
        let built = self.build_mesh(&model, skin, bodygroups).map(std::sync::Arc::new);
        self.mesh_cache.insert(key, built.clone());
        built
    }

    /// The first material a model references, decoded to RGBA.
    ///
    /// A model can have many materials; the viewport shades whole props, so
    /// the first one is a fair stand-in and avoids splitting every mesh into
    /// per-material draws for a tool that isn't a renderer.
    pub fn texture(&mut self, model: &str) -> Option<std::sync::Arc<Texture>> {
        let key = normalise(model);
        if let Some(hit) = self.texture_cache.get(&key) {
            return hit.clone();
        }
        let built = self.build_texture(&key).map(std::sync::Arc::new);
        self.texture_cache.insert(key, built.clone());
        built
    }

    /// Like material_texture, but says why it failed.
    pub fn material_texture_diag(
        &mut self,
        material: &str,
        extra: &dyn Fn(&str) -> Option<Vec<u8>>,
    ) -> Result<std::sync::Arc<Texture>, &'static str> {
        if let Some(t) = self.material_texture(material, extra) {
            return Ok(t);
        }
        let vmt_path = format!("materials/{}.vmt", normalise(material));
        let Some(vmt) = extra(&vmt_path).or_else(|| self.read_file(&vmt_path).map(|(b, _)| b)) else {
            return Err("no .vmt");
        };
        let text = String::from_utf8_lossy(&vmt).into_owned();
        let text = match include_from_vmt(&text) {
            Some(inc) => {
                let inc = inc.trim_start_matches("materials/").trim_end_matches(".vmt").to_owned();
                let p = format!("materials/{inc}.vmt");
                match extra(&p).or_else(|| self.read_file(&p).map(|(b, _)| b)) {
                    Some(b) => String::from_utf8_lossy(&b).into_owned(),
                    None => return Err("patch include missing"),
                }
            }
            None => text,
        };
        let Some(base) = basetexture_from_vmt(&text) else {
            return Err("no $basetexture");
        };
        let vtf_path = format!("materials/{}.vtf", normalise(&base));
        let Some(vtf) = extra(&vtf_path).or_else(|| self.read_file(&vtf_path).map(|(b, _)| b)) else {
            return Err("no .vtf");
        };
        if decode_vtf(&vtf).is_none() {
            return Err("vtf decode failed");
        }
        Err("unknown")
    }

    /// The text of the VMT a material name resolves to. A patch material is
    /// returned with the material it includes, so a setting made in either
    /// is seen.
    pub fn material_vmt(&self, material: &str) -> Option<String> {
        let path = format!("materials/{}.vmt", normalise(material));
        let (bytes, _) = self.read_file(&path)?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let included = include_from_vmt(&text).and_then(|include| {
            let include = include.trim_start_matches("materials/").trim_end_matches(".vmt").to_owned();
            self.read_file(&format!("materials/{include}.vmt")).map(|(bytes, _)| String::from_utf8_lossy(&bytes).into_owned())
        });
        Some(match included {
            Some(base) => format!("{base}\n{text}"),
            None => text,
        })
    }

    /// The VMT behind a section key from MeshData, found the way
    /// `section_texture` finds the texture.
    pub fn section_vmt(&self, key: &str) -> Option<String> {
        let (dirs, name) = key.split_once("::").unwrap_or(("", key));
        dirs.split('|')
            .filter(|dir| !dir.is_empty())
            .find_map(|dir| self.material_vmt(&format!("{dir}/{name}")))
            .or_else(|| self.material_vmt(name))
    }

    /// A texture for a section key from MeshData ("dirs::name"). Tries each
    /// directory the model listed, then the name as a full path.
    pub fn section_texture(&mut self, key: &str) -> Option<std::sync::Arc<Texture>> {
        let cache_key = format!("section:{key}");
        if let Some(hit) = self.texture_cache.get(&cache_key) {
            return hit.clone();
        }
        let (dirs, name) = key.split_once("::").unwrap_or(("", key));
        let mut found = None;
        for dir in dirs.split('|').filter(|d| !d.is_empty()) {
            if let Some(t) = self.material_texture(&format!("{dir}/{name}"), &|_| None) {
                found = Some(t);
                break;
            }
        }
        if found.is_none() {
            found = self.material_texture(name, &|_| None);
        }
        self.texture_cache.insert(cache_key, found.clone());
        found
    }

    /// A texture for a material name, as a map face gives one:
    /// `concrete/concretefloor001a` -> materials/…vmt -> $basetexture -> vtf.
    /// `extra` is consulted before the archives, for a map's own packed
    /// materials.
    pub fn material_texture(
        &mut self,
        material: &str,
        extra: &dyn Fn(&str) -> Option<Vec<u8>>,
    ) -> Option<std::sync::Arc<Texture>> {
        let key = format!("material:{}", normalise(material));
        if let Some(hit) = self.texture_cache.get(&key) {
            return hit.clone();
        }
        let built = (|| {
            let vmt_path = format!("materials/{}.vmt", normalise(material));
            let vmt = extra(&vmt_path).or_else(|| self.read_file(&vmt_path).map(|(b, _)| b))?;
            let mut text = String::from_utf8_lossy(&vmt).into_owned();
            // A patch material has no $basetexture of its own; follow it.
            if let Some(inc) = include_from_vmt(&text) {
                let inc = inc.trim_start_matches("materials/").trim_end_matches(".vmt").to_owned();
                let p = format!("materials/{inc}.vmt");
                let b = extra(&p).or_else(|| self.read_file(&p).map(|(b, _)| b))?;
                text = String::from_utf8_lossy(&b).into_owned();
            }
            // Candidates, in order: $basetexture, $basetexture2 (a blend's
            // second layer — concrete/sand on a WorldVertexTransition), then
            // the material's own name, which is what the .vtf is usually
            // called anyway. Values sometimes carry "materials/" or ".vtf".
            let mut candidates: Vec<String> = Vec::new();
            if let Some(b) = basetexture_from_vmt(&text) {
                candidates.push(texture_name(&b));
            }
            if let Some(b) = basetexture2_from_vmt(&text) {
                candidates.push(texture_name(&b));
            }
            candidates.push(normalise(material));
            for base in &candidates {
                let vtf_path = format!("materials/{base}.vtf");
                if let Some(vtf) = extra(&vtf_path).or_else(|| self.read_file(&vtf_path).map(|(b, _)| b)) {
                    if let Some(t) = decode_vtf(&vtf) {
                        return Some(t);
                    }
                }
            }
            // Water has no base texture at all — it is refraction and a fog
            // colour. Draw the fog colour so it reads as water.
            if is_water_vmt(&text) || normalise(material).contains("water") {
                let (r, g, b) = fogcolor_from_vmt(&text).unwrap_or((22, 58, 84));
                return Some(solid_texture(r, g, b));
            }
            None
        })()
        .map(std::sync::Arc::new);
        self.texture_cache.insert(key, built.clone());
        built
    }

    fn build_texture(&self, key: &str) -> Option<Texture> {
        let (mdl_bytes, _) = self.read_file(key)?;
        let mdl = vmdl::mdl::Mdl::read(&mdl_bytes).ok()?;

        // An MDL stores material names and the directories to look in
        // separately, and the real path is every combination of the two.
        let dirs: Vec<String> = mdl
            .texture_paths
            .iter()
            .map(|d| normalise(d))
            .collect();

        for tex in mdl.textures.iter() {
            let name = normalise(&tex.name);
            let mut candidates: Vec<String> = Vec::new();
            for dir in &dirs {
                let dir = dir.trim_end_matches('/');
                candidates.push(format!("materials/{dir}/{name}.vmt"));
            }
            // Some models store a full path in the material name itself.
            candidates.push(format!("materials/{name}.vmt"));

            for vmt_path in candidates {
                let Some((vmt, _)) = self.read_file(&vmt_path) else {
                    continue;
                };
                let Some(base) = basetexture_from_vmt(&String::from_utf8_lossy(&vmt)) else {
                    continue;
                };
                let vtf_path = format!("materials/{}.vtf", normalise(&base));
                let Some((vtf_bytes, _)) = self.read_file(&vtf_path) else {
                    continue;
                };
                if let Some(t) = decode_vtf(&vtf_bytes) {
                    return Some(t);
                }
            }
        }
        None
    }

    /// A named attachment's local position on a model ("driveshaft",
    /// "input", "driveshaftL"…), from the MDL's attachment table. Cached.
    pub fn attachment(&mut self, model: &str, name: &str) -> Option<(f64, f64, f64)> {
        let key = format!("att:{}:{}", normalise(model), name.to_lowercase());
        if let Some(hit) = self.attachment_cache.get(&key) {
            return *hit;
        }
        let found = (|| {
            let (bytes, _) = self.read_file(&normalise(model))?;
            let mdl = vmdl::mdl::Mdl::read(&bytes).ok()?;
            let att = mdl.attachments.iter().find(|a| a.name.eq_ignore_ascii_case(name))?;
            let t = att.local.translate();
            Some((t.x as f64, t.y as f64, t.z as f64))
        })();
        self.attachment_cache.insert(key, found);
        found
    }

    /// Why a model has no mesh. A cube in the viewport is this function's
    /// answer being None, and "no dx90.vtx" and "vtx parse error" are
    /// different bugs.
    pub fn mesh_reason(&self, model: &str) -> String {
        let key = normalise(model);
        let Some(stem) = key.strip_suffix(".mdl") else {
            return "not a .mdl path".into();
        };
        let Some((mdl_bytes, _)) = self.read_file(&key) else {
            return format!("no file {key}");
        };
        let vtx_path = format!("{stem}.dx90.vtx");
        let Some((vtx_bytes, _)) = self.read_file(&vtx_path) else {
            return format!("no file {vtx_path}");
        };
        let vvd_path = format!("{stem}.vvd");
        let Some((vvd_bytes, _)) = self.read_file(&vvd_path) else {
            return format!("no file {vvd_path}");
        };
        if let Err(e) = vmdl::mdl::Mdl::read(&mdl_bytes) {
            return format!("mdl parse: {e}");
        }
        if let Err(e) = vmdl::vtx::Vtx::read(&vtx_bytes) {
            return format!("vtx parse: {e}");
        }
        if let Err(e) = vmdl::vvd::Vvd::read(&vvd_bytes) {
            return format!("vvd parse: {e}");
        }
        "all three files parse but produced no triangles".into()
    }

    fn build_mesh(&self, key: &str, skin: usize, bodygroups: &[usize]) -> Option<MeshData> {
        // vmdl wants .mdl, .dx90.vtx and .vvd. Its from_path reads them off
        // disk; ours come out of whichever archive holds them.
        let stem = key.strip_suffix(".mdl")?;
        let (mdl_bytes, _) = self.read_file(key)?;
        let (vtx_bytes, _) = self.read_file(&format!("{stem}.dx90.vtx"))?;
        let (vvd_bytes, _) = self.read_file(&format!("{stem}.vvd"))?;

        let mdl = vmdl::mdl::Mdl::read(&mdl_bytes).ok()?;
        let vtx = vmdl::vtx::Vtx::read(&vtx_bytes).ok()?;
        let vvd = vmdl::vvd::Vvd::read(&vvd_bytes).ok()?;
        let model = vmdl::Model::from_parts(mdl, vtx, vvd);

        let verts = model.vertices();
        // VVD vertices are in the reference pose. The game draws the idle
        // sequence, and many ACF models turn or shift their root bones in
        // it (guns are modelled along -Y and posed along +X; some have three
        // roots posed differently). So each vertex is skinned by its own
        // bones: idle pose times the bone's pose-to-bone matrix. Rotating
        // the whole model by bone 0 alone stood the 40 mm gun on end and
        // rolled the 20 mm one on its side.
        let bone_matrices = idle_bone_matrices(&model);
        let placed: Vec<([f32; 3], [f32; 3])> = verts
            .iter()
            .map(|v| {
                let at = [v.position.x, v.position.y, v.position.z];
                let normal = [v.normal.x, v.normal.y, v.normal.z];
                let (mut p, mut n, mut total) = ([0.0f32; 3], [0.0f32; 3], 0.0f32);
                for weight in v.bone_weights.weights() {
                    let Some(m) = bone_matrices.get(weight.bone_id as usize) else { continue };
                    for row in 0..3 {
                        p[row] += weight.weight * (m[row][0] * at[0] + m[row][1] * at[1] + m[row][2] * at[2] + m[row][3]);
                        n[row] += weight.weight * (m[row][0] * normal[0] + m[row][1] * normal[1] + m[row][2] * normal[2]);
                    }
                    total += weight.weight;
                }
                if total > 1e-6 {
                    (p.map(|c| c / total), n)
                } else {
                    (at, normal)
                }
            })
            .collect();
        let mut out = MeshData {
            positions: placed.iter().map(|(p, _)| *p).collect(),
            normals: placed.iter().map(|(_, n)| *n).collect(),
            uvs: verts.iter().map(|v| v.texture_coordinates).collect(),
            indices: Vec::new(),
            sections: Vec::new(),
            section_slots: Vec::new(),
        };

        // vmdl hands back strips already triangulated — Strip::indices expands
        // a tri-strip into a list and leaves a tri-list alone — so this is a
        // flatten rather than a re-triangulation. Each vmdl mesh carries its
        // own material index; skin 0 maps that to a material name, and the
        // model's texture directories give the folders to look in.
        let dirs: Vec<String> = model
            .texture_directories()
            .iter()
            .map(|d| normalise(d).trim_end_matches('/').to_owned())
            .collect();
        let skin = model.skin_tables().nth(skin).or_else(|| model.skin_tables().next());
        let limit = out.positions.len() as u32;
        for mesh in model.meshes_with(bodygroups) {
            let start = out.indices.len() as u32;
            for strip in mesh.vertex_strip_indices() {
                for index in strip {
                    let i = index as u32;
                    if i < limit {
                        out.indices.push(i);
                    }
                }
            }
            // Keep whole triangles only.
            let whole = out.indices.len() - out.indices.len() % 3;
            out.indices.truncate(whole);
            let count = out.indices.len() as u32 - start;
            if count == 0 {
                continue;
            }
            let name = skin
                .as_ref()
                .and_then(|t| t.texture(mesh.material_index()))
                .or_else(|| {
                    model
                        .textures()
                        .get(mesh.material_index() as usize)
                        .map(|t| t.name.as_str())
                })
                .map(normalise)
                .unwrap_or_default();
            // Stored as "dir|dir|dir::name" so the texture lookup can try
            // each directory without re-parsing the model.
            let key = format!("{}::{}", dirs.join("|"), name);
            out.sections.push((start, count, key));
            let slot = skin.as_ref().and_then(|table| table.texture_index(mesh.material_index())).unwrap_or(mesh.material_index() as usize);
            out.section_slots.push(slot);
        }

        if out.indices.is_empty() {
            return None;
        }
        Some(out)
    }

    fn locate(&self, key: &str) -> Option<(Bounds, Source)> {
        // 1. loose on disk
        for root in &self.roots {
            let p = root.join(key);
            if let Ok(data) = fs::read(&p) {
                if let Some(b) = read_bounds(&data) {
                    return Some((b, Source::Loose(p)));
                }
            }
        }

        // 2. inside a .gma
        if let Some((archive, entry)) = self.gma_index.get(key) {
            if let Some(bytes) = read_gma_entry(archive, entry) {
                if let Some(b) = read_bounds(&bytes) {
                    return Some((b, Source::Gma(archive.clone())));
                }
            }
        }

        // 3. inside a .vpk
        for (pak, vpk) in &self.vpks {
            if !vpk.contains(key) {
                continue;
            }
            let Ok(data) = vpk.read(key) else { continue };
            if let Some(b) = read_bounds(&data) {
                return Some((b, Source::Vpk(pak.clone())));
            }
        }

        None
    }
}

/// The bounding box out of an MDL header.
fn read_bounds(mdl: &[u8]) -> Option<Bounds> {
    let model = vmdl::mdl::Mdl::read(mdl).ok()?;
    let bb = model.header.bounding_box;
    Some(Bounds {
        min: (bb[0].x as f64, bb[0].y as f64, bb[0].z as f64),
        max: (bb[1].x as f64, bb[1].y as f64, bb[1].z as f64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built GMA with two entries, to prove the index walk lands on the
    /// right offsets. Layout per Facepunch/gmad AddonReader.h.
    #[test]
    fn gma_index_finds_entries_at_the_right_offsets() {
        let mut g = Vec::new();
        g.extend_from_slice(b"GMAD");
        g.push(3); // version
        g.extend_from_slice(&0u64.to_le_bytes()); // steamid
        g.extend_from_slice(&0u64.to_le_bytes()); // timestamp
        g.push(0); // required content: empty string terminates
        g.extend_from_slice(b"name\0desc\0author\0");
        g.extend_from_slice(&1i32.to_le_bytes()); // addon version

        for (n, name, size) in [(1u32, "models/a.mdl", 4i64), (2u32, "models/b.mdl", 6i64)] {
            g.extend_from_slice(&n.to_le_bytes());
            g.extend_from_slice(name.as_bytes());
            g.push(0);
            g.extend_from_slice(&size.to_le_bytes());
            g.extend_from_slice(&0u32.to_le_bytes()); // crc
        }
        g.extend_from_slice(&0u32.to_le_bytes()); // end of index
        let block = g.len() as u64;
        g.extend_from_slice(b"AAAA");
        g.extend_from_slice(b"BBBBBB");

        let idx = index_gma(&g).expect("should parse");
        assert_eq!(idx.len(), 2);
        assert_eq!(idx[0].0, "models/a.mdl");
        assert_eq!(idx[0].1.offset, block);
        assert_eq!(idx[0].1.size, 4);
        assert_eq!(idx[1].0, "models/b.mdl");
        assert_eq!(idx[1].1.offset, block + 4);
        assert_eq!(idx[1].1.size, 6);

        // And the offsets actually address the right bytes.
        let a = &g[idx[0].1.offset as usize..][..idx[0].1.size as usize];
        let b = &g[idx[1].1.offset as usize..][..idx[1].1.size as usize];
        assert_eq!(a, b"AAAA");
        assert_eq!(b, b"BBBBBB");
    }

    /// A 64 MB GMA whose index is a few hundred bytes. If the reader touches
    /// the whole file this takes visible time; it should take none.
    #[test]
    fn a_huge_gma_is_indexed_from_its_prefix_only() {
        let dir = std::env::temp_dir().join("ad2read_gma_test");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("big.gma");

        let mut g = Vec::new();
        g.extend_from_slice(b"GMAD");
        g.push(3);
        g.extend_from_slice(&0u64.to_le_bytes());
        g.extend_from_slice(&0u64.to_le_bytes());
        g.push(0);
        g.extend_from_slice(b"name\0desc\0author\0");
        g.extend_from_slice(&1i32.to_le_bytes());
        g.extend_from_slice(&1u32.to_le_bytes());
        g.extend_from_slice(b"models/big.mdl\0");
        let payload: i64 = 64 << 20;
        g.extend_from_slice(&payload.to_le_bytes());
        g.extend_from_slice(&0u32.to_le_bytes());
        g.extend_from_slice(&0u32.to_le_bytes());
        let header_len = g.len();

        // Write the index, then extend the file to 64 MB without holding
        // it in memory.
        fs::write(&path, &g).unwrap();
        let f = fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(header_len as u64 + payload as u64).unwrap();

        let started = std::time::Instant::now();
        let index = read_gma_index(&path).expect("index");
        let took = started.elapsed();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].0, "models/big.mdl");
        assert_eq!(index[0].1.offset, header_len as u64);
        assert!(
            took.as_millis() < 500,
            "indexing read far more than the prefix: {took:?}"
        );

        let _ = fs::remove_file(&path);
    }

    /// The rotary autocannon failed with "invalid utf-8 sequence of 1 bytes
    /// from index 0": a name whose first byte is >= 0x80. vmdl is vendored
    /// with lossy decoding so that model loads instead of drawing as a cube.
    #[test]
    fn vmdl_accepts_a_fixed_name_that_is_not_utf8() {
        use std::convert::TryFrom;
        let mut buf = [0u8; 64];
        buf[0] = 0xE9; // a lone Latin-1 'é'
        buf[1] = b'k';
        buf[2] = b'w';
        let name = vmdl::FixedString::<64>::try_from(buf).expect("lossy, not an error");
        let text: &str = name.as_ref();
        assert!(text.ends_with("kw"), "{text:?}");
        assert!(text.starts_with('\u{FFFD}'), "{text:?}");
    }

    /// A VPK v1 whose only file lives entirely in the directory's preload
    /// section (archive index 0x7fff, zero archive bytes) — how small VMTs
    /// are stored in garrysmod_dir.vpk. The whole content must come back.
    #[test]
    fn a_preload_only_vpk_entry_reads_in_full() {
        let dir = std::env::temp_dir().join(format!("ad2read_vpk_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let pak = dir.join("test_dir.vpk");

        let content = b"\"VertexLitGeneric\"\n{\n\t\"$basetexture\" \"models/props/x\"\n}\n";
        // Tree: ext "vmt" / path "materials/models" / name "x" / entry.
        let mut tree = Vec::new();
        tree.extend_from_slice(b"vmt\0");
        tree.extend_from_slice(b"materials/models\0");
        tree.extend_from_slice(b"x\0");
        tree.extend_from_slice(&0u32.to_le_bytes()); // crc (unchecked)
        tree.extend_from_slice(&(content.len() as u16).to_le_bytes()); // preload bytes
        tree.extend_from_slice(&0x7fffu16.to_le_bytes()); // archive index: directory
        tree.extend_from_slice(&0u32.to_le_bytes()); // entry offset
        tree.extend_from_slice(&0u32.to_le_bytes()); // entry length (all in preload)
        tree.extend_from_slice(&0xffffu16.to_le_bytes()); // terminator
        tree.extend_from_slice(content); // preload data follows the entry
        tree.extend_from_slice(b"\0"); // end of names in this path
        tree.extend_from_slice(b"\0"); // end of paths in this ext
        tree.extend_from_slice(b"\0"); // end of exts

        let mut file = Vec::new();
        file.extend_from_slice(&0x55aa1234u32.to_le_bytes());
        file.extend_from_slice(&1u32.to_le_bytes()); // v1
        file.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        file.extend_from_slice(&tree);
        fs::write(&pak, &file).unwrap();

        let vpk = source_vpk::Vpk::open(&pak).expect("open");
        assert!(vpk.contains("materials/models/x.vmt"));
        let data = vpk.read("materials/models/x.vmt").expect("read");
        assert_eq!(data, content, "preload bytes must be returned in full");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vbsp_cubemap_patch_include_is_read_with_quoted_and_bare_keys() {
        // Exactly what buildcubemaps writes into the BSP pak.
        let quoted = "\"patch\"\n{\n\t\"include\"\t\"materials/building_template/building_template006b.vmt\"\n\t\"replace\"\n\t{\n\t\t\"$envmap\" \"maps/gm_construct/c-256_-1472_-96\"\n\t}\n}\n";
        assert_eq!(include_from_vmt(quoted).as_deref(), Some("materials/building_template/building_template006b.vmt"));
        // Hand-written variants people ship.
        let bare = "patch\n{\n\tinclude materials/x/y.vmt\n\tinsert { $envmap env }\n}";
        assert_eq!(include_from_vmt(bare).as_deref(), Some("materials/x/y.vmt"));
        let bare_key_quoted_value = "patch { include \"Materials\\X\\Y.VMT\" }";
        assert_eq!(include_from_vmt(bare_key_quoted_value).as_deref(), Some("materials/x/y.vmt"));
        // A patch stub itself has no basetexture; the include is the only way in.
        assert!(basetexture_from_vmt(quoted).is_none());
    }

    #[test]
    fn texture_names_and_water_fog_parse() {
        assert_eq!(texture_name("Materials\\Nature\\Sand.VTF"), "nature/sand");
        let water = "\"Water\"\n{\n\t\"$refracttexture\" \"_rt_WaterRefraction\"\n\t\"$fogcolor\" \"{ 12 33 46 }\"\n}";
        assert!(is_water_vmt(water));
        assert_eq!(fogcolor_from_vmt(water), Some((12, 33, 46)));
        let water01 = "Water { $fogcolor \"[0.5 0.25 1]\" }";
        assert_eq!(fogcolor_from_vmt(water01), Some((127, 63, 255)));
        let blend = "\"WorldVertexTransition\"\n{\n\t\"$basetexture\" \"a/concrete\"\n\t\"$basetexture2\" \"a/sand\"\n}";
        assert_eq!(basetexture_from_vmt(blend).as_deref(), Some("a/concrete"));
        assert_eq!(basetexture2_from_vmt(blend).as_deref(), Some("a/sand"));
        assert!(!is_water_vmt(blend));
    }

    #[test]
    fn rejects_things_that_are_not_gmas() {
        assert!(index_gma(b"not a gma at all").is_none());
        assert!(index_gma(b"").is_none());
    }

    #[test]
    fn paths_normalise_to_one_form() {
        assert_eq!(normalise("Models\\Props\\Cake.MDL"), "models/props/cake.mdl");
        assert_eq!(normalise("\"models/a.mdl\""), "models/a.mdl");
    }

    #[test]
    fn basetexture_is_read_out_of_a_vmt() {
        let vmt = "\"VertexLitGeneric\"\n{\n\t\"$basetexture\" \"models/props/crate01\"\n}";
        assert_eq!(
            basetexture_from_vmt(vmt).as_deref(),
            Some("models/props/crate01")
        );
        // Unquoted, backslashes, and a blend layer that must not win.
        let vmt2 = "LightmappedGeneric\n{\n$basetexture2 other\n$basetexture models\\a\\b\n}";
        assert_eq!(basetexture_from_vmt(vmt2).as_deref(), Some("models/a/b"));
        assert_eq!(basetexture_from_vmt("no material here"), None);
    }

    #[test]
    fn patch_materials_point_at_their_include() {
        let vmt = "patch\n{\n\tinclude \"materials/concrete/concretefloor001a.vmt\"\n\tinsert { }\n}";
        assert_eq!(
            include_from_vmt(vmt).as_deref(),
            Some("materials/concrete/concretefloor001a.vmt")
        );
        assert_eq!(include_from_vmt("LightmappedGeneric { $basetexture x }"), None);
    }

    #[test]
    fn half_extents_are_half_the_span() {
        let b = Bounds {
            min: (-10.0, -4.0, 0.0),
            max: (10.0, 4.0, 20.0),
        };
        assert_eq!(b.half_extents(), (10.0, 4.0, 10.0));
        assert_eq!(b.centre(), (0.0, 0.0, 10.0));
    }
}

