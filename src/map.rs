//! Source BSP maps as a backdrop for a build.
//!
//! Three things make a whole city drawable at frame rate, in order of how
//! much they matter:
//!
//! 1. **Batching.** Faces are grouped by material into one vertex buffer
//!    each. The cost that kills a naive renderer is a draw call per face —
//!    CPU-side, indifferent to how many are on screen. `gm_bigcity` goes from
//!    ~20k draws to ~200 this way, before anything is culled.
//! 2. **PVS.** The map ships a potentially-visible set computed by `vvis`
//!    at compile time: for each leaf, which other leaves can be seen from
//!    anywhere inside it. That is occlusion culling that was paid for once,
//!    and it removes what's behind walls — most of a city from any street.
//! 3. **Frustum.** PVS is conservative (any position, any direction in the
//!    leaf), so the surviving leaves are tested against the view frustum to
//!    drop what's behind the camera.
//!
//! Within each material batch, faces are sorted by leaf so a leaf's faces
//! are one contiguous index range. Culling then becomes: visible leaf set →
//! coalesced index ranges per batch → a handful of draws.
//!
//! No lightmaps: this is fullbright, which is what the viewport is for.

use std::collections::HashMap;

/// One vertex of map geometry.
#[derive(Clone, Copy, Debug, Default)]
pub struct MapVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Baked light sampled from the face's lightmap at this vertex; white
    /// when the map has no lighting or the face no lightmap.
    pub light: [f32; 3],
}

/// The LDR lighting lump (lump 8): ColorRGBExp32 samples, four bytes each,
/// addressed by each face's light_offset. vbsp doesn't keep it, but the
/// header's lump directory is 64 × 16 bytes after the 8-byte header.
fn lighting_lump(bytes: &[u8]) -> Option<&[u8]> {
    let base = 8 + 8 * 16;
    let ofs = u32::from_le_bytes(bytes.get(base..base + 4)?.try_into().ok()?) as usize;
    let len = u32::from_le_bytes(bytes.get(base + 4..base + 8)?.try_into().ok()?) as usize;
    if len == 0 {
        return None;
    }
    bytes.get(ofs..ofs + len)
}

/// One ColorRGBExp32 → linear 0..1 rgb, with Source's overbright headroom
/// and a display gamma so mid-lit surfaces don't read as black.
fn luxel(lighting: &[u8], byte_offset: usize) -> Option<[f32; 3]> {
    let s = lighting.get(byte_offset..byte_offset + 4)?;
    let e = (s[3] as i8) as f32;
    let scale = 2f32.powf(e) / 255.0;
    let f = |c: u8| ((c as f32 * scale) * 1.6).clamp(0.0, 1.0).powf(1.0 / 2.2);
    Some([f(s[0]), f(s[1]), f(s[2])])
}

/// A face's lightmap sample at a world point, via the texinfo lightmap
/// vectors: luxel s = p·vec_s + offset_s − mins_s (t likewise), clamped
/// to the face's luxel rectangle.
fn sample_face_light(
    lighting: &[u8],
    face: &vbsp::data::Face,
    tex: &vbsp::data::TextureInfo,
    p: vbsp::Vector,
) -> Option<[f32; 3]> {
    if face.light_offset < 0 {
        return None;
    }
    let w = face.light_map_texture_size[0] + 1;
    let h = face.light_map_texture_size[1] + 1;
    if w <= 0 || h <= 0 {
        return None;
    }
    let sv = tex.light_map_scale;
    let tv = tex.light_map_transform;
    let s = p.x * sv[0] + p.y * sv[1] + p.z * sv[2] + sv[3] - face.light_map_texture_min[0] as f32;
    let t = p.x * tv[0] + p.y * tv[1] + p.z * tv[2] + tv[3] - face.light_map_texture_min[1] as f32;
    let si = (s.round() as i32).clamp(0, w - 1);
    let ti = (t.round() as i32).clamp(0, h - 1);
    luxel(lighting, face.light_offset as usize + ((ti * w + si) * 4) as usize)
}

