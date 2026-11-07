"""A fake Telescreen D-Bus service for the Plasma applet (UI-11, UI-18).

It owns the name dev.telescreen.Telescreen and serves the interface
dev.telescreen.Menu1 at /dev/telescreen/Menu, as the app will do. The `Menu`
property is the `menu` value of a `MainScreen` fixture (or a `MenuScreen`
fixture). When the file changes, the service sends PropertiesChanged. Each
method call goes to stdout.

Usage (Linux, in the user session; needs dbus-python and PyGObject):

    python3 tools/fake_menu_service.py crates/telescreen-ui-linux/fixtures/main-idle.json

Then show the applet with `plasmoidviewer -a applet/dev.telescreen.menu`.
"""

import json
import os
import sys

import dbus
import dbus.mainloop.glib
import dbus.service
from gi.repository import GLib

NAME = "dev.telescreen.Telescreen"
PATH = "/dev/telescreen/Menu"
IFACE = "dev.telescreen.Menu1"


def read_menu(path: str) -> str:
    with open(path, encoding="utf-8") as f:
        value = json.load(f)
    return json.dumps(value.get("menu", value))


class Menu(dbus.service.Object):
    def __init__(self, bus: dbus.Bus, fixture: str) -> None:
        super().__init__(bus, PATH)
        self.fixture = fixture
        self.mtime = os.stat(fixture).st_mtime
        self.menu = read_menu(fixture)

    def poll(self) -> bool:
        mtime = os.stat(self.fixture).st_mtime
        if mtime != self.mtime:
            self.mtime = mtime
            try:
                self.menu = read_menu(self.fixture)
            except ValueError as e:
                print(f"fake_menu_service: {e}", flush=True)
                return True
            self.PropertiesChanged(IFACE, {"Menu": self.menu}, [])
            print("fake_menu_service: Menu changed", flush=True)
        return True

    @dbus.service.method(IFACE, in_signature="s", out_signature="")
    def Send(self, intent: str) -> None:
        print(f"Send {intent}", flush=True)

    @dbus.service.method(IFACE, in_signature="", out_signature="")
    def ShowMainWindow(self) -> None:
        print("ShowMainWindow", flush=True)

    @dbus.service.method(IFACE, in_signature="", out_signature="")
    def OpenSettings(self) -> None:
        print("OpenSettings", flush=True)

    @dbus.service.method(IFACE, in_signature="", out_signature="")
    def Quit(self) -> None:
        print("Quit", flush=True)
        loop.quit()

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="ss", out_signature="v")
    def Get(self, interface: str, name: str):
        if interface == IFACE and name == "Menu":
            return dbus.String(self.menu)
        raise dbus.exceptions.DBusException(f"No property {interface}.{name}",
                                            name="org.freedesktop.DBus.Error.UnknownProperty")

    @dbus.service.method(dbus.PROPERTIES_IFACE, in_signature="s", out_signature="a{sv}")
    def GetAll(self, interface: str):
        return {"Menu": dbus.String(self.menu)} if interface == IFACE else {}

    @dbus.service.signal(dbus.PROPERTIES_IFACE, signature="sa{sv}as")
    def PropertiesChanged(self, interface, changed, invalidated):
        pass


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    session = dbus.SessionBus()
    name = dbus.service.BusName(NAME, session, do_not_queue=True)
    menu = Menu(session, sys.argv[1])
    GLib.timeout_add(300, menu.poll)
    loop = GLib.MainLoop()
    print(f"fake_menu_service: serving {sys.argv[1]}", flush=True)
    loop.run()
