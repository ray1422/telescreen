// A rounded rectangle with continuous (G2) corners: each corner is a quadrant
// of a superellipse, as in the app icon. Use it for all cards and fields
// instead of Rectangle.radius, which draws circular (G1) arcs (plan, M20 notes).
import QtQuick
import QtQuick.Shapes

Shape {
    id: root

    property real radius: 6
    property color color: "transparent"
    property color borderColor: "transparent"
    property real borderWidth: 0
    // The superellipse exponent. 2 is a circle; larger values are more square.
    property real exponent: 4

    // The curve renderer (Qt 6.6) antialiases the outline. Qt 6.4 has no such
    // property, and a binding to an unknown property stops the load.
    Component.onCompleted: if (root.preferredRendererType !== undefined)
        root.preferredRendererType = Shape.CurveRenderer

    function outline(w, h, r, p, inset) {
        const x = inset, y = inset
        w -= 2 * inset
        h -= 2 * inset
        if (w <= 0 || h <= 0)
            return ""
        const e = Math.min(r * 1.7, w / 2, h / 2)
        const pts = []
        function corner(cx, cy, sx, sy, fromTop) {
            for (let i = 0; i <= 8; i++) {
                const t = (fromTop ? 8 - i : i) / 8 * Math.PI / 2
                pts.push((cx + sx * e * Math.pow(Math.cos(t), 2 / p)).toFixed(2) + " "
                         + (cy + sy * e * Math.pow(Math.sin(t), 2 / p)).toFixed(2))
            }
        }
        corner(x + w - e, y + e, 1, -1, true)
        corner(x + w - e, y + h - e, 1, 1, false)
        corner(x + e, y + h - e, -1, 1, true)
        corner(x + e, y + e, -1, -1, false)
        return "M" + pts.join("L") + "Z"
    }

    ShapePath {
        fillColor: root.color
        strokeColor: root.borderWidth > 0 ? root.borderColor : "transparent"
        strokeWidth: root.borderWidth > 0 ? root.borderWidth : -1
        PathSvg {
            path: root.outline(root.width, root.height, root.radius, root.exponent, root.borderWidth / 2)
        }
    }
}