/// A displacement's lightmap is laid over its vertex grid, not projected:
/// luxel (gx/steps·w, gy/steps·h).
fn sample_disp_light(lighting: &[u8], face: &vbsp::data::Face, gx: usize, gy: usize, steps: usize) -> Option<[f32; 3]> {
    if face.light_offset < 0 {
        return None;
    }
    let w = face.light_map_texture_size[0] + 1;
    let h = face.light_map_texture_size[1] + 1;
    if w <= 0 || h <= 0 || steps == 0 {
        return None;
    }
    let si = ((gx as f32 / steps as f32) * (w - 1) as f32).round() as i32;
    let ti = ((gy as f32 / steps as f32) * (h - 1) as f32).round() as i32;
    luxel(lighting, face.light_offset as usize + ((ti.clamp(0, h - 1) * w + si.clamp(0, w - 1)) * 4) as usize)
}

/// One face's slice of a batch, and every leaf that contains it.
///
/// A big wall face sits in many leaves. Attaching it to just the first one
/// made it vanish whenever that leaf was culled while another leaf holding
/// the same face was in view — which is what "things disappearing while
/// still visible" was. Source draws a face if ANY of its leaves is visible,
/// and so does this. An empty leaf list means always draw (brush entities,
/// which live in no leaf list at all).
pub struct FaceRange {
    pub start: u32,
    pub count: u32,
    pub leaves: Vec<u32>,
}

/// All faces sharing one material.
pub struct MaterialBatch {
    pub material: String,
    pub vertices: Vec<MapVertex>,
    pub indices: Vec<u32>,
    pub faces: Vec<FaceRange>,
}

