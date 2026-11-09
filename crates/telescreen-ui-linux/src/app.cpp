// The Qt application of the Linux UI. A QApplication (not a QGuiApplication),
// because the KDE Qt Quick Controls style (org.kde.desktop) draws with QStyle.

#include "app.h"

#include <cstdio>

#include <QtCore/QCoreApplication>
#include <QtCore/QUrl>
#include <QtGui/QIcon>
#include <QtQml/QQmlApplicationEngine>
#include <QtQml/QQmlError>
#include <QtQuickControls2/QQuickStyle>
#include <QtWidgets/QApplication>

namespace {

// The application settings that the run and the self-check share.
void setUpApplication()
{
    // The style of the desktop, unless the user selected a different one.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }
}

// Loads the main window. Returns false if QML cannot make it, for example
// because a QML module is missing. The engine writes the QML errors to stderr.
bool loadMainWindow(QQmlApplicationEngine &engine)
{
    // Qt 6.4 (Ubuntu 24.04, the release build) has no loadFromModule, and its
    // default import path does not have the resource root of the QML modules.
    engine.addImportPath(QStringLiteral("qrc:/qt/qml"));
    engine.load(QUrl(QStringLiteral("qrc:/qt/qml/dev/telescreen/ui/qml/App.qml")));
    return !engine.rootObjects().isEmpty();
}

int argc = 1;
char name[] = "telescreen";
char *argv[] = {name, nullptr};

} // namespace

std::int32_t ts_ui_run()
{
    setUpApplication();
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
    if (!loadMainWindow(engine)) {
        return 1;
    }
    return app.exec();
}

std::int32_t ts_ui_self_check()
{
    // DIST-10: no window on a screen, also with no display server.
    qputenv("QT_QPA_PLATFORM", "offscreen");
    setUpApplication();
    QApplication app(argc, argv);
    QQmlApplicationEngine engine;
    // Write each QML error directly: a logging rule of the desktop can hide
    // the default output of the engine.
    engine.setOutputWarningsToStandardError(false);
    QObject::connect(&engine, &QQmlEngine::warnings, [](const QList<QQmlError> &warnings) {
        for (const QQmlError &warning : warnings) {
            std::fprintf(stderr, "telescreen: QML: %s\n", qPrintable(warning.toString()));
        }
    });
    return loadMainWindow(engine) ? 0 : 1;
}

void ts_ui_quit()
{
    QCoreApplication::quit();
}
