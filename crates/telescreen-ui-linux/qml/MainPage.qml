// The main window content (UI-13): a header in the Header color set, which is
// one surface with the KWin title bar, then the displays on the left and this
// computer, the computers, and the devices on the right.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

Item {
    id: root

    // A `MainScreen` value (telescreen-view), or null before the first one.
    property var screen: null
    signal intent(var intent)
    signal openSettings()
    signal quit()

    readonly property var menu: screen?.menu ?? null

    function sessionText() {
        if (!menu || menu.sessions.length === 0)
            return qsTr("No session")
        const s = menu.sessions[0]
        return s.role.kind === "source" ? qsTr("Controlling %1").arg(s.peer_label)
                                        : qsTr("Controlled by %1").arg(s.peer_label)
    }

    function deviceLabel(id) {
        for (const d of menu?.displays ?? [])
            for (const v of d.devices)
                if (v.device === id)
                    return v.label
        return ""
    }

    function minutes(seconds) {
        return Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0")
    }

    // A control request has its own row if its peer is not a candidate of the
    // device on a display (UI-18).
    readonly property var ownControlRows: (menu?.control_requests ?? []).filter(r =>
        !(menu.displays.some(d => d.devices.some(v => v.device === r.device
            && v.candidates.some(c => c.target === r.peer)))))

    Kirigami.Theme.colorSet: Kirigami.Theme.Window
    Kirigami.Theme.inherit: false

    Rectangle {
        anchors.fill: parent
        color: Kirigami.Theme.backgroundColor
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // The header.
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: header.implicitHeight
            Kirigami.Theme.colorSet: Kirigami.Theme.Header
            Kirigami.Theme.inherit: false
            color: Kirigami.Theme.backgroundColor

            RowLayout {
                id: header
                anchors.left: parent.left
                anchors.right: parent.right
                spacing: Kirigami.Units.largeSpacing * 1.5

                Image {
                    Layout.leftMargin: Kirigami.Units.gridUnit
                    Layout.topMargin: Kirigami.Units.largeSpacing
                    Layout.bottomMargin: Kirigami.Units.largeSpacing * 1.5
                    source: Qt.resolvedUrl("telescreen.png")
                    sourceSize.width: Kirigami.Units.iconSizes.huge
                    sourceSize.height: Kirigami.Units.iconSizes.huge
                    Accessible.name: qsTr("Telescreen")
                }
                ColumnLayout {
                    spacing: 0
                    Kirigami.Heading {
                        text: qsTr("Telescreen")
                        level: 1
                        type: Kirigami.Heading.Type.Normal
                    }
                    QQC2.Label {
                        text: root.menu ? root.menu.computer + " · " + root.sessionText() : ""
                        color: Kirigami.Theme.disabledTextColor
                    }
                }
                Item { Layout.fillWidth: true }
                QQC2.ToolButton {
                    icon.name: "list-add"
                    text: qsTr("Add Computer…")
                    // The pairing flow of ADR 0014 is not implemented.
                    enabled: false
                }
                QQC2.ToolButton {
                    Layout.rightMargin: Kirigami.Units.largeSpacing
                    icon.name: "application-menu"
                    display: QQC2.AbstractButton.IconOnly
                    text: qsTr("Open Menu")
                    down: appMenu.visible
                    onPressed: appMenu.popup(this, 0, height)
                    QQC2.ToolTip.text: text
                    QQC2.ToolTip.visible: hovered && !appMenu.visible
                    QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
                    QQC2.Menu {
                        id: appMenu
                        QQC2.MenuItem {
                            icon.name: "configure"
                            text: qsTr("Configure Telescreen…")
                            onTriggered: root.openSettings()
                        }
                        QQC2.MenuSeparator {}
                        QQC2.MenuItem {
                            icon.name: "application-exit"
                            text: qsTr("Quit Telescreen")
                            onTriggered: root.quit()
                        }
                    }
                }
            }
            Kirigami.Separator {
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
            }
        }

        QQC2.ScrollView {
            id: scroll
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: availableWidth
            QQC2.ScrollBar.horizontal.policy: QQC2.ScrollBar.AlwaysOff

            GridLayout {
                width: scroll.availableWidth - 2 * Kirigami.Units.gridUnit
                x: Kirigami.Units.gridUnit
                columns: width > Kirigami.Units.gridUnit * 36 ? 2 : 1
                columnSpacing: Kirigami.Units.gridUnit
                rowSpacing: 0

                // Left: the sessions, the displays, and the notices.
                ColumnLayout {
                    Layout.alignment: Qt.AlignTop
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    Layout.bottomMargin: Kirigami.Units.gridUnit
                    spacing: Kirigami.Units.largeSpacing

                    Repeater {
                        model: root.menu?.sessions ?? []
                        delegate: SessionCard {
                            required property var modelData
                            session: modelData
                            onIntent: intent => root.intent(intent)
                        }
                    }
                    Repeater {
                        model: root.menu?.displays ?? []
                        delegate: DisplaySection {
                            required property var modelData
                            display: modelData
                            controlRequests: root.menu.control_requests
                            onIntent: intent => root.intent(intent)
                        }
                    }
                    ColumnLayout {
                        visible: root.ownControlRows.length > 0
                        Layout.fillWidth: true
                        spacing: 0
                        SectionHeader { text: qsTr("Other Devices") }
                        Card {
                            Repeater {
                                model: root.ownControlRows
                                delegate: TextRow {
                                    required property var modelData
                                    required property int index
                                    separator: index > 0
                                    text: modelData.device_label
                                    subtitle: modelData.waiting ? qsTr("Waiting for %1…").arg(modelData.peer_label) : ""
                                    QQC2.Button {
                                        enabled: !modelData.waiting
                                        text: qsTr("Let %1 Control This Display").arg(modelData.peer_label)
                                        onClicked: root.intent({ kind: "let_control", peer: modelData.peer, device: modelData.device })
                                    }
                                }
                            }
                        }
                    }
                    Repeater {
                        model: root.screen?.notices ?? []
                        delegate: Kirigami.InlineMessage {
                            required property var modelData
                            Layout.fillWidth: true
                            Layout.topMargin: Kirigami.Units.largeSpacing
                            visible: true
                            type: Kirigami.MessageType.Warning
                            text: modelData
                        }
                    }
                }

                // Right: this computer, the computers, and the devices.
                ColumnLayout {
                    Layout.alignment: Qt.AlignTop
                    Layout.fillWidth: true
                    Layout.preferredWidth: 1
                    Layout.bottomMargin: Kirigami.Units.gridUnit
                    spacing: 0

                    SectionHeader { text: qsTr("This Computer") }
                    Card {
                        SwitchRow {
                            visible: root.menu?.clipboard !== null && root.menu?.clipboard !== undefined
                            text: qsTr("Sync text clipboard")
                            on: root.menu?.clipboard === true
                            onToggle: on => root.intent({ kind: "set_clipboard", on: on })
                        }
                        SwitchRow {
                            readonly property var discovery: root.screen?.discovery ?? null
                            readonly property bool active: discovery !== null && discovery.remaining_seconds !== null
                            visible: discovery !== null
                            separator: root.menu?.clipboard !== null && root.menu?.clipboard !== undefined
                            text: qsTr("Discoverable")
                            subtitle: active
                                ? qsTr("Other computers can find this computer · %1 left").arg(root.minutes(discovery.remaining_seconds))
                                : qsTr("Other computers cannot find this computer")
                            on: active
                            onToggle: on => root.intent({ kind: on ? "allow_discovery" : "stop_discovery" })
                        }
                        TextRow {
                            separator: true
                            text: qsTr("Fingerprint")
                            subtitle: root.screen?.fingerprint ?? ""
                        }
                    }

                    SectionHeader { text: qsTr("Computers") }
                    Card {
                        Repeater {
                            model: root.screen?.computers ?? []
                            delegate: TextRow {
                                required property var modelData
                                required property int index
                                separator: index > 0
                                text: modelData.label
                                // UI-13: an untrusted computer shows the trust command.
                                subtitle: !modelData.this_computer && !modelData.trusted
                                    ? qsTr("Add its fingerprint with <tt>telescreen trust</tt>") : ""
                                trailingText: modelData.this_computer ? qsTr("This computer")
                                    : !modelData.trusted ? qsTr("Not trusted")
                                    : modelData.connected ? qsTr("Trusted · online") : qsTr("Trusted · offline")
                                trailingColor: modelData.this_computer ? Kirigami.Theme.disabledTextColor
                                    : !modelData.trusted ? Kirigami.Theme.neutralTextColor
                                    : modelData.connected ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.disabledTextColor
                            }
                        }
                    }

                    SectionHeader {
                        visible: (root.screen?.devices ?? []).length > 0
                        text: qsTr("Devices")
                    }
                    Card {
                        visible: (root.screen?.devices ?? []).length > 0
                        Repeater {
                            model: root.screen?.devices ?? []
                            delegate: TextRow {
                                required property var modelData
                                required property int index
                                separator: index > 0
                                text: root.deviceLabel(modelData.device) || modelData.alias
                                trailingText: ({
                                    unchanged: qsTr("Not switched"),
                                    unverified: qsTr("Switched, not verified"),
                                    confirmed: qsTr("Confirmed"),
                                    unknown: qsTr("Unknown"),
                                })[modelData.state] ?? modelData.state
                                // VER-6: the user says that an unknown device is back.
                                QQC2.Button {
                                    visible: modelData.state === "unknown"
                                    text: qsTr("It Is Back")
                                    onClicked: root.intent({ kind: "device_is_back", device: modelData.device })
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
