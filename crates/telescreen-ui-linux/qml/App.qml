// The main window of the app. `Backend` is the Rust side (src/bridge.rs).
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import dev.telescreen.ui

QQC2.ApplicationWindow {
    id: window

    title: qsTr("Telescreen")
    width: Kirigami.Units.gridUnit * 56
    height: Kirigami.Units.gridUnit * 38
    minimumWidth: Kirigami.Units.gridUnit * 22
    minimumHeight: Kirigami.Units.gridUnit * 20
    visible: true

    MainPage {
        anchors.fill: parent
        screen: Backend.screen === "" ? null : JSON.parse(Backend.screen)
        onIntent: intent => Backend.send(JSON.stringify(intent))
        onOpenSettings: Backend.openSettings()
        onQuit: Backend.quit()
    }
}
