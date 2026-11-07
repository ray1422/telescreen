// A row with a switch (FormSwitchDelegate). A click on the row toggles it.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

QQC2.ItemDelegate {
    id: root

    property string subtitle
    property bool separator: false
    // The switch shows the model value; a click only asks for a change.
    property bool on: false
    signal toggle(bool on)

    Layout.fillWidth: true
    padding: 0
    onClicked: root.toggle(!root.on)
    Accessible.role: Accessible.CheckBox
    Accessible.checked: root.on

    contentItem: ColumnLayout {
        spacing: 0
        Kirigami.Separator {
            visible: root.separator
            Layout.fillWidth: true
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.minimumHeight: root.subtitle ? Kirigami.Units.gridUnit * 3 : Kirigami.Units.gridUnit * 2.25
            Layout.leftMargin: Kirigami.Units.largeSpacing
            Layout.rightMargin: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                QQC2.Label {
                    Layout.fillWidth: true
                    text: root.text
                    elide: Text.ElideRight
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    visible: root.subtitle !== ""
                    text: root.subtitle
                    color: Kirigami.Theme.disabledTextColor
                    font: Kirigami.Theme.smallFont
                    wrapMode: Text.Wrap
                }
            }
            QQC2.Switch {
                checked: root.on
                onToggled: {
                    checked = Qt.binding(() => root.on)
                    root.toggle(!root.on)
                }
                Accessible.name: root.text
            }
        }
    }
}
