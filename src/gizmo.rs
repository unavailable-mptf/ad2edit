//! The geometry behind the viewport's move and rotate handles: where along
//! an axis, or how far round a ring, the pointer's ray is. A drag is the
//! difference between that reading now and the reading when the handle was
//! grabbed, applied to where the entity was then, so it cannot jump or drift.

use crate::transform::Vec3;

fn dot(a: Vec3, b: Vec3) -> f64 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

/// How far along the line `origin + s * direction` the point nearest the
/// ray from `eye` lies. None when the ray runs along the axis, where a
/// pixel of pointer travel would be an unbounded distance.
pub fn along_axis(origin: Vec3, direction: Vec3, eye: Vec3, ray: Vec3) -> Option<f64> {
    let between = sub(origin, eye);
    let (a, b, c) = (dot(direction, direction), dot(direction, ray), dot(ray, ray));
    let (d, e) = (dot(direction, between), dot(ray, between));
    let spread = a * c - b * b;
    if spread < 0.02 * a * c {
        return None;
    }
    Some((b * e - c * d) / spread)
}

/// The angle, in the plane through `origin` spanned by `u` and `w`, of the
/// point where the ray from `eye` meets that plane. `u x w` is the rotation
/// axis, so the angle grows the right-handed way round it. None when the
/// plane is seen edge-on or lies behind the eye.
pub fn angle_in_plane(origin: Vec3, axis: Vec3, u: Vec3, w: Vec3, eye: Vec3, ray: Vec3) -> Option<f64> {
    let facing = dot(ray, axis);
    if facing.abs() < 0.15 * dot(ray, ray).sqrt() {
        return None;
    }
    let distance = dot(sub(origin, eye), axis) / facing;
    if distance <= 0.0 {
        return None;
    }
    let hit = (eye.0 + ray.0 * distance, eye.1 + ray.1 * distance, eye.2 + ray.2 * distance);
    let from_origin = sub(hit, origin);
    Some(dot(from_origin, w).atan2(dot(from_origin, u)))
}

/// The signed turn from one angle to another, the short way round, so a
/// drag across the wrap point adds a few degrees rather than a full circle.
pub fn shortest_turn(from: f64, to: f64) -> f64 {
    let turn = (to - from) % std::f64::consts::TAU;
    if turn > std::f64::consts::PI {
        turn - std::f64::consts::TAU
    } else if turn < -std::f64::consts::PI {
        turn + std::f64::consts::TAU
    } else {
        turn
    }
}

/// `value` rounded to the nearest multiple of `step`.
pub fn snapped(value: f64, step: f64) -> f64 {
    if step > 0.0 { (value / step).round() * step } else { value }
}

/// `point` carried round `pivot` by `radians` about the unit `axis`: where
/// each of several parts goes when they are turned together as one.
pub fn turned_about(point: Vec3, pivot: Vec3, axis: Vec3, radians: f64) -> Vec3 {
    let arm = sub(point, pivot);
    let (cos, sin) = (radians.cos(), radians.sin());
    let across = (axis.1 * arm.2 - axis.2 * arm.1, axis.2 * arm.0 - axis.0 * arm.2, axis.0 * arm.1 - axis.1 * arm.0);
    let along = dot(axis, arm) * (1.0 - cos);
    (
        pivot.0 + arm.0 * cos + across.0 * sin + axis.0 * along,
        pivot.1 + arm.1 * cos + across.1 * sin + axis.1 * along,
        pivot.2 + arm.2 * cos + across.2 * sin + axis.2 * along,
    )
}

/// A turn as the handle shows it: a full circle either way reads 0 again,
/// since the part is back where it began.
pub fn shown_turn(turn: f64) -> f64 {
    let shown = turn % std::f64::consts::TAU;
    if (shown.abs() - std::f64::consts::TAU).abs() < 1e-9 { 0.0 } else { shown }
}

