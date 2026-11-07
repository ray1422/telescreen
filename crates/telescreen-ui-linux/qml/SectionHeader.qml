// The title above a card (FormHeader), with optional trailing items.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

RowLayout {
    id: root

    property alias text: title.text
    default property alias trailing: root.data

    Layout.fillWidth: true
    Layout.topMargin: Kirigami.Units.largeSpacing
    Layout.leftMargin: Kirigami.Units.largeSpacing
    Layout.rightMargin: Kirigami.Units.smallSpacing / 2
    Layout.bottomMargin: Kirigami.Units.smallSpacing
    // The height of a tool button, so that headers with and without one align.
    Layout.minimumHeight: Kirigami.Units.gridUnit * 2
    spacing: Kirigami.Units.smallSpacing

    QQC2.Label {
        id: title
        Layout.fillWidth: true
        font.weight: Font.DemiBold
        elide: Text.ElideRight
        Accessible.role: Accessible.Heading
    }
}