#[derive(Clone, Copy, Debug)]
pub struct LeafBounds {
    pub cluster: i16,
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

/// A prop the map places itself.
#[derive(Clone, Debug)]
pub struct StaticProp {
    pub model: String,
    pub origin: [f32; 3],
    pub angles: [f32; 3],
    /// Leaves the prop touches, for culling.
    pub leaves: Vec<u32>,
}

pub struct Map {
    pub name: String,
    pub batches: Vec<MaterialBatch>,
    pub leaves: Vec<LeafBounds>,
    pub props: Vec<StaticProp>,
    /// A spawn point, if the map declares one.
    pub spawn: Option<[f32; 3]>,
    /// Displacement faces drawn (terrain).
    pub displacements: usize,
    /// Whether vertices carry baked lighting from the LDR lump.
    pub lit: bool,
    /// lowercase pak path -> the name the zip actually stores
    pak_index: std::collections::HashMap<String, String>,
    /// worldspawn's skyname, for the 2D skybox.
    pub skyname: Option<String>,
    vis: vbsp::VisData,
    /// The BSP tree stays loaded for leaf_at, which walks it.
    bsp: vbsp::Bsp,
}

/// Loads a map. `bytes` is the whole .bsp.
pub fn load(name: &str, bytes: &[u8]) -> Result<Map, String> {
    let bsp = vbsp::Bsp::read(bytes).map_err(|e| format!("{name}: {e}"))?;
    let lighting = lighting_lump(bytes);

    // The 3D skybox is a separate sealed room somewhere in the world; the
    // engine draws it scaled around the camera and hides the room itself.
    // Drawn at 1:1 it's a floating sheet of terrain in the sky. The room is
    // sealed, so the PVS of sky_camera's own leaf is exactly the room — a
    // cluster set we can cull by. (Areas don't work: without areaportals
    // the room is area 0 like the world.)
    let sky_clusters = bsp.entities.iter().find_map(|e| {
        if e.prop("classname") != Some("sky_camera") {
            return None;
        }
        let origin = e.prop("origin")?;
        let mut it = origin.split_whitespace().filter_map(|x| x.parse::<f32>().ok());
        let (x, y, z) = (it.next()?, it.next()?, it.next()?);
        let leaf = bsp.leaf_at(vbsp::Vector { x, y, z });
        let c = leaf.cluster;
        if c < 0 || c as usize >= bsp.vis_data.cluster_count as usize {
            return None;
        }
        Some(bsp.vis_data.visible_clusters(c))
    });
    let in_sky = |cluster: i16| -> bool {
        match (&sky_clusters, cluster) {
            (Some(set), c) if c >= 0 && (c as u64) < set.len() => set.get(c as u64),
            _ => false,
        }
    };

    // First: which leaves hold each face. A face can be in many.
    let mut face_leaves: Vec<Vec<u32>> = vec![Vec::new(); bsp.faces.len()];
    let leaf_count = bsp.leaves.len();
    let mut leaves = Vec::with_capacity(leaf_count);
    for leaf_index in 0..leaf_count {
        let Some(leaf) = bsp.leaf(leaf_index) else { continue };
        leaves.push(LeafBounds {
            cluster: leaf.cluster,
            mins: [leaf.mins[0] as f32, leaf.mins[1] as f32, leaf.mins[2] as f32],
            maxs: [leaf.maxs[0] as f32, leaf.maxs[1] as f32, leaf.maxs[2] as f32],
        });
        if in_sky(leaf.cluster) {
            // Keep the slot (indices must line up) but as a leaf that never
            // draws: cluster -1 is skipped by visible_leaves.
            if let Some(last) = leaves.last_mut() {
                last.cluster = -1;
            }
            continue;
        }
        let first = leaf.first_leaf_face as usize;
        let count = leaf.leaf_face_count as usize;
        for lf in first..(first + count).min(bsp.leaf_faces.len()) {
            let face_index = bsp.leaf_faces[lf].face as usize;
            if let Some(list) = face_leaves.get_mut(face_index) {
                list.push(leaf_index as u32);
            }
        }
    }

    // Brush entities — doors, buttons, func_brush and the rest — are
    // models[1..], and their faces appear in no leaf's list. Without this
    // pass they are simply never emitted, which was the missing geometry.
    // They get no leaf list, so they always draw.
    let mut brush_faces: Vec<usize> = Vec::new();
    for model in bsp.models.iter().skip(1) {
        let first = model.first_face.max(0) as usize;
        let n = model.face_count.max(0) as usize;
        for f in first..(first + n).min(bsp.faces.len()) {
            brush_faces.push(f);
        }
    }

    // material name -> batch being built
    let mut building: HashMap<String, (Vec<MapVertex>, Vec<u32>, Vec<FaceRange>)> =
        HashMap::new();

    let mut emit = |face_index: usize, leaves_of: Vec<u32>| {
        let Some(face) = bsp.face(face_index) else { return };
        let tex = face.texture();
        let flags = tex.flags;
        if flags.contains(vbsp::TextureFlags::SKY)
            || flags.contains(vbsp::TextureFlags::SKY2D)
            || flags.contains(vbsp::TextureFlags::NODRAW)
            || flags.contains(vbsp::TextureFlags::TRIGGER)
        {
            return;
        }
        let material = tex.texture_data().name().to_lowercase().replace('\\', "/");
        let normal = face.normal();
        let n = [normal.x, normal.y, normal.z];

        let (verts, idx, faces) = building
            .entry(material)
            .or_insert_with(|| (Vec::new(), Vec::new(), Vec::new()));
        let start = idx.len() as u32;
        // vbsp's triangulated_displaced_vertices indexes a vertex grid it
        // assumes complete; a displacement whose vertex run is short (real
        // maps have them) panics there. Build the grid ourselves and check.
        let face_data: &vbsp::data::Face = &*face;
        let tex_data: &vbsp::data::TextureInfo = &*tex;
        // (position, light) pairs
        let lit: Vec<(vbsp::Vector, [f32; 3])> = match face.displacement() {
            Some(disp) => {
                let grid: Vec<vbsp::Vector> = disp.displaced_vertices().collect();
                let steps = 2usize.pow(disp.power as u32);
                let side = steps + 1;
                if grid.len() != side * side {
                    return;
                }
                let at = |x: usize, y: usize| {
                    let l = lighting
                        .and_then(|lm| sample_disp_light(lm, face_data, x, y, steps))
                        .unwrap_or([1.0, 1.0, 1.0]);
                    (grid[y * side + x], l)
                };
                let mut out = Vec::with_capacity(steps * steps * 6);
                for x in 0..steps {
                    for y in 0..steps {
                        out.extend_from_slice(&[at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y), at(x + 1, y + 1), at(x, y + 1)]);
                    }
                }
                out
            }
            None => face
                .vertex_positions()
                .map(|pos| {
                    let l = lighting
                        .and_then(|lm| sample_face_light(lm, face_data, tex_data, pos))
                        .unwrap_or([1.0, 1.0, 1.0]);
                    (pos, l)
                })
                .collect(),
        };
        for (pos, light) in lit {
            let uv = tex.uv(pos);
            idx.push(verts.len() as u32);
            verts.push(MapVertex {
                position: [pos.x, pos.y, pos.z],
                normal: n,
                uv,
                light,
            });
        }
        let count = idx.len() as u32 - start;
        if count > 0 {
            faces.push(FaceRange {
                start,
                count,
                leaves: leaves_of,
            });
        }
    };

