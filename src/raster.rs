//! A z-buffered software rasteriser.
//!
//! The viewport used painter's algorithm on whole boxes, which is exact for
//! convex shapes and useless for real model meshes — a prop's own geometry
//! self-occludes, and no draw order fixes that. This is the smallest thing
//! that solves it properly: a depth buffer.
//!
//! Deliberately not wgpu. A GPU path would mean shaders, pipelines and buffer
//! plumbing against an API version that can't be compile-checked here, for a
//! scene of a few tens of thousands of triangles. This is plain Rust, every
//! line of it testable, and it runs at a resolution the caller picks.

/// RGBA8 colour target plus its depth buffer.
pub struct Target {
    pub width: usize,
    pub height: usize,
    pub colour: Vec<u8>,
    /// Reciprocal depth. Larger is nearer, so the test is a plain `>`.
    depth: Vec<f32>,
}

impl Target {
    pub fn new(width: usize, height: usize) -> Target {
        Target {
            width: width.max(1),
            height: height.max(1),
            colour: vec![0; width.max(1) * height.max(1) * 4],
            depth: vec![f32::NEG_INFINITY; width.max(1) * height.max(1)],
        }
    }

    pub fn clear(&mut self, rgba: [u8; 4]) {
        for px in self.colour.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        self.depth.fill(f32::NEG_INFINITY);
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        let (w, h) = (width.max(1), height.max(1));
        if w != self.width || h != self.height {
            self.width = w;
            self.height = h;
            self.colour = vec![0; w * h * 4];
            self.depth = vec![f32::NEG_INFINITY; w * h];
        }
    }

    /// Depth at a pixel, for tests and for picking.
    pub fn depth_at(&self, x: usize, y: usize) -> f32 {
        self.depth[y * self.width + x]
    }

    pub fn colour_at(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [
            self.colour[i],
            self.colour[i + 1],
            self.colour[i + 2],
            self.colour[i + 3],
        ]
    }
}

/// A vertex already projected to screen space: pixel x/y, and the reciprocal
/// of view depth so it interpolates linearly across the triangle.
#[derive(Clone, Copy, Debug)]
pub struct ScreenVertex {
    pub x: f32,
    pub y: f32,
    pub inv_z: f32,
}

fn edge(ax: f32, ay: f32, bx: f32, by: f32, px: f32, py: f32) -> f32 {
    (px - ax) * (by - ay) - (py - ay) * (bx - ax)
}

