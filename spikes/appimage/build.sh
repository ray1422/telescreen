#!/bin/sh
# Spike 9: an AppImage of the Linux UI with its own Qt and Kirigami.
#
# It answers three questions: the size of the AppImage, the oldest glibc that
# it needs, and whether the bundled Kirigami and KDE style follow the color
# scheme of the desktop. The AppImage runs the `qml` tool of Qt with the
# preview QML of telescreen-ui-linux (no Rust build: spikes 6 and 7 tested the
# cxx-qt part).
#
# Run it in an AlmaLinux 9 container (glibc 2.34) from the repository root:
#
#   sh spikes/appimage/build.sh
#
# The output is spikes/appimage/out/Telescreen-preview-x86_64.AppImage.
set -eux

QT_VERSION=${QT_VERSION:-6.8.3}
KF_VERSION=${KF_VERSION:-6.20.0}
ROOT=$(pwd)
WORK=$ROOT/spikes/appimage/work
OUT=$ROOT/spikes/appimage/out
PREFIX=$WORK/kf6
mkdir -p "$WORK" "$OUT"
cd "$WORK"

# Qt from the official binaries.
python3 -m pip install --quiet aqtinstall
aqt list-qt linux desktop --archives "$QT_VERSION" linux_gcc_64 || true

aqt install-qt --outputdir "$WORK/qt" linux desktop "$QT_VERSION" linux_gcc_64 -m qtshadertools qt5compat
QT=$WORK/qt/$QT_VERSION/gcc_64
ls "$QT/plugins/platforms"
export PATH=$QT/bin:$PATH
export CMAKE_PREFIX_PATH=$QT:$PREFIX

# The KDE Frameworks that Kirigami and the KDE style need, from the release tarballs.
series=${KF_VERSION%.*}
for name in extra-cmake-modules kconfig kguiaddons ki18n kcolorscheme kwidgetsaddons karchive kiconthemes sonnet kirigami qqc2-desktop-style; do
    if [ ! -f "$PREFIX/.done-$name" ]; then
        curl -fsSL "https://download.kde.org/stable/frameworks/$series/$name-$KF_VERSION.tar.xz" | tar xJ
        cmake -S "$name-$KF_VERSION" -B "build-$name" -G Ninja \
            -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX" \
            -DBUILD_TESTING=OFF -DBUILD_QCH=OFF -DBUILD_PYTHON_BINDINGS=OFF \
            -DWITH_WAYLAND=OFF -DWITH_X11=OFF -DUSE_DBUS=ON \
            -DUSE_BreezeIcons=OFF -DBUILD_DESIGNERPLUGIN=OFF
        cmake --build "build-$name"
        cmake --install "build-$name"
        touch "$PREFIX/.done-$name"
    fi
done
LIBDIR=$PREFIX/lib64
[ -d "$LIBDIR" ] || LIBDIR=$PREFIX/lib

# The AppDir.
rm -rf AppDir
mkdir -p AppDir/usr/share/telescreen AppDir/usr/share/applications AppDir/usr/share/icons/hicolor/256x256/apps
cp -R "$ROOT/crates/telescreen-ui-linux/qml" "$ROOT/crates/telescreen-ui-linux/fixtures" AppDir/usr/share/telescreen/
cp "$ROOT/assets/linux/telescreen.png" AppDir/usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png
cat > AppDir/usr/share/applications/dev.telescreen.Telescreen.desktop <<DESKTOP
[Desktop Entry]
Type=Application
Name=Telescreen preview
Exec=AppRun
Icon=dev.telescreen.Telescreen
Categories=Utility;
DESKTOP
# The KDE style is selected at runtime, not imported, so the import scan
# does not find it. Copy its QML modules and plugins.
mkdir -p AppDir/usr/qml/org/kde AppDir/usr/plugins
cp -R "$LIBDIR/qml/org/kde/desktop" "$LIBDIR/qml/org/kde/qqc2desktopstyle" AppDir/usr/qml/org/kde/ || true
cp -R "$LIBDIR/plugins/"* AppDir/usr/plugins/ || true
find AppDir/usr/plugins AppDir/usr/qml -name '*.so' | sort

curl -fsSLO https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-x86_64.AppImage
curl -fsSLO https://github.com/linuxdeploy/linuxdeploy-plugin-qt/releases/download/continuous/linuxdeploy-plugin-qt-x86_64.AppImage
chmod +x linuxdeploy-*.AppImage
export APPIMAGE_EXTRACT_AND_RUN=1
export QMAKE=$QT/bin/qmake
export LD_LIBRARY_PATH=$LIBDIR:$QT/lib
export QML_SOURCES_PATHS=$ROOT/crates/telescreen-ui-linux/qml
export QML_MODULES_PATHS=$LIBDIR/qml
export EXTRA_PLATFORM_PLUGINS="libqwayland-generic.so;libqwayland-egl.so;libqoffscreen.so"
export EXTRA_QT_MODULES="waylandclient;svg"
export LDAI_OUTPUT=$OUT/Telescreen-preview-x86_64.AppImage
./linuxdeploy-x86_64.AppImage --appdir AppDir \
    --executable "$QT/bin/qml" \
    --custom-apprun "$ROOT/spikes/appimage/AppRun" \
    --desktop-file AppDir/usr/share/applications/dev.telescreen.Telescreen.desktop \
    --icon-file AppDir/usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png \
    --plugin qt --output appimage

# The results of the spike.
ls -l "$OUT"
du -sh AppDir
echo "glibc needed:"
find AppDir -type f \( -name '*.so*' -o -perm -u+x \) -exec sh -c 'objdump -T "$1" 2>/dev/null' _ {} \; \
    | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
echo "Bundled libraries:"
find AppDir/usr/lib -name '*.so*' -printf '%s %p\n' | sort -n | awk '{s+=$1; print} END {print s/1048576 " MiB"}'
