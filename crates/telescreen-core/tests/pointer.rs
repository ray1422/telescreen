//! IN-11 pointer mapping, FLOW-14 and FLOW-15 edges, AC-12.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::pointer::{Motion, Normalized, Point, Rect, denormalize, normalize, step};

const PORTAL: Rect = Rect {
    x: 0.0,
    y: 0.0,
    width: 3412.0,
    height: 1440.0,
};

fn p(x: f64, y: f64) -> Point {
    Point { x, y }
}

#[test]
fn ac12_corners_map_to_corners() {
    // Linux output: logical 2048x864 at a negative origin.
    let target = Rect {
        x: -2048.0,
        y: -100.0,
        width: 2048.0,
        height: 864.0,
    };
    let tl = denormalize(target, normalize(PORTAL, p(0.0, 0.0)));
    let br = denormalize(target, normalize(PORTAL, p(3411.0, 1439.0)));
    assert_eq!(tl, p(-2048.0, -100.0));
    assert_eq!(br, p(-1.0, 763.0));
    let center = denormalize(target, Normalized { x: 0.5, y: 0.5 });
    assert!((center.x - (-2048.0 + 2047.0 / 2.0)).abs() < 1e-3);
}

#[test]
fn ac12_out_of_bounds_is_clamped() {
    assert_eq!(
        normalize(PORTAL, p(-50.0, 5000.0)),
        Normalized { x: 0.0, y: 1.0 }
    );
    let target = Rect {
        x: 10.0,
        y: 10.0,
        width: 100.0,
        height: 50.0,
    };
    assert_eq!(
        denormalize(target, Normalized { x: 2.0, y: -1.0 }),
        p(109.0, 10.0)
    );
}

#[test]
fn degenerate_area() {
    let line = Rect {
        x: 5.0,
        y: 5.0,
        width: 1.0,
        height: 0.0,
    };
    assert_eq!(normalize(line, p(9.0, 9.0)), Normalized { x: 0.0, y: 0.0 });
    assert_eq!(
        denormalize(line, Normalized { x: 1.0, y: 1.0 }),
        p(5.0, 5.0)
    );
}

#[test]
fn flow14_exit_into_adjacent_display() {
    // The built-in display is to the left of the portal, with a different height.
    let builtin = Rect {
        x: -1512.0,
        y: 200.0,
        width: 1512.0,
        height: 982.0,
    };
    assert_eq!(
        step(PORTAL, &[builtin], p(2.0, 500.0), -5.0, 0.0),
        Motion::Exit(p(-3.0, 500.0))
    );
}

#[test]
fn flow15_edges_without_display_stop_the_pointer() {
    let builtin = Rect {
        x: -1512.0,
        y: 200.0,
        width: 1512.0,
        height: 982.0,
    };
    // Left edge above the built-in display: no display there.
    assert_eq!(
        step(PORTAL, &[builtin], p(2.0, 50.0), -5.0, 0.0),
        Motion::Inside(p(0.0, 50.0))
    );
    // Top, right, and bottom edges.
    assert_eq!(
        step(PORTAL, &[builtin], p(10.0, 3.0), 0.0, -9.0),
        Motion::Inside(p(10.0, 0.0))
    );
    assert_eq!(
        step(PORTAL, &[builtin], p(3410.0, 9.0), 9.0, 0.0),
        Motion::Inside(p(3411.0, 9.0))
    );
    assert_eq!(
        step(PORTAL, &[builtin], p(9.0, 1439.0), 0.0, 3.5),
        Motion::Inside(p(9.0, 1439.0))
    );
}

#[test]
fn inside_motion() {
    assert_eq!(
        step(PORTAL, &[], p(100.0, 100.0), 1.5, -2.5),
        Motion::Inside(p(101.5, 97.5))
    );
}
