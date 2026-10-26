//! Pointer mapping (IN-11).
//!
//! All coordinates are local desktop logical coordinates: the origin is at the
//! top-left, x increases to the right, y increases downward, and negative values
//! are permitted (`docs/config.md`). The platform backends convert OS coordinates.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A position in a rectangle: 0.0 is the first logical pixel and 1.0 is the last
/// logical pixel on each axis.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Normalized {
    pub x: f32,
    pub y: f32,
}

impl Rect {
    /// True if the point is in the rectangle. The right and bottom edges are not in it.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }

    /// The nearest point in the rectangle: x in `[x, x + width - 1]`, and the same for y.
    pub fn clamp(&self, p: Point) -> Point {
        Point {
            x: p.x.clamp(self.x, self.x + span(self.width)),
            y: p.y.clamp(self.y, self.y + span(self.height)),
        }
    }
}

fn span(length: f64) -> f64 {
    (length - 1.0).max(0.0)
}

fn fraction(offset: f64, length: f64) -> f32 {
    let span = span(length);
    if span <= 0.0 {
        0.0
    } else {
        (offset / span).clamp(0.0, 1.0) as f32
    }
}

/// Normalizes a point to an area. A point out of the area goes to the nearest edge.
pub fn normalize(area: Rect, p: Point) -> Normalized {
    Normalized {
        x: fraction(p.x - area.x, area.width),
        y: fraction(p.y - area.y, area.height),
    }
}

/// Maps a normalized position to a point in an area. The result is always in the
/// area (IN-11: the pointer must not leave the input-target display).
pub fn denormalize(area: Rect, n: Normalized) -> Point {
    let nx = f64::from(n.x.clamp(0.0, 1.0));
    let ny = f64::from(n.y.clamp(0.0, 1.0));
    Point {
        x: area.x + nx * span(area.width),
        y: area.y + ny * span(area.height),
    }
}
