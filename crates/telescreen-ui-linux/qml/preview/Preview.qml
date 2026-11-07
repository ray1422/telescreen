// Shows MainPage with a fixture, with no Rust build. Run on Linux:
//
//   QML_XHR_ALLOW_FILE_READ=1 QT_QUICK_CONTROLS_STYLE=org.kde.desktop \
//     qml6 -a widget crates/telescreen-ui-linux/qml/preview/Preview.qml -- fixtures/main-idle.json
//
// The fixture path is relative to the crate directory. The intents go to stdout.
// With a second argument that ends in ".png", the preview saves the window
// content to that file and quits.
import QtQuick
import QtQuick.Controls as QQC2
import org.kde.kirigami as Kirigami
import ".."

QQC2.ApplicationWindow {
    id: window

    property var screen: null

    title: "Telescreen preview"
    width: Kirigami.Units.gridUnit * 56
    height: Kirigami.Units.gridUnit * 38
    visible: true

    function load() {
        const args = Qt.application.arguments
        const name = args.find(a => a.endsWith(".json")) ?? "fixtures/main-idle.json"
        const request = new XMLHttpRequest()
        request.open("GET", Qt.resolvedUrl("../../" + name), false)
        request.send()
        window.screen = JSON.parse(request.responseText)
    }

    Component.onCompleted: load()

    Timer {
        running: Qt.application.arguments.some(a => a.endsWith(".png"))
        interval: 1500
        onTriggered: page.grabToImage(result => {
            result.saveToFile(Qt.application.arguments.find(a => a.endsWith(".png")))
            Qt.quit()
        })
    }

    MainPage {
        id: page
        anchors.fill: parent
        screen: window.screen
        onIntent: intent => console.log("intent", JSON.stringify(intent))
        onOpenSettings: console.log("open settings")
        onQuit: Qt.quit()
    }
}
