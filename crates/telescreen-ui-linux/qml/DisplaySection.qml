// One local display (UI-9, UI-18): the header with the `portal-match` result
// and "Refresh" (NET-10), then a card with one row for each candidate.
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property var display
    property var controlRequests: []
    signal intent(var intent)

    readonly property var shown: display.devices.filter(d => d.candidates.length > 0 || d.switch_back === true)

    function deviceText() {
        if (display.devices.length !== 1)
            return ""
        return {
            confirmed: qsTr("Shows this computer"),
            unverified: qsTr("Switched, not verified"),
            unknown: qsTr("Unknown state"),
        }[display.devices[0].state] ?? ""
    }

    Layout.fillWidth: true
    spacing: 0

    SectionHeader {
        text: root.display.name
        QQC2.Label {
            text: root.deviceText()
            visible: text !== ""
            color: Kirigami.Theme.disabledTextColor
            font: Kirigami.Theme.smallFont
        }
        // Always shown; disabled when no candidate can dial (NET-10).
        QQC2.ToolButton {
            icon.name: "view-refresh"
            display: QQC2.AbstractButton.IconOnly
            text: qsTr("Refresh %1").arg(root.display.name)
            enabled: root.display.can_refresh
            onClicked: root.intent({ kind: "refresh", display: root.display.token })
            QQC2.ToolTip.text: qsTr("Refresh")
            QQC2.ToolTip.visible: hovered
            QQC2.ToolTip.delay: Kirigami.Units.toolTipDelay
        }
    }

    Card {
        TextRow {
            visible: root.display.matching.kind === "evaluating"
            text: qsTr("Checking the display…")
            dim: true
            QQC2.BusyIndicator {
                running: parent.visible
                implicitWidth: Kirigami.Units.iconSizes.smallMedium
                implicitHeight: implicitWidth
            }
        }
        TextRow {
            visible: root.display.matching.kind === "failed"
            text: qsTr("The display check failed")
            subtitle: root.display.matching.error ?? ""
            subtitleColor: Kirigami.Theme.negativeTextColor
        }
        TextRow {
            visible: root.display.matching.kind === "done" && root.shown.length === 0
            text: qsTr("No handoff routes in the config")
            dim: true
        }
        Repeater {
            model: root.display.matching.kind === "done" ? root.shown : []
            delegate: ColumnLayout {
                id: deviceItem
                required property var modelData
                required property int index
                Layout.fillWidth: true
                spacing: 0
                // A name row only when more than one device matches the display.
                TextRow {
                    visible: root.shown.length > 1
                    separator: deviceItem.index > 0
                    text: deviceItem.modelData.label
                    dim: true
                }
                Repeater {
                    model: deviceItem.modelData.candidates
                    delegate: CandidateRow {
                        required property var modelData
                        required property int index
                        candidate: modelData
                        display: root.display.token
                        device: deviceItem.modelData.device
                        controlRequest: root.controlRequests.find(r =>
                            r.peer === modelData.target && r.device === deviceItem.modelData.device) ?? null
                        separator: index > 0 || (deviceItem.index > 0 && root.shown.length === 1)
                        onIntent: intent => root.intent(intent)
                    }
                }
                TextRow {
                    visible: deviceItem.modelData.candidates.length === 0
                    text: qsTr("No handoff routes in the config")
                    dim: true
                }
                QQC2.ItemDelegate {
                    visible: deviceItem.modelData.switch_back === true
                    Layout.fillWidth: true
                    implicitHeight: Kirigami.Units.gridUnit * 3
                    leftPadding: Kirigami.Units.largeSpacing
                    rightPadding: Kirigami.Units.largeSpacing
                    text: qsTr("Switch back to this computer")
                    onClicked: root.intent({
                        kind: "switch_back",
                        display: root.display.token,
                        device: deviceItem.modelData.device,
                    })
                    background: Rectangle {
                        color: parent.hovered ? Qt.alpha(Kirigami.Theme.textColor, 0.05) : "transparent"
                    }
                    contentItem: RowLayout {
                        spacing: Kirigami.Units.largeSpacing
                        Kirigami.Icon {
                            source: "edit-undo"
                            implicitWidth: Kirigami.Units.iconSizes.smallMedium
                            implicitHeight: implicitWidth
                            opacity: 0.6
                        }
                        QQC2.Label {
                            Layout.fillWidth: true
                            text: qsTr("Switch back to this computer")
                            elide: Text.ElideRight
                        }
                    }
                }
            }
        }
    }
}