    let mut emitted = vec![false; bsp.faces.len()];
    for (face_index, list) in face_leaves.iter().enumerate() {
        if !list.is_empty() {
            emit(face_index, list.clone());
            emitted[face_index] = true;
        }
    }
    for face_index in brush_faces {
        if face_leaves[face_index].is_empty() {
            emit(face_index, Vec::new());
            emitted[face_index] = true;
        }
    }
    // Displacement faces — the terrain, the grass field on gm_construct —
    // are in no leaf's face list and belong to no brush model: the engine
    // renders them through its own displacement path. Emit them here as
    // always-visible ranges, or the whole ground goes missing.
    let mut displacements = 0;
    for face_index in 0..bsp.faces.len() {
        if emitted[face_index] {
            continue;
        }
        let Some(f) = bsp.face(face_index) else { continue };
        let Some(disp) = f.displacement() else { continue };
        // Its leaf, for PVS culling and the skybox test. A corner sits on
        // the surface boundary and lands in a solid leaf (cluster -1), which
        // is what made the ground vanish — so try a point just above the
        // centre first, then the centre, then the corner, and fall back to
        // always-visible rather than never-visible.
        let grid: Vec<vbsp::Vector> = disp.displaced_vertices().collect();
        let n = f.normal();
        let centre = if grid.is_empty() {
            disp.start_position
        } else {
            let k = grid.len() as f32;
            let sum = grid.iter().fold((0.0f32, 0.0f32, 0.0f32), |a, v| (a.0 + v.x, a.1 + v.y, a.2 + v.z));
            vbsp::Vector { x: sum.0 / k, y: sum.1 / k, z: sum.2 / k }
        };
        let probes = [
            vbsp::Vector { x: centre.x + n.x * 4.0, y: centre.y + n.y * 4.0, z: centre.z + n.z * 4.0 },
            centre,
            disp.start_position,
        ];
        let mut leaf_index: Vec<u32> = Vec::new();
        let mut skybox = false;
        for pnt in probes {
            let leaf = bsp.leaf_at(pnt);
            if leaf.cluster < 0 {
                continue;
            }
            if in_sky(leaf.cluster) {
                skybox = true;
            }
            if let Some(i) = bsp.leaves.iter().position(|l| std::ptr::eq(l, &*leaf)) {
                leaf_index = vec![i as u32];
            }
            break;
        }
        if skybox {
            continue;
        }
        emit(face_index, leaf_index);
        emitted[face_index] = true;
        displacements += 1;
    }

    let mut batches: Vec<MaterialBatch> = building
        .into_iter()
        .map(|(material, (vertices, indices, faces))| MaterialBatch {
            material,
            vertices,
            indices,
            faces,
        })
        .collect();
    batches.sort_by(|a, b| a.material.cmp(&b.material));

    // Static props, with the leaf lists the lump carries for them.
    let mut props = Vec::new();
    for prop in bsp.static_props() {
        if in_sky(bsp.leaf_at(prop.origin).cluster) {
            continue;
        }
        let model = prop.model().to_lowercase().replace('\\', "/");
        let first = prop.first_leaf as usize;
        let n = prop.leaf_count as usize;
        let leaves_of: Vec<u32> = bsp
            .static_props
            .leaf
            .leaves
            .iter()
            .skip(first)
            .take(n)
            .map(|l| *l as u32)
            .collect();
        props.push(StaticProp {
            model,
            origin: [prop.origin.x, prop.origin.y, prop.origin.z],
            angles: [prop.angles.pitch, prop.angles.yaw, prop.angles.roll],
            leaves: leaves_of,
        });
    }

    let skyname = bsp.entities.iter().find_map(|e| {
        (e.prop("classname")? == "worldspawn").then(|| e.prop("skyname").map(|s| s.to_owned()))?
    });

    // Where the map would put a player.
    let spawn = bsp.entities.iter().find_map(|e| {
        let class = e.prop("classname")?;
        if class != "info_player_start" {
            return None;
        }
        let origin = e.prop("origin")?;
        let mut it = origin.split_whitespace().filter_map(|s| s.parse::<f32>().ok());
        Some([it.next()?, it.next()?, it.next()?])
    });

    let vis = vbsp::VisData {
        cluster_count: bsp.vis_data.cluster_count,
        pvs_offsets: bsp.vis_data.pvs_offsets.clone(),
        pas_offsets: bsp.vis_data.pas_offsets.clone(),
        data: bsp.vis_data.data.clone(),
    };

