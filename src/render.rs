//! A picture of a dupe without the editor: the same models, sizes,
//! materials and hidden parts the viewport uses, drawn by the CPU
//! rasteriser and written as a PNG. Each triangle takes one colour, its
//! texture sampled at its middle, so this shows what is where and what it
//! wears rather than fine texture detail.

use crate::dupe::Dupe;
use crate::look;
use crate::models::{ModelLibrary, Texture};
use crate::raster::{self, ScreenVertex, Target};
use crate::scale;
use crate::transform::{self, Vec3};

pub struct Picture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn sub(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn dot(a: Vec3, b: Vec3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}
fn unit(a: Vec3) -> Vec3 {
    let length = dot(a, a).sqrt();
    if length > 1e-9 { (a.0 / length, a.1 / length, a.2 / length) } else { (0.0, 0.0, 1.0) }
}

fn sample(texture: &Texture, u: f32, v: f32) -> [f32; 4] {
    let wrap = |t: f32, size: u32| (((t.fract() + 1.0).fract() * size as f32) as u32).min(size.saturating_sub(1));
    let at = ((wrap(v, texture.height) * texture.width + wrap(u, texture.width)) * 4) as usize;
    match texture.rgba.get(at..at + 4) {
        Some(px) => [px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0, px[3] as f32 / 255.0],
        None => [0.55, 0.55, 0.56, 1.0],
    }
}

/// A texture's colour taken as a whole. A triangle that spans much of a
/// texture gets this instead of the one texel under its middle, which on a
/// plate with a grid on it is as likely a black line as not.
fn overall(texture: &Texture) -> [f32; 4] {
    // Every texel: a stride would fall in step with the very grid lines
    // this is here to average out.
    let (mut sum, mut count) = ([0.0f32; 4], 0.0f32);
    for texel in texture.rgba.chunks_exact(4) {
        for (total, value) in sum.iter_mut().zip(texel) {
            *total += *value as f32 / 255.0;
        }
        count += 1.0;
    }
    if count == 0.0 { [0.55, 0.55, 0.56, 1.0] } else { sum.map(|total| total / count) }
}

/// The dupe seen from `yaw` and `pitch` degrees round its middle, far
/// enough back to fit, every colour multiplied by `brightness`. Returns the
/// picture and how many entities had no mesh and were drawn as nothing.
pub fn picture(dupe: &Dupe, lib: &mut ModelLibrary, width: usize, height: usize, yaw: f64, pitch: f64, brightness: f32) -> (Picture, usize) {
    let entities: Vec<(f64, Vec3, Vec3, String)> = dupe
        .list_entities()
        .into_iter()
        .filter_map(|(index, _, model)| {
            let (at, ang) = transform::entity_transform(dupe, index)?;
            Some((index, at, ang, model.trim_matches('"').to_owned()))
        })
        .collect();
    let (mut low, mut high) = ((f64::MAX, f64::MAX, f64::MAX), (f64::MIN, f64::MIN, f64::MIN));
    for (_, at, _, _) in &entities {
        low = (low.0.min(at.0), low.1.min(at.1), low.2.min(at.2));
        high = (high.0.max(at.0), high.1.max(at.1), high.2.max(at.2));
    }
    if entities.is_empty() {
        (low, high) = ((0.0, 0.0, 0.0), (0.0, 0.0, 0.0));
    }
    let middle = ((low.0 + high.0) / 2.0, (low.1 + high.1) / 2.0, (low.2 + high.2) / 2.0);
    let reach = dot(sub(high, low), sub(high, low)).sqrt() / 2.0 + 48.0;
    let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
    let forward = unit((-yaw.cos() * pitch.cos(), -yaw.sin() * pitch.cos(), -pitch.sin()));
    let eye = sub(middle, (forward.0 * reach * 1.9, forward.1 * reach * 1.9, forward.2 * reach * 1.9));
    let right = unit(cross(forward, (0.0, 0.0, 1.0)));
    let up = cross(right, forward);
    let focal = (height as f64 * 0.5) / (30.0f64.to_radians()).tan();
    let project = |point: Vec3| -> Option<ScreenVertex> {
        let from_eye = sub(point, eye);
        let depth = dot(from_eye, forward);
        if depth < 1.0 {
            return None;
        }
        let k = focal / depth;
        Some(ScreenVertex {
            x: (width as f64 * 0.5 + dot(from_eye, right) * k) as f32,
            y: (height as f64 * 0.5 - dot(from_eye, up) * k) as f32,
            inv_z: (1.0 / depth) as f32,
        })
    };

    let mut target = Target::new(width, height);
    let backdrop = [58u8, 62, 70];
    target.clear([backdrop[0], backdrop[1], backdrop[2], 255]);
    let light = unit((0.35, 0.5, 1.0));
    let mut without_mesh = 0;
    let mut overall_colours: std::collections::HashMap<String, [f32; 4]> = std::collections::HashMap::new();
    for (index, at, ang, model) in &entities {
        let appearance = look::of_entity(dupe, *index);
        let material_hides = appearance.material.as_ref().is_some_and(|material| {
            let path = format!("materials/{}.vmt", material.to_lowercase().replace('\\', "/"));
            lib.read_file(&path).is_some_and(|(bytes, _)| look::material_hides(&String::from_utf8_lossy(&bytes)))
        });
        if appearance.hidden.is_some() || material_hides {
            continue;
        }
        let model = &lib.model_of(dupe, *index, model);
        let Some(mesh) = lib.mesh_as(model, appearance.skin, &appearance.bodygroups) else {
            without_mesh += 1;
            continue;
        };
        let half = lib.bounds(model).map(|(bounds, _)| bounds.half_extents()).unwrap_or((6.0, 6.0, 6.0));
        let size = scale::model_scale(dupe, *index, half);
        let axes = [
            transform::rotate_vec(*ang, (1.0, 0.0, 0.0)),
            transform::rotate_vec(*ang, (0.0, 1.0, 0.0)),
            transform::rotate_vec(*ang, (0.0, 0.0, 1.0)),
        ];
        let place = |local: [f32; 3]| -> Vec3 {
            let (x, y, z) = (local[0] as f64 * size.0, local[1] as f64 * size.1, local[2] as f64 * size.2);
            add(*at, (
                axes[0].0 * x + axes[1].0 * y + axes[2].0 * z,
                axes[0].1 * x + axes[1].1 * y + axes[2].1 * z,
                axes[0].2 * x + axes[1].2 * y + axes[2].2 * z,
            ))
        };
        let tint = appearance.colour.map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]).unwrap_or([1.0; 3]);
        let opacity = appearance.colour.map_or(1.0, |c| c[3] as f32 / 255.0);
        let worn = appearance.material.as_ref().and_then(|material| lib.material_texture(material, &|_| None));
        let worn_vmt = appearance.material.as_ref().and_then(|material| lib.material_vmt(material));
        for (section, (first, count, key)) in mesh.sections.iter().enumerate() {
            // The SubMaterial tool replaces one slot; the Material tool, or
            // ACF's spawn material, replaces them all.
            let slot = mesh.section_slots.get(section).copied();
            let swapped = appearance.submaterials.iter().find(|(wanted, _)| Some(*wanted) == slot).map(|(_, material)| material.clone());
            let (texture, vmt) = match &swapped {
                Some(material) => (lib.material_texture(material, &|_| None), lib.material_vmt(material)),
                None => (worn.clone().or_else(|| lib.section_texture(key)), worn_vmt.clone().or_else(|| lib.section_vmt(key))),
            };
            let alpha_use = vmt.as_deref().map_or(look::AlphaUse::Opaque, look::alpha_use);
            for corner in (*first as usize..(*first + *count) as usize).step_by(3) {
                let Some(ids) = mesh.indices.get(corner..corner + 3) else { break };
                let points = [place(mesh.positions[ids[0] as usize]), place(mesh.positions[ids[1] as usize]), place(mesh.positions[ids[2] as usize])];
                let (Some(a), Some(b), Some(c)) = (project(points[0]), project(points[1]), project(points[2])) else { continue };
                let normal = unit(cross(sub(points[1], points[0]), sub(points[2], points[0])));
                let shade = 0.35 + 0.65 * dot(normal, light).abs() as f32;
                let albedo = match &texture {
                    Some(texture) => {
                        let uv = |n: usize| mesh.uvs.get(ids[n] as usize).copied().unwrap_or([0.0, 0.0]);
                        let spread = (0..2).map(|axis| (0..3).map(|n| uv(n)[axis]).fold(f32::MIN, f32::max) - (0..3).map(|n| uv(n)[axis]).fold(f32::MAX, f32::min)).fold(0.0, f32::max);
                        if spread > 0.35 {
                            let worn_as = swapped.clone().or_else(|| appearance.material.clone()).unwrap_or_else(|| key.clone());
                            *overall_colours.entry(worn_as).or_insert_with(|| overall(texture))
                        } else {
                            sample(texture, (uv(0)[0] + uv(1)[0] + uv(2)[0]) / 3.0, (uv(0)[1] + uv(1)[1] + uv(2)[1]) / 3.0)
                        }
                    }
                    None => [0.55, 0.55, 0.56, 1.0],
                };
                // The rasteriser cannot blend, so a see-through triangle is
                // mixed with the backdrop by how solid it is: a cut-out is
                // there or not, glass and a faded part come out paler.
                let solid = opacity
                    * match alpha_use {
                        look::AlphaUse::Opaque => 1.0,
                        look::AlphaUse::Cutout(threshold) => {
                            if albedo[3] >= threshold { 1.0 } else { 0.0 }
                        }
                        look::AlphaUse::Blended => albedo[3],
                    };
                if solid < 0.04 {
                    continue;
                }
                let channel = |n: usize| {
                    let lit = (albedo[n] * tint[n] * shade * brightness * 255.0).clamp(0.0, 255.0);
                    (lit * solid + backdrop[n] as f32 * (1.0 - solid)) as u8
                };
                raster::triangle(&mut target, [a, b, c], [channel(0), channel(1), channel(2), 255]);
            }
        }
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&target.colour_at(x, y));
        }
    }
    (Picture { width, height, rgba }, without_mesh)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + *byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// The picture as a PNG. The pixel data is stored, not compressed: no
