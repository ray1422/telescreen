// One session of this peer in the applet (UI-9): as the source or the target.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.components as PlasmaComponents3
import org.kde.kirigami as Kirigami

Item {
    id: root

    required property var session
    signal intent(var intent)

    readonly property bool source: session.role.kind === "source"

    function phaseText(phase) {
        switch (phase.kind) {
        case "requesting": return i18n("Requesting a session…")
        case "handing_off": return i18n("Switching the display…")
        case "verifying": return i18n("Checking the display…")
        case "waiting_for_ack": return i18n("Press %1 in the portal to start", phase.key)
        case "forwarding": return i18n("Forwarding input")
        case "ended": return i18n("Ended: %1", phase.reason)
        }
        return phase.kind
    }

    Layout.fillWidth: true
    Layout.leftMargin: Kirigami.Units.smallSpacing
    Layout.rightMargin: Kirigami.Units.smallSpacing
    Layout.topMargin: Kirigami.Units.smallSpacing
    implicitHeight: column.implicitHeight + Kirigami.Units.largeSpacing * 2

    G2Rect {
        anchors.fill: parent
        radius: Kirigami.Units.cornerRadius
        color: Qt.alpha(Kirigami.Theme.highlightColor, 0.14)
        borderColor: Qt.alpha(Kirigami.Theme.highlightColor, 0.4)
        borderWidth: 1
    }

    ColumnLayout {
        id: column
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: Kirigami.Units.largeSpacing
        spacing: Kirigami.Units.smallSpacing

        PlasmaComponents3.Label {
            Layout.fillWidth: true
            text: root.source ? i18n("Controlling %1", root.session.peer_label)
                              : i18n("Controlled by %1", root.session.peer_label)
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
        PlasmaComponents3.Label {
            Layout.fillWidth: true
            text: root.source ? root.phaseText(root.session.role.phase)
                              : [root.session.role.state, root.session.role.hand_back].filter(t => t).join(" · ")
            font: Kirigami.Theme.smallFont
            wrapMode: Text.Wrap
            opacity: 0.7
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing
            PlasmaComponents3.Button {
                Layout.fillWidth: true
                text: i18nc("@action:button", "Hand back")
                enabled: root.source || root.session.role.hand_back === null
                onClicked: root.intent(root.source ? { kind: "source_hand_back" }
                                                   : { kind: "hand_back", session: root.session.id.session })
            }
            PlasmaComponents3.Button {
                Layout.fillWidth: true
                text: root.source ? i18nc("@action:button", "End session") : i18nc("@action:button", "Stop")
                onClicked: root.intent(root.source ? { kind: "end_session" }
                                                   : { kind: "stop_session", session: root.session.id.session })
            }
        }
        PlasmaComponents3.Button {
            visible: !root.source
            Layout.fillWidth: true
            text: i18nc("@action:button", "Control %1 from here", root.session.peer_label)
            onClicked: root.intent({ kind: "control_from_here", session: root.session.id.session })
        }
    }
}
