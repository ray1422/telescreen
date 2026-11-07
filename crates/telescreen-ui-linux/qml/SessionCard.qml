// One session of this peer (UI-9): as the source or as the target (FLOW-17).
import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami

ColumnLayout {
    id: root

    required property var session
    signal intent(var intent)

    readonly property bool source: session.role.kind === "source"

    function phaseText(phase) {
        switch (phase.kind) {
        case "requesting": return qsTr("Requesting a session…")
        case "handing_off": return qsTr("Switching the display…")
        case "verifying": return qsTr("Checking the display…")
        case "waiting_for_ack": return qsTr("Press %1 in the portal to start").arg(phase.key)
        case "forwarding": return qsTr("Forwarding input")
        case "ended": return qsTr("Ended: %1").arg(phase.reason)
        }
        return phase.kind
    }

    function videoText(state) {
        return {
            unchanged: qsTr("Display not switched"),
            unverified: qsTr("Display switched, not verified"),
            confirmed: qsTr("Display confirmed"),
            unknown: qsTr("Display state unknown"),
        }[state] ?? state
    }

    Layout.fillWidth: true
    spacing: 0

    SectionHeader {
        text: root.source ? qsTr("Controlling %1").arg(root.session.peer_label)
                          : qsTr("Controlled by %1").arg(root.session.peer_label)
    }
    Card {
        TextRow {
            icon: root.source ? "input-mouse" : "input-keyboard"
            text: root.source ? root.phaseText(root.session.role.phase) : root.session.role.state
            subtitle: root.source ? root.videoText(root.session.role.video_input)
                                  : (root.session.role.hand_back ?? "")
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Kirigami.Units.largeSpacing
            Layout.topMargin: 0
            spacing: Kirigami.Units.smallSpacing
            Item { Layout.fillWidth: true }
            QQC2.Button {
                visible: !root.source
                text: qsTr("Control %1 From Here").arg(root.session.peer_label)
                onClicked: root.intent({ kind: "control_from_here", session: root.session.id.session })
            }
            QQC2.Button {
                text: qsTr("Hand Back")
                enabled: root.source || root.session.role.hand_back === null
                onClicked: root.intent(root.source ? { kind: "source_hand_back", portal: root.session.id.portal }
                                                   : { kind: "hand_back", session: root.session.id.session })
            }
            QQC2.Button {
                text: root.source ? qsTr("End Session") : qsTr("Stop")
                onClicked: root.intent(root.source ? { kind: "end_session", portal: root.session.id.portal }
                                                   : { kind: "stop_session", session: root.session.id.session })
            }
        }
    }
}
