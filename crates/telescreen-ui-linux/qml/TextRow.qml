// A row with a title, an optional subtitle, a trailing text, and trailing items.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    property string text
    property string subtitle
    property color subtitleColor: Kirigami.Theme.disabledTextColor
    property string trailingText
    property color trailingColor: Kirigami.Theme.disabledTextColor
    property bool dim: false
    property bool separator: false
    property string icon
    default property alias trailing: row.data

    Layout.fillWidth: true
    spacing: 0

    Kirigami.Separator {
        visible: root.separator
        Layout.fillWidth: true
        Layout.leftMargin: root.icon !== "" ? Kirigami.Units.largeSpacing * 2 + icon.width : Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
    }
    RowLayout {
        id: row
        Layout.fillWidth: true
        Layout.minimumHeight: root.subtitle ? Kirigami.Units.gridUnit * 3 : Kirigami.Units.gridUnit * 2.25
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        Kirigami.Icon {
            id: icon
            visible: root.icon !== ""
            source: root.icon
            implicitWidth: Kirigami.Units.iconSizes.smallMedium
            implicitHeight: implicitWidth
            opacity: root.dim ? 0.6 : 1
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            QQC2.Label {
                Layout.fillWidth: true
                text: root.text
                color: root.dim ? Kirigami.Theme.disabledTextColor : Kirigami.Theme.textColor
                elide: Text.ElideRight
            }
            QQC2.Label {
                Layout.fillWidth: true
                visible: root.subtitle !== ""
                text: root.subtitle
                color: root.subtitleColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.Wrap
                textFormat: Text.StyledText
            }
        }
        QQC2.Label {
            visible: root.trailingText !== ""
            text: root.trailingText
            color: root.trailingColor
            font: Kirigami.Theme.smallFont
        }
    }
}
