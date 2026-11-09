// One candidate of a display (UI-18): the computer, its status, the primary
// action on a row click, and the other actions in an overflow menu. A
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
    readonly property var primary: actions.find(a => a.kind === candidate.primary) ?? null
    readonly property bool canLetControl: controlRequest !== null && !controlRequest.waiting
    readonly property int available: actions.filter(a => a.reason === null).length
        + (canLetControl ? 1 : 0) + (candidate.wake.kind === "available" ? 1 : 0)
    readonly property bool hasMore: available !== 1 || primary === null
    readonly property bool online: candidate.status === "online"

    function statusText() {
        const status = {
            online: qsTr("Online"),
            connecting: qsTr("Connecting…"),
            searching: qsTr("Searching…"),
            offline: qsTr("Offline"),
        }[candidate.status] ?? candidate.status
        // Rust selects the primary action. With none, show the reason.
        const detail = primary?.text ?? candidate.switch_and_control
        let text = detail ? status + " · " + detail : status
        if (controlRequest !== null && controlRequest.waiting)
            text += qsTr(" · Waiting for %1…").arg(candidate.label)
        return text
    }

    function run(kind) {
        if (kind === "let_control")
            intent({ kind: kind, peer: candidate.target, device: device })
        else if (kind === "wake")
            intent({ kind: kind, target: candidate.target })
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
            onTapped: menu.popup()
        }

        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true
            implicitHeight: content.implicitHeight

            HoverHandler { id: hover }
            TapHandler {
                acceptedButtons: Qt.LeftButton
                onTapped: if (root.primary !== null) root.run(root.primary.kind)
            }
            Rectangle {
                anchors.fill: parent
                color: hover.hovered ? Qt.alpha(Kirigami.Theme.textColor, 0.05) : "transparent"
            }
            RowLayout {
                id: content
                anchors.fill: parent
                spacing: Kirigami.Units.largeSpacing
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
                        color: Kirigami.Theme.disabledTextColor
                        font: Kirigami.Theme.smallFont
                        wrapMode: Text.Wrap
                    }
                }
            }
        }
        QQC2.ToolButton {
            // Hide only when the primary action is the sole available item.
            opacity: root.hasMore ? 1 : 0
            enabled: root.hasMore
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
            visible: root.controlRequest !== null || root.candidate.wake.kind !== "hidden"
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
        QQC2.MenuItem {
            visible: root.candidate.wake.kind !== "hidden"
            height: visible ? implicitHeight : 0
            enabled: root.candidate.wake.kind === "available"
            text: qsTr("Wake %1").arg(root.candidate.label)
            onTriggered: root.run("wake")
            contentItem: ColumnLayout {
                spacing: 0
                QQC2.Label {
                    Layout.fillWidth: true
                    leftPadding: Kirigami.Units.largeSpacing
                    text: qsTr("Wake %1").arg(root.candidate.label)
                    color: root.candidate.wake.kind === "available"
                        ? Kirigami.Theme.textColor : Kirigami.Theme.disabledTextColor
                }
                QQC2.Label {
                    Layout.fillWidth: true
                    leftPadding: Kirigami.Units.largeSpacing
                    visible: root.candidate.wake.kind === "unavailable"
                    text: root.candidate.wake.reason ?? ""
                    color: Kirigami.Theme.disabledTextColor
                    font: Kirigami.Theme.smallFont
                }
            }
        }
    }
}