    let pak_index: std::collections::HashMap<String, String> = bsp
        .pack
        .names()
        .into_iter()
        .map(|n| (n.to_lowercase().replace('\\', "/"), n))
        .collect();

    Ok(Map {
        displacements,
        lit: lighting.is_some(),
        pak_index,
        name: name.to_owned(),
        batches,
        leaves,
        props,
        spawn,
        skyname,
        vis,
        bsp,
    })
}

/// Six planes as (normal, d) with the inside on the positive side.
pub type Frustum = [[f32; 4]; 6];

/// Extracts the frustum from a column-major view-projection matrix
/// (Gribb & Hartmann). Depth is 0..1 as wgpu has it, so the near plane is
/// row 2 on its own rather than row 3 + row 2.
pub fn frustum_of(vp: [[f32; 4]; 4]) -> Frustum {
    // vp[c][r]: pull the four ROWS back out of the column-major array.
    let row = |r: usize| [vp[0][r], vp[1][r], vp[2][r], vp[3][r]];
    let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    let mut planes = [
        add(r3, r0), // left
        sub(r3, r0), // right
        add(r3, r1), // bottom
        sub(r3, r1), // top
        r2,          // near (z >= 0)
        sub(r3, r2), // far
    ];
    for p in planes.iter_mut() {
        let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-9);
        for v in p.iter_mut() {
            *v /= len;
        }
    }
    planes
}

/// Whether an axis-aligned box touches the frustum. Uses the "positive
/// vertex" test: for each plane, the box corner furthest along the normal
/// must be inside, or the whole box is out.
pub fn aabb_in_frustum(f: &Frustum, mins: [f32; 3], maxs: [f32; 3]) -> bool {
    for p in f {
        let px = if p[0] >= 0.0 { maxs[0] } else { mins[0] };
        let py = if p[1] >= 0.0 { maxs[1] } else { mins[1] };
        let pz = if p[2] >= 0.0 { maxs[2] } else { mins[2] };
        if p[0] * px + p[1] * py + p[2] * pz + p[3] < 0.0 {
            return false;
        }
    }
    true
}

impl Map {
    /// Leaves worth drawing from `eye` with this frustum: PVS first, then a
    /// frustum test on what survives. A leaf with cluster -1 is solid or
    /// outside the map and never drawn.
    pub fn visible_leaves(&self, eye: [f32; 3], frustum: &Frustum) -> Vec<bool> {
        let mut out = vec![false; self.leaves.len()];

        let here = self.bsp.leaf_at(vbsp::Vector {
            x: eye[0],
            y: eye[1],
            z: eye[2],
        });
        let cluster = here.cluster;

        // Outside the map (in solid, or beyond the world): PVS has nothing
        // for us, so fall back to frustum-only rather than drawing nothing.
        let pvs = if cluster >= 0 && (cluster as u32) < self.vis.cluster_count {
            Some(self.vis.visible_clusters(cluster))
        } else {
            None
        };

        for (i, leaf) in self.leaves.iter().enumerate() {
            if leaf.cluster < 0 {
                continue;
            }
            if let Some(pvs) = pvs.as_ref() {
                // BitVec::get panics past the end; a cluster the PVS doesn't
                // cover is treated as visible rather than as a crash.
                let c = leaf.cluster as u64;
                if c < pvs.len() && !pvs.get(c) {
                    continue;
                }
            }
            out[i] = aabb_in_frustum(frustum, leaf.mins, leaf.maxs);
        }
        out
    }

    /// Coalesced (first, count) index ranges to draw for one batch. A face
    /// draws if any of its leaves is visible, or if it has none. Adjacent
    /// ranges merge so the draw count stays small.
    pub fn draw_ranges(batch: &MaterialBatch, visible: &[bool]) -> Vec<(u32, u32)> {
        let mut out: Vec<(u32, u32)> = Vec::new();
        for f in &batch.faces {
            let seen = f.leaves.is_empty()
                || f
                    .leaves
                    .iter()
                    .any(|l| visible.get(*l as usize).copied().unwrap_or(false));
            if !seen {
                continue;
            }
            match out.last_mut() {
                Some((s, c)) if *s + *c == f.start => *c += f.count,
                _ => out.push((f.start, f.count)),
            }
        }
        out
    }