/// dependency, and a picture made to be looked at once is not worth one.
pub fn png(picture: &Picture) -> Vec<u8> {
    let mut raw = Vec::with_capacity(picture.rgba.len() + picture.height);
    for row in picture.rgba.chunks_exact(picture.width * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut stream = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65535).peekable();
    while let Some(block) = blocks.next() {
        stream.push(if blocks.peek().is_none() { 1 } else { 0 });
        stream.extend_from_slice(&(block.len() as u16).to_le_bytes());
        stream.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        stream.extend_from_slice(block);
    }
    stream.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut header = Vec::new();
    header.extend_from_slice(&(picture.width as u32).to_be_bytes());
    header.extend_from_slice(&(picture.height as u32).to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(b"IDAT", &stream);
    chunk(b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_checksums_match_the_published_test_vectors() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_png_has_the_signature_the_size_and_every_pixel_stored() {
        let picture = Picture { width: 3, height: 2, rgba: (0..24).collect() };
        let bytes = png(&picture);
        assert_eq!(&bytes[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&bytes[12..16], b"IHDR");
        assert_eq!(&bytes[16..24], &[0, 0, 0, 3, 0, 0, 0, 2]);
        assert_eq!(&bytes[bytes.len() - 8..bytes.len() - 4], b"IEND");
        let stored = 2 * (1 + 3 * 4);
        let at = bytes.windows(4).position(|w| w == b"IDAT").unwrap() + 4;
        assert_eq!(&bytes[at..at + 2], &[0x78, 0x01]);
        assert_eq!(bytes[at + 2], 1, "one final stored block");
        assert_eq!(u16::from_le_bytes([bytes[at + 3], bytes[at + 4]]) as usize, stored);
    }
}
