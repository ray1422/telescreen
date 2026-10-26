//! IN-11 pointer mapping, FLOW-14 and FLOW-15 edges, AC-12.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::pointer::{Normalized, Point, Rect, denormalize, normalize};

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