/// Fills one triangle, depth-testing per pixel.
///
/// Returns how many pixels it actually wrote, which is what the tests check
/// and what tells the caller whether a frame is getting expensive.
pub fn triangle(t: &mut Target, v: [ScreenVertex; 3], rgba: [u8; 4]) -> usize {
    // Screen-space area. Zero means degenerate; the sign tells us the winding,
    // and rather than cull here we just normalise so either winding fills.
    let area = edge(v[0].x, v[0].y, v[1].x, v[1].y, v[2].x, v[2].y);
    if area.abs() < 1e-6 {
        return 0;
    }
    let flip = if area < 0.0 { -1.0 } else { 1.0 };
    let inv_area = 1.0 / (area * flip);

    let min_x = v.iter().fold(f32::MAX, |m, p| m.min(p.x)).floor().max(0.0) as usize;
    let max_x = (v.iter().fold(f32::MIN, |m, p| m.max(p.x)).ceil() as isize)
        .clamp(0, t.width as isize) as usize;
    let min_y = v.iter().fold(f32::MAX, |m, p| m.min(p.y)).floor().max(0.0) as usize;
    let max_y = (v.iter().fold(f32::MIN, |m, p| m.max(p.y)).ceil() as isize)
        .clamp(0, t.height as isize) as usize;
    if min_x >= max_x || min_y >= max_y {
        return 0;
    }

    let mut written = 0;
    for y in min_y..max_y {
        let py = y as f32 + 0.5;
        for x in min_x..max_x {
            let px = x as f32 + 0.5;

            let w0 = edge(v[1].x, v[1].y, v[2].x, v[2].y, px, py) * flip;
            let w1 = edge(v[2].x, v[2].y, v[0].x, v[0].y, px, py) * flip;
            let w2 = edge(v[0].x, v[0].y, v[1].x, v[1].y, px, py) * flip;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }

            // Barycentric interpolation of 1/z, which is linear in screen
            // space where z itself is not.
            let z = (w0 * v[0].inv_z + w1 * v[1].inv_z + w2 * v[2].inv_z) * inv_area;

            let i = y * t.width + x;
            if z > t.depth[i] {
                t.depth[i] = z;
                let o = i * 4;
                t.colour[o..o + 4].copy_from_slice(&rgba);
                written += 1;
            }
        }
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f32, y: f32, inv_z: f32) -> ScreenVertex {
        ScreenVertex { x, y, inv_z }
    }

    #[test]
    fn a_triangle_covers_pixels_inside_it_only() {
        let mut t = Target::new(8, 8);
        t.clear([0, 0, 0, 255]);
        // Right triangle over the top-left corner.
        triangle(&mut t, [v(0.0, 0.0, 1.0), v(8.0, 0.0, 1.0), v(0.0, 8.0, 1.0)], [255, 0, 0, 255]);
        assert_eq!(t.colour_at(1, 1), [255, 0, 0, 255], "inside");
        assert_eq!(t.colour_at(7, 7), [0, 0, 0, 255], "outside the hypotenuse");
    }

    #[test]
    fn winding_does_not_matter() {
        let mut a = Target::new(8, 8);
        let mut b = Target::new(8, 8);
        a.clear([0, 0, 0, 255]);
        b.clear([0, 0, 0, 255]);
        let tri = [v(0.0, 0.0, 1.0), v(8.0, 0.0, 1.0), v(0.0, 8.0, 1.0)];
        let flipped = [tri[0], tri[2], tri[1]];
        let n1 = triangle(&mut a, tri, [1, 2, 3, 255]);
        let n2 = triangle(&mut b, flipped, [1, 2, 3, 255]);
        assert_eq!(n1, n2);
        assert_eq!(a.colour_at(1, 1), b.colour_at(1, 1));
    }

    #[test]
    fn the_nearer_triangle_wins_whatever_the_draw_order() {
        let far = [v(0.0, 0.0, 0.10), v(8.0, 0.0, 0.10), v(0.0, 8.0, 0.10)];
        let near = [v(0.0, 0.0, 0.90), v(8.0, 0.0, 0.90), v(0.0, 8.0, 0.90)];

        // Near first, then far: the far one must not overwrite it.
        let mut t = Target::new(8, 8);
        t.clear([0, 0, 0, 255]);
        triangle(&mut t, near, [10, 10, 10, 255]);
        let overdrawn = triangle(&mut t, far, [90, 90, 90, 255]);
        assert_eq!(overdrawn, 0, "a farther triangle wrote pixels");
        assert_eq!(t.colour_at(1, 1), [10, 10, 10, 255]);

        // And the other order gives the same picture, which is the whole point
        // of having a depth buffer rather than sorting.
        let mut u = Target::new(8, 8);
        u.clear([0, 0, 0, 255]);
        triangle(&mut u, far, [90, 90, 90, 255]);
        triangle(&mut u, near, [10, 10, 10, 255]);
        assert_eq!(u.colour_at(1, 1), [10, 10, 10, 255]);
    }

    #[test]
    fn degenerate_triangles_draw_nothing() {
        let mut t = Target::new(8, 8);
        t.clear([0, 0, 0, 255]);
        assert_eq!(
            triangle(&mut t, [v(1.0, 1.0, 1.0), v(1.0, 1.0, 1.0), v(1.0, 1.0, 1.0)], [1, 1, 1, 255]),
            0
        );
        assert_eq!(
            triangle(&mut t, [v(0.0, 0.0, 1.0), v(4.0, 0.0, 1.0), v(8.0, 0.0, 1.0)], [1, 1, 1, 255]),
            0,
            "collinear"
        );
    }

    #[test]
    fn geometry_off_screen_is_clipped_not_panicking() {
        let mut t = Target::new(8, 8);
        t.clear([0, 0, 0, 255]);
        // Entirely to the left, entirely below, and straddling the edge.
        triangle(&mut t, [v(-50.0, -50.0, 1.0), v(-10.0, -50.0, 1.0), v(-50.0, -10.0, 1.0)], [9, 9, 9, 255]);
        triangle(&mut t, [v(100.0, 100.0, 1.0), v(140.0, 100.0, 1.0), v(100.0, 140.0, 1.0)], [9, 9, 9, 255]);
        let n = triangle(&mut t, [v(-4.0, -4.0, 1.0), v(20.0, -4.0, 1.0), v(-4.0, 20.0, 1.0)], [7, 7, 7, 255]);
        assert!(n > 0, "the straddling triangle should still fill");
        assert_eq!(t.colour_at(0, 0), [7, 7, 7, 255]);
    }
}
