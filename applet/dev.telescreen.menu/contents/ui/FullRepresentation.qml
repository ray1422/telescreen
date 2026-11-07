// The popup of the applet: the menu of UI-9. The sessions, then one part for
// each local display with its candidates, then the clipboard switch.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.components as PlasmaComponents3
import org.kde.plasma.extras as PlasmaExtras
import org.kde.kirigami as Kirigami

PlasmaExtras.Representation {
    id: root

    property var menu: null
    signal intent(var intent)

    // A control request has its own row if its peer is not a candidate of the
    // device on a display (UI-18).
    readonly property var ownControlRows: (menu?.control_requests ?? []).filter(r =>
        !(menu.displays.some(d => d.devices.some(v => v.device === r.device
            && v.candidates.some(c => c.target === r.peer)))))

    Layout.minimumWidth: Kirigami.Units.gridUnit * 18
    Layout.preferredWidth: Kirigami.Units.gridUnit * 20
    Layout.preferredHeight: Math.min(content.implicitHeight + Kirigami.Units.largeSpacing * 2, Kirigami.Units.gridUnit * 30)
    collapseMarginsHint: true

    PlasmaExtras.PlaceholderMessage {
        anchors.centerIn: parent
        width: parent.width - Kirigami.Units.gridUnit * 2
        visible: root.menu === null
        iconName: "video-display"
        text: i18n("Telescreen is not running")
    }

    PlasmaComponents3.ScrollView {
        id: scroll
        anchors.fill: parent
        visible: root.menu !== null
        contentWidth: availableWidth
        PlasmaComponents3.ScrollBar.horizontal.policy: PlasmaComponents3.ScrollBar.AlwaysOff

        ColumnLayout {
            id: content
            width: scroll.availableWidth
            spacing: Kirigami.Units.smallSpacing

            Repeater {
                model: root.menu?.sessions ?? []
                delegate: SessionItem {
                    required property var modelData
                    session: modelData
                    onIntent: intent => root.intent(intent)
                }
            }

            Repeater {
                model: root.menu?.displays ?? []
                delegate: ColumnLayout {
                    id: displayItem
                    required property var modelData
                    readonly property var shown: modelData.devices.filter(d => d.candidates.length > 0)
                    Layout.fillWidth: true
                    spacing: Kirigami.Units.smallSpacing

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        Layout.rightMargin: Kirigami.Units.smallSpacing
                        Layout.topMargin: Kirigami.Units.smallSpacing
                        PlasmaComponents3.Label {
                            Layout.fillWidth: true
                            text: displayItem.modelData.devices.length === 1
                                && displayItem.modelData.devices[0].state === "confirmed"
                                ? i18nc("@title display name", "%1 · shows this computer", displayItem.modelData.name)
                                : displayItem.modelData.name
                            elide: Text.ElideRight
                            opacity: 0.7
                            font: Kirigami.Theme.smallFont
                        }
                        // Always shown; disabled when no candidate can dial (NET-10).
                        PlasmaComponents3.ToolButton {
                            icon.name: "view-refresh"
                            display: PlasmaComponents3.AbstractButton.IconOnly
                            text: i18nc("@action:button", "Refresh %1", displayItem.modelData.name)
                            enabled: displayItem.modelData.can_refresh
                            onClicked: root.intent({ kind: "refresh", display: displayItem.modelData.token })
                            PlasmaComponents3.ToolTip.text: i18nc("@info:tooltip", "Refresh")
                            PlasmaComponents3.ToolTip.visible: hovered
                        }
                    }

                    PlasmaComponents3.Label {
                        Layout.fillWidth: true
                        Layout.leftMargin: Kirigami.Units.largeSpacing
                        visible: text !== ""
                        text: {
                            const m = displayItem.modelData.matching
                            if (m.kind === "evaluating")
                                return i18n("Checking the display…")
                            if (m.kind === "failed")
                                return i18n("The display check failed: %1", m.error)
                            return displayItem.shown.length === 0 ? i18n("No handoff routes in the config") : ""
                        }
                        wrapMode: Text.Wrap
                        opacity: 0.7
                    }

                    Repeater {
                        model: displayItem.modelData.matching.kind === "done" ? displayItem.shown : []
                        delegate: ColumnLayout {
                            id: deviceItem
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: Kirigami.Units.smallSpacing
                            Repeater {
                                model: deviceItem.modelData.candidates
                                delegate: CandidateItem {
                                    required property var modelData
                                    candidate: modelData
                                    display: displayItem.modelData.token
                                    device: deviceItem.modelData.device
                                    controlRequest: (root.menu?.control_requests ?? []).find(r =>
                                        r.peer === modelData.target && r.device === deviceItem.modelData.device) ?? null
                                    onIntent: intent => root.intent(intent)
                                }
                            }
                        }
                    }
                }
            }

            Repeater {
                model: root.ownControlRows
                delegate: RowLayout {
                    required property var modelData
                    Layout.fillWidth: true
                    Layout.leftMargin: Kirigami.Units.largeSpacing
                    Layout.rightMargin: Kirigami.Units.smallSpacing
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        PlasmaComponents3.Label {
                            Layout.fillWidth: true
                            text: modelData.device_label
                            elide: Text.ElideRight
                        }
                        PlasmaComponents3.Label {
                            Layout.fillWidth: true
                            visible: modelData.waiting
                            text: i18n("Waiting for %1…", modelData.peer_label)
                            font: Kirigami.Theme.smallFont
                            opacity: 0.7
                        }
                    }
                    PlasmaComponents3.Button {
                        enabled: !modelData.waiting
                        text: i18n("Let %1 control this display", modelData.peer_label)
                        onClicked: root.intent({ kind: "let_control", peer: modelData.peer, device: modelData.device })
                    }
                }
            }

            Kirigami.Separator {
                visible: root.menu?.clipboard !== null && root.menu?.clipboard !== undefined
                Layout.fillWidth: true
                Layout.topMargin: Kirigami.Units.smallSpacing
            }
            RowLayout {
                visible: root.menu?.clipboard !== null && root.menu?.clipboard !== undefined
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.largeSpacing
                Layout.rightMargin: Kirigami.Units.largeSpacing
                Layout.bottomMargin: Kirigami.Units.smallSpacing
                PlasmaComponents3.Label {
                    Layout.fillWidth: true
                    text: i18n("Sync text clipboard")
                }
                PlasmaComponents3.Switch {
                    checked: root.menu?.clipboard === true
                    Accessible.name: i18n("Sync text clipboard")
                    onToggled: {
                        const on = checked
                        checked = Qt.binding(() => root.menu?.clipboard === true)
                        root.intent({ kind: "set_clipboard", on: on })
                    }
                }
            }
        }
    }
}