    /// A file packed inside the map itself, or None.
    /// A file from the BSP's embedded pak, matched case-insensitively —
    /// custom-packed textures keep whatever casing the author used.
    pub fn packed(&self, path: &str) -> Option<Vec<u8>> {
        let want = path.to_lowercase().replace('\\', "/");
        let real = self.pak_index.get(&want).map(|s| s.as_str()).unwrap_or(path);
        self.bsp.pack.get(real).ok().flatten()
    }

    pub fn triangles(&self) -> usize {
        self.batches.iter().map(|b| b.indices.len() / 3).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_luxel_decodes_rgbexp32_with_headroom_and_gamma() {
        // (255, 128, 0) at exponent 0 → red saturates, green ~0.8, blue 0.
        let lm = [255u8, 128, 0, 0, 10, 10, 10, 3];
        let a = luxel(&lm, 0).unwrap();
        assert_eq!(a[0], 1.0);
        assert!(a[1] > 0.75 && a[1] < 0.95, "{}", a[1]);
        assert_eq!(a[2], 0.0);
        // exponent 3 multiplies by 8: 10 * 8 = 80 → dim but not black.
        let b = luxel(&lm, 4).unwrap();
        assert!(b[0] > 0.4 && b[0] < 0.9, "{}", b[0]);
        assert!(luxel(&lm, 8).is_none(), "past the end");
    }

    #[test]
    fn the_lighting_lump_is_read_from_the_header_directory() {
        // A fake BSP: header, 64 lump entries, lighting (lump 8) pointing at 8 bytes.
        let mut f = vec![0u8; 8 + 64 * 16];
        f[..4].copy_from_slice(b"VBSP");
        let base = 8 + 8 * 16;
        let ofs = f.len() as u32;
        f[base..base + 4].copy_from_slice(&ofs.to_le_bytes());
        f[base + 4..base + 8].copy_from_slice(&8u32.to_le_bytes());
        f.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let lump = lighting_lump(&f).unwrap();
        assert_eq!(lump, &[1, 2, 3, 0, 4, 5, 6, 0]);
        let mut g = f.clone();
        g[base + 4..base + 8].copy_from_slice(&0u32.to_le_bytes());
        assert!(lighting_lump(&g).is_none(), "zero length = unlit map");
    }

    /// A view-projection looking down -Z from the origin, as the renderer
    /// builds it, so the frustum test is checked against the real matrix.
    fn looking_down_z() -> Frustum {
        let vp = crate::gpu::view_projection(
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, -1.0],
            60f32.to_radians(),
            1.0,
            1.0,
            1000.0,
        );
        frustum_of(vp)
    }

    #[test]
    fn a_box_in_front_is_visible_and_one_behind_is_not() {
        let f = looking_down_z();
        assert!(aabb_in_frustum(&f, [-5.0, -5.0, -20.0], [5.0, 5.0, -10.0]), "in front");
        assert!(!aabb_in_frustum(&f, [-5.0, -5.0, 10.0], [5.0, 5.0, 20.0]), "behind");
        assert!(!aabb_in_frustum(&f, [500.0, -5.0, -20.0], [510.0, 5.0, -10.0]), "far right");
        assert!(!aabb_in_frustum(&f, [-5.0, -5.0, -3000.0], [5.0, 5.0, -2000.0]), "past far");
    }

    #[test]
    fn a_box_straddling_the_edge_still_counts() {
        let f = looking_down_z();
        // Huge box centred way off to the side, but it reaches the view.
        assert!(aabb_in_frustum(&f, [-1000.0, -1000.0, -50.0], [1000.0, 1000.0, -40.0]));
    }

    #[test]
    fn draw_ranges_merge_neighbours_and_skip_hidden() {
        let batch = MaterialBatch {
            material: "x".into(),
            vertices: Vec::new(),
            indices: vec![0; 60],
            faces: vec![
                FaceRange { start: 0, count: 12, leaves: vec![0] },
                FaceRange { start: 12, count: 12, leaves: vec![1] },
                FaceRange { start: 24, count: 12, leaves: vec![2] },
                // In a hidden leaf AND a visible one: must still draw.
                FaceRange { start: 36, count: 12, leaves: vec![2, 3] },
                // No leaves at all: a brush entity, always drawn.
                FaceRange { start: 48, count: 12, leaves: vec![] },
            ],
        };
        let visible = vec![true, true, false, true, false];
        assert_eq!(Map::draw_ranges(&batch, &visible), vec![(0, 24), (36, 24)]);
    }
}
