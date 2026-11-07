// The Telescreen applet in the system tray (UI-11). It reads the menu view
// model (`MenuScreen`, UI-18) as JSON from the D-Bus property `Menu`, and
// sends each intent as JSON with the method `Send`. The tray loads the applet
// only while Telescreen owns its D-Bus name (X-Plasma-DBusActivationService).
import QtQuick
import org.kde.plasma.plasmoid
import org.kde.plasma.core as PlasmaCore
import org.kde.kirigami as Kirigami
import org.kde.plasma.workspace.dbus as DBus

PlasmoidItem {
    id: root

    readonly property string service: "dev.telescreen.Telescreen"
    readonly property string objectPath: "/dev/telescreen/Menu"
    readonly property string iface: "dev.telescreen.Menu1"

    readonly property var menu: {
        const text = properties.properties.Menu
        if (!text)
            return null
        try {
            return JSON.parse(text)
        } catch (e) {
            console.warn("Telescreen: the Menu property is not JSON:", e)
            return null
        }
    }

    // UI-17: the glyph of the state; "menu open" while the popup shows.
    readonly property string glyph: {
        const state = menu?.glyph ?? "idle"
        return state === "idle" && expanded ? "menu-open" : state
    }

    function call(member, args) {
        DBus.SessionBus.asyncCall({
            service: service,
            path: objectPath,
            iface: iface,
            member: member,
            arguments: args,
            signature: args.length > 0 ? "s" : "",
        })
    }

    function send(intent) {
        call("Send", [JSON.stringify(intent)])
    }

    function sessionText() {
        if (!menu)
            return i18nc("@info:status", "Not running")
        if (menu.sessions.length === 0)
            return i18nc("@info:status", "No session")
        const s = menu.sessions[0]
        return s.role.kind === "source" ? i18nc("@info:status", "Controlling %1", s.peer_label)
                                        : i18nc("@info:status", "Controlled by %1", s.peer_label)
    }

    DBus.Properties {
        id: properties
        busType: DBus.BusType.Session
        service: root.service
        path: root.objectPath
        iface: root.iface
    }

    Plasmoid.icon: Qt.resolvedUrl("../icons/glyph-" + glyph + ".svg")
    Plasmoid.status: PlasmaCore.Types.ActiveStatus
    toolTipMainText: i18n("Telescreen")
    toolTipSubText: sessionText()

    compactRepresentation: MouseArea {
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton
        onClicked: root.expanded = !root.expanded
        Kirigami.Icon {
            anchors.fill: parent
            source: Plasmoid.icon
            // The glyph is black; the tray draws it in the text color.
            isMask: true
            active: parent.containsMouse
        }
    }

    fullRepresentation: FullRepresentation {
        menu: root.menu
        onIntent: intent => root.send(intent)
    }

    Plasmoid.contextualActions: [
        PlasmaCore.Action {
            text: i18nc("@action", "Open Telescreen")
            icon.name: "window-new"
            onTriggered: root.call("ShowMainWindow", [])
        },
        PlasmaCore.Action {
            text: i18nc("@action", "Configure Telescreen…")
            icon.name: "configure"
            onTriggered: root.call("OpenSettings", [])
        },
        PlasmaCore.Action {
            text: i18nc("@action", "Quit Telescreen")
            icon.name: "application-exit"
            onTriggered: root.call("Quit", [])
        },
    ]
}
