// One candidate of a display (UI-18): the computer, its status, the primary
// action as a push button, and the other actions in an overflow menu. A
// right-click on the row opens the same menu.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property var candidate
    required property string display
    required property string device
    // The "Let <peer> control this display" row of this peer and device, or null.
    property var controlRequest: null
    property bool separator: false
    signal intent(var intent)

    readonly property var actions: [
        { kind: "switch_and_control", text: qsTr("Switch & Control"), reason: candidate.switch_and_control },
        { kind: "switch_display_only", text: qsTr("Switch Display Only"), reason: candidate.switch_display_only },
        { kind: "control_only", text: qsTr("Control Only"), reason: candidate.control_only },
    ]
    readonly property var primary: actions.find(a => a.reason === null) ?? null
    readonly property bool canLetControl: controlRequest !== null && !controlRequest.waiting
    readonly property int available: actions.filter(a => a.reason === null).length + (canLetControl ? 1 : 0)
    readonly property bool online: candidate.status === "online"

    function statusText() {
        const status = {
            online: qsTr("Online"),
            connecting: qsTr("Connecting…"),
            searching: qsTr("Searching…"),
            offline: qsTr("Offline"),
        }[candidate.status] ?? candidate.status
        if (controlRequest !== null && controlRequest.waiting)
            return qsTr("%1 · Waiting for %2…").arg(status).arg(candidate.label)
        // With no action, the subtitle gives the reason of "Switch & control".
        if (primary === null && candidate.switch_and_control)
            return status + " · " + candidate.switch_and_control
        return status
    }

    function run(kind) {
        if (kind === "let_control")
            intent({ kind: kind, peer: candidate.target, device: device })
        else
            intent({ kind: kind, display: display, device: device, target: candidate.target })
    }

    Layout.fillWidth: true
    spacing: 0

    Kirigami.Separator {
        visible: root.separator
        Layout.fillWidth: true
        Layout.leftMargin: Kirigami.Units.largeSpacing * 2 + Kirigami.Units.iconSizes.smallMedium
        Layout.rightMargin: Kirigami.Units.largeSpacing
    }
    RowLayout {
        Layout.fillWidth: true
        Layout.minimumHeight: Kirigami.Units.gridUnit * 3
        Layout.leftMargin: Kirigami.Units.largeSpacing
        Layout.rightMargin: Kirigami.Units.smallSpacing
        spacing: Kirigami.Units.largeSpacing

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: if (root.available > 0) menu.popup()
        }

        Kirigami.Icon {
            source: "computer-symbolic"
            implicitWidth: Kirigami.Units.iconSizes.smallMedium
            implicitHeight: implicitWidth
            opacity: root.online ? 1 : 0.6
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            QQC2.Label {
                Layout.fillWidth: true
                text: root.candidate.label
                color: root.online ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
            }
            QQC2.Label {
                Layout.fillWidth: true
                text: root.statusText()
                color: root.online && root.primary !== null ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                font: Kirigami.Theme.smallFont
                wrapMode: Text.Wrap
            }
        }
        QQC2.Button {
            visible: root.primary !== null
            text: root.primary?.text ?? ""
            onClicked: root.run(root.primary.kind)
        }
        QQC2.ToolButton {
            // No overflow menu when it has only one available item (UI-18).
            opacity: root.available > 1 ? 1 : 0
            enabled: root.available > 1
            icon.name: "overflow-menu"
            display: QQC2.AbstractButton.IconOnly
            text: qsTr("More Actions for %1").arg(root.candidate.label)
            down: menu.visible
            onPressed: menu.popup(this, 0, height)
            QQC2.ToolTip.text: qsTr("More actions")
            QQC2.ToolTip.visible: hovered && !menu.visible
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        }
    }

    QQC2.Menu {
        id: menu

        Repeater {
            model: root.actions
            delegate: QQC2.MenuItem {
                required property var modelData
                enabled: modelData.reason === null
                text: modelData.text
                onTriggered: root.run(modelData.kind)
                // An unavailable action shows its reason (UI-9).
                contentItem: ColumnLayout {
                    spacing: 0
                    QQC2.Label {
                        Layout.fillWidth: true
                        leftPadding: Kirigami.Units.largeSpacing
                        text: modelData.text
                        color: enabled ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
                    }
                    QQC2.Label {
                        Layout.fillWidth: true
                        leftPadding: Kirigami.Units.largeSpacing
                        visible: modelData.reason !== null
                        text: modelData.reason ?? ""
                        color: Kirigami.Theme.disabledTextColor
                        font: Kirigami.Theme.smallFont
                    }
                }
            }
        }
        QQC2.MenuSeparator {
            visible: root.controlRequest !== null
            height: visible ? implicitHeight : 0
        }
        QQC2.MenuItem {
            visible: root.controlRequest !== null
            height: visible ? implicitHeight : 0
            enabled: root.canLetControl
            text: root.controlRequest?.waiting
                ? qsTr("Waiting for %1…").arg(root.candidate.label)
                : qsTr("Let %1 Control This Display").arg(root.candidate.label)
            onTriggered: root.run("let_control")
        }
    }
}
