// A card of rows, as a Kirigami Addons FormCard, with G2 corners. The rows
// draw their own separators.
import QtQuick
import QtQuick.Layouts
import org.kde.kirigami as Kirigami

Item {
    id: root

    default property alias rows: column.data
    // Kirigami.Units.cornerRadius is from recent Kirigami versions; older versions have none.
    readonly property real radius: Kirigami.Units.cornerRadius ?? Kirigami.Units.smallSpacing

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight

    Kirigami.Theme.colorSet: Kirigami.Theme.View
    Kirigami.Theme.inherit: false

    G2Rect {
        anchors.fill: parent
        anchors.margins: -1
        radius: root.radius + 1
        color: Kirigami.ColorUtils.linearInterpolation(Kirigami.Theme.backgroundColor, Kirigami.Theme.textColor, 0.12)
    }
    G2Rect {
        anchors.fill: parent
        radius: root.radius
        color: Kirigami.Theme.backgroundColor
    }
    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: 0
    }
}
