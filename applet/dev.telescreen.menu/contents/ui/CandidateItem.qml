// One candidate in the applet: the computer and its status, then a button for
// each available action. The first one is the primary action (UI-18). An
// unavailable action has no button; if "Switch & control" is unavailable, the
// status line gives its reason.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

Item {
    id: root

    required property var candidate
    required property string display
    required property string device
    property var controlRequest: null
    // The device is in a session of this computer as the source (UI-9).
    property bool inSession: false
    signal intent(var intent)

    readonly property bool online: candidate.status === "online"
    readonly property var actions: {
        const all = [
            { kind: "switch_and_control", text: i18nc("@action:button", "Switch & control %1", candidate.label), reason: candidate.switch_and_control },
            { kind: "switch_display_only", text: i18nc("@action:button", "Switch display only"), reason: candidate.switch_display_only },
            { kind: "control_only", text: i18nc("@action:button", "Control only"), reason: candidate.control_only },
        ].filter(a => a.reason === null)
        if (controlRequest !== null && !controlRequest.waiting)
            all.push({ kind: "let_control", text: i18nc("@action:button", "Let %1 control this display", candidate.label) })
        return all
    }

    function statusText() {
        const status = {
            online: i18nc("@info:status", "Online"),
            connecting: i18nc("@info:status", "Connecting…"),
            searching: i18nc("@info:status", "Searching…"),
            offline: i18nc("@info:status", "Offline"),
        }[candidate.status] ?? candidate.status
        if (controlRequest !== null && controlRequest.waiting)
            return i18n("%1 · Waiting for %2…", status, candidate.label)
        if (candidate.switch_and_control && candidate.status === "online")
            return status + " · " + candidate.switch_and_control
        if (inSession)
            return i18n("%1 · ends the current session", status)
        return status
    }

    function run(kind) {
        if (kind === "let_control")
            intent({ kind: kind, peer: candidate.target, device: device })
        else
            intent({ kind: kind, display: display, device: device, target: candidate.target })
    }

    Layout.fillWidth: true
    Layout.leftMargin: Kirigami.Units.smallSpacing
    Layout.rightMargin: Kirigami.Units.smallSpacing
    implicitHeight: column.implicitHeight + Kirigami.Units.largeSpacing * 2

    // An online candidate has the tinted frame of the mockup.
    G2Rect {
        anchors.fill: parent
        radius: Kirigami.Units.cornerRadius
        color: root.online ? Qt.alpha(Kirigami.Theme.highlightColor, 0.14) : Qt.alpha(Kirigami.Theme.textColor, 0.05)
        borderColor: root.online ? Qt.alpha(Kirigami.Theme.highlightColor, 0.4) : Qt.alpha(Kirigami.Theme.textColor, 0.1)
        borderWidth: 1
    }

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                source: "computer-symbolic"
                implicitWidth: Kirigami.Units.iconSizes.medium
                implicitHeight: implicitWidth
                opacity: root.online ? 1 : 0.6
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                PlasmaComponents3.Label {
                    Layout.fillWidth: true
                    text: root.candidate.label
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                PlasmaComponents3.Label {
                    Layout.fillWidth: true
                    text: root.statusText()
                    font: Kirigami.Theme.smallFont
                    wrapMode: Text.Wrap
                    opacity: 0.7
                }
            }
        }

        Repeater {
            model: root.actions
            // The primary action is the accent button of the mockup.
            delegate: Loader {
                id: slot
                required property var modelData
                required property int index
                Layout.fillWidth: true
                Layout.leftMargin: Kirigami.Units.iconSizes.medium + Kirigami.Units.largeSpacing
                sourceComponent: index === 0 ? primaryButton : plainButton
                Component {
                    id: primaryButton
                    PrimaryButton {
                        text: slot.modelData.text
                        onClicked: root.run(slot.modelData.kind)
                    }
                }
                Component {
                    id: plainButton
                    PlasmaComponents3.Button {
                        text: slot.modelData.text
                        onClicked: root.run(slot.modelData.kind)
                    }
                }
            }
        }
    }
}
