// The Qt application of the Linux UI. A QApplication (not a QGuiApplication),
// because the KDE Qt Quick Controls style (org.kde.desktop) draws with QStyle.

#include "app.h"

#include <QtCore/QCoreApplication>
#include <QtCore/QUrl>
#include <QtGui/QIcon>
#include <QtQml/QQmlApplicationEngine>
#include <QtQuickControls2/QQuickStyle>
#include <QtWidgets/QApplication>

std::int32_t ts_ui_run()
{
    static int argc = 1;
    static char name[] = "telescreen";
    static char *argv[] = {name, nullptr};

    // The style of the desktop, unless the user selected a different one.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }
    QApplication app(argc, argv);
    QApplication::setApplicationName(QStringLiteral("Telescreen"));
    // The Wayland app ID; the launcher entry has this name (DIST-4).
    QApplication::setDesktopFileName(QStringLiteral("dev.telescreen.Telescreen"));
    // The window icon also with no installed launcher entry (DIST-4). Qt 6.8 or
    // newer sends it to a compositor with xdg-toplevel-icon.
    QApplication::setWindowIcon(QIcon(QStringLiteral(":/qt/qml/dev/telescreen/ui/qml/telescreen.png")));
    // UI-15: the app continues to run when the user closes all its windows.
    QApplication::setQuitOnLastWindowClosed(false);

    QQmlApplicationEngine engine;
    // Qt 6.4 (Ubuntu 24.04, the release build) has no loadFromModule, and its
    // default import path does not have the resource root of the QML modules.
    engine.addImportPath(QStringLiteral("qrc:/qt/qml"));
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/dev/telescreen/ui/qml/App.qml")));
    if (engine.rootObjects().isEmpty()) {
        return 1;
    }
    return app.exec();
}

void ts_ui_quit()
{
    QCoreApplication::quit();
}
