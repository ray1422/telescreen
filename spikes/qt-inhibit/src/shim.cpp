#include "shim.h"

#include <QtCore/QTimer>
#include <QtGui/QGuiApplication>
#include <QtGui/QWindow>
#include <QtGui/qguiapplication_platform.h>
#include <QtGui/qpa/qplatformwindow_p.h>

extern "C" void ts_tick();

namespace {
QWindow *window()
{
    const auto windows = QGuiApplication::topLevelWindows();
    return windows.isEmpty() ? nullptr : windows.first();
}
}

bool ts_platform_is_wayland()
{
    return QGuiApplication::platformName() == QStringLiteral("wayland");
}

std::uintptr_t ts_wl_display()
{
    auto *app = qGuiApp->nativeInterface<QNativeInterface::QWaylandApplication>();
    return app ? reinterpret_cast<std::uintptr_t>(app->display()) : 0;
}

std::uintptr_t ts_window_surface()
{
    QWindow *w = window();
    if (!w || !w->isVisible()) {
        return 0;
    }
    auto *native = w->nativeInterface<QNativeInterface::Private::QWaylandWindow>();
    return native ? reinterpret_cast<std::uintptr_t>(native->surface()) : 0;
}

bool ts_window_active()
{
    QWindow *w = window();
    return w && w->isActive();
}

void ts_set_window_visible(bool visible)
{
    if (QWindow *w = window()) {
        w->setVisible(visible);
    }
}

void ts_request_activate()
{
    if (QWindow *w = window()) {
        w->requestActivate();
    }
}

void ts_start_timer(std::int32_t ms)
{
    auto *timer = new QTimer(qGuiApp);
    QObject::connect(timer, &QTimer::timeout, []() { ts_tick(); });
    timer->start(ms);
}

void ts_quit()
{
    QGuiApplication::quit();
}