/// Where a drag of `moved` from `start` ends when the end, not the distance,
/// lands on a multiple of `step`: a part at 10.3 dragged with a step of 0.5
/// stops at 10.5 and 11, not 10.8.
pub fn snapped_travel(start: f64, moved: f64, step: f64) -> f64 {
    snapped(start + moved, step) - start
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_ray_at_a_point_on_the_axis_reads_that_point() {
        let eye = (0.0, -100.0, 50.0);
        for s in [-40.0, 0.0, 12.5, 90.0] {
            let target = (10.0 + s, 5.0, 0.0);
            let ray = sub(target, eye);
            let read = along_axis((10.0, 5.0, 0.0), (1.0, 0.0, 0.0), eye, ray).unwrap();
            assert!(close(read, s), "{read} for {s}");
        }
    }

    #[test]
    fn a_drag_is_the_difference_between_two_readings_so_a_grab_moves_nothing() {
        let (origin, direction, eye) = ((0.0, 0.0, 0.0), (0.0, 1.0, 0.0), (80.0, -30.0, 40.0));
        let grabbed_at = (0.0, 37.0, 0.0);
        let first = along_axis(origin, direction, eye, sub(grabbed_at, eye)).unwrap();
        let same = along_axis(origin, direction, eye, sub(grabbed_at, eye)).unwrap();
        assert!(close(same - first, 0.0));
        let later = along_axis(origin, direction, eye, sub((0.0, 49.0, 0.0), eye)).unwrap();
        assert!(close(later - first, 12.0));
    }

    #[test]
    fn looking_down_the_axis_gives_no_reading() {
        let eye = (-200.0, 0.0, 0.0);
        assert!(along_axis((0.0, 0.0, 0.0), (1.0, 0.0, 0.0), eye, (1.0, 0.001, 0.0)).is_none());
    }

    #[test]
    fn the_ring_angle_is_right_handed_about_its_axis() {
        let (axis, u, w) = ((0.0, 0.0, 1.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0));
        let eye = (0.0, 0.0, 300.0);
        let at = |point: Vec3| angle_in_plane((0.0, 0.0, 0.0), axis, u, w, eye, sub(point, eye)).unwrap();
        assert!(close(at((50.0, 0.0, 0.0)), 0.0));
        assert!(close(at((0.0, 50.0, 0.0)), FRAC_PI_2));
        assert!(close(at((-50.0, 0.0, 0.0)).abs(), PI));
        assert!(angle_in_plane((0.0, 0.0, 0.0), axis, u, w, (300.0, 0.0, 1.0), (-1.0, 0.0, 0.0)).is_none());
        assert!(angle_in_plane((0.0, 0.0, 0.0), axis, u, w, eye, (0.0, 0.0, 1.0)).is_none());
    }

    #[test]
    fn turns_take_the_short_way_and_snap_to_a_step() {
        assert!(close(shortest_turn(3.0, -3.0), std::f64::consts::TAU - 6.0));
        assert!(close(shortest_turn(-3.0, 3.0), 6.0 - std::f64::consts::TAU));
        assert!(close(shortest_turn(0.2, 0.5), 0.3));
        assert!(close(snapped(37.4, 15.0), 30.0));
        assert!(close(snapped(-38.0, 15.0), -45.0));
        assert!(close(snapped(7.3, 0.0), 7.3));
    }
    #[test]
    fn parts_turned_together_go_round_the_pivot_and_keep_their_distance() {
        let pivot = (10.0, 5.0, 0.0);
        let moved = turned_about((20.0, 5.0, 3.0), pivot, (0.0, 0.0, 1.0), FRAC_PI_2);
        assert!(close(moved.0, 10.0) && close(moved.1, 15.0) && close(moved.2, 3.0), "{moved:?}");
        let back = turned_about(moved, pivot, (0.0, 0.0, 1.0), -FRAC_PI_2);
        assert!(close(back.0, 20.0) && close(back.1, 5.0));
        let on_the_axis = turned_about((10.0, 5.0, 40.0), pivot, (0.0, 0.0, 1.0), PI);
        assert!(close(on_the_axis.0, 10.0) && close(on_the_axis.1, 5.0) && close(on_the_axis.2, 40.0));
    }

    #[test]
    fn a_full_circle_either_way_reads_zero_again() {
        assert!(close(shown_turn(370f64.to_radians()), 10f64.to_radians()));
        assert!(close(shown_turn(-370f64.to_radians()), -10f64.to_radians()));
        assert!(close(shown_turn(std::f64::consts::TAU), 0.0));
        assert!(close(shown_turn(-std::f64::consts::TAU), 0.0));
        assert!(close(shown_turn(3.0 * std::f64::consts::TAU + 0.25), 0.25));
        assert!(close(shown_turn(-1.0), -1.0));
    }

    #[test]
    fn a_snapped_drag_lands_on_the_grid_wherever_it_began() {
        assert!(close(snapped_travel(10.3, 0.1, 0.5), 0.2));
        assert!(close(snapped_travel(10.3, 0.6, 0.5), 0.7));
        assert!(close(snapped_travel(-4.0, -0.74, 0.5), -0.5));
        assert!(close(snapped_travel(10.3, 0.6, 0.0), 0.6));
    }
}
