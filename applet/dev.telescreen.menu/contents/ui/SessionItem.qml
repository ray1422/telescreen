// One session of this peer in the applet (UI-9), as in the mockups "KDE
// plasmoid, controlling Mac" and "controlled by Mac". The source card has the
// accent color; the target card has the warning color, because this computer
// is controlled.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

Item {
    id: root

    required property var session
    signal intent(var intent)

    readonly property bool source: session.role.kind === "source"
    readonly property color tint: source ? Kirigami.Theme.highlightColor : Kirigami.Theme.neutralTextColor
    readonly property var videoInput: session.role.video_input ?? null

    function phaseText(phase) {
        switch (phase.kind) {
        case "requesting": return i18n("requesting a session")
        case "handing_off": return i18n("switching the display")
        case "verifying": return i18n("checking the display")
        case "waiting_for_ack": return i18n("press %1 in the portal", phase.key)
        case "forwarding": return i18n("forwarding")
        case "ended": return i18n("ended: %1", phase.reason)
        }
        return phase.kind
    }

    function videoInputText(state) {
        switch (state) {
        case "confirmed": return i18n("display confirmed")
        case "unverified": return i18n("display not verified")
        case "unknown": return i18n("display state unknown")
        }
        return ""
    }

    Layout.fillWidth: true
    Layout.leftMargin: Kirigami.Units.smallSpacing
    Layout.rightMargin: Kirigami.Units.smallSpacing
    Layout.topMargin: Kirigami.Units.smallSpacing
    implicitHeight: column.implicitHeight + Kirigami.Units.largeSpacing * 2

    G2Rect {
        anchors.fill: parent
        radius: Kirigami.Units.cornerRadius
        color: Qt.alpha(root.tint, 0.14)
        borderColor: Qt.alpha(root.tint, 0.45)
        borderWidth: 1
    }

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.largeSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.largeSpacing
            Kirigami.Icon {
                source: "video-display-symbolic"
                color: root.tint
                isMask: true
                implicitWidth: Kirigami.Units.iconSizes.medium
                implicitHeight: implicitWidth
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 0
                PlasmaComponents3.Label {
                    Layout.fillWidth: true
                    text: root.source ? i18n("Controlling %1", root.session.peer_label)
                                      : i18n("Controlled by %1", root.session.peer_label)
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                PlasmaComponents3.Label {
                    Layout.fillWidth: true
                    text: [root.session.device_label,
                           root.source ? root.phaseText(root.session.role.phase) : root.session.role.state,
                           root.videoInputText(root.videoInput)].filter(t => t).join(" · ")
                    font: Kirigami.Theme.smallFont
                    wrapMode: Text.Wrap
                    opacity: 0.75
                }
            }
        }

        // The source: "Hand back" and "End session" side by side.
        RowLayout {
            visible: root.source
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            PrimaryButton {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                text: i18nc("@action:button", "Hand back")
                onClicked: root.intent({ kind: "source_hand_back", portal: root.session.id.portal })
            }
            PlasmaComponents3.Button {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                text: i18nc("@action:button", "End session")
                onClicked: root.intent({ kind: "end_session", portal: root.session.id.portal })
            }
        }

        // The target: "Hand back to <source>" is the main action (FLOW-17).
        PrimaryButton {
            visible: !root.source
            Layout.fillWidth: true
            enabled: !root.source && root.session.role.hand_back === null
            text: i18nc("@action:button", "Hand back to %1", root.session.peer_label)
            onClicked: root.intent({ kind: "hand_back", session: root.session.id.session })
        }
        PlasmaComponents3.Label {
            visible: !root.source && root.session.role.hand_back !== null
            Layout.fillWidth: true
            text: root.session.role.hand_back ?? ""
            font: Kirigami.Theme.smallFont
            wrapMode: Text.Wrap
            opacity: 0.75
        }
        RowLayout {
            visible: !root.source
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            PlasmaComponents3.Button {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                text: i18nc("@action:button", "Control from here")
                onClicked: root.intent({ kind: "control_from_here", session: root.session.id.session })
            }
            PlasmaComponents3.Button {
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                text: i18nc("@action:button", "Disconnect")
                onClicked: root.intent({ kind: "stop_session", session: root.session.id.session })
            }
        }
    }
}
