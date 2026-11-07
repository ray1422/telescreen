#!/bin/sh
# Make the Linux AppImage (docs/spikes.md, spike 9).
#
#   packaging/appimage/build.sh [--preview] [output-dir]      default: target/dist
#
# Run it on AlmaLinux 9 (glibc 2.34), as .github/workflows/appimage.yml does.
# The AppImage then runs on each distribution with glibc 2.34 or newer. It
# bundles Qt, the KDE Frameworks that Kirigami and the KDE style need, and their
# dependencies. The host must have: OpenGL and EGL (libglvnd), fontconfig,
# freetype, a font, libxkbcommon, libdbus, and glib.
#
# Without --preview, the AppImage runs `telescreen`. It bundles Qt only when the
# binary links Qt (the Qt UI of M20 step g). With --preview, it runs the QML
# preview of telescreen-ui-linux with a fixture and no Rust build:
#
#   Telescreen-preview-x86_64.AppImage [fixtures/main-idle.json] [out.png]
#
# Environment:
#   QT_VERSION      default 6.12.0. Qt 6.8 draws stale Shape outlines (spike 9).
#   KF_VERSION      default 6.20.0
#   APPIMAGE_WORK   the download and build cache (default: target/appimage)
set -eu

QT_VERSION=${QT_VERSION:-6.12.0}
KF_VERSION=${KF_VERSION:-6.20.0}
# Kirigami and the KDE style (qqc2-desktop-style), with their dependencies, in build order.
FRAMEWORKS="extra-cmake-modules kconfig kguiaddons ki18n kcolorscheme kwidgetsaddons karchive kiconthemes sonnet kirigami qqc2-desktop-style"

preview=false
if [ "${1:-}" = --preview ]; then
    preview=true
    shift
fi
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${1:-"$root/target/dist"}
work=${APPIMAGE_WORK:-"$root/target/appimage"}
qt=$work/qt/$QT_VERSION/gcc_64
kf=$work/kf6-$KF_VERSION-qt$QT_VERSION
appdir=$work/AppDir
mkdir -p "$work" "$out"

# Qt from the official binaries (aqtinstall). The Wayland client is a default archive.
install_qt() {
    [ -x "$qt/bin/qmake" ] && return
    python3 -m pip install --quiet aqtinstall
    aqt install-qt --outputdir "$work/qt" linux desktop "$QT_VERSION" linux_gcc_64 -m qtshadertools qt5compat
}

# The KDE Frameworks from the release tarballs, which are also the source
# that the LGPL needs. Each one that is complete has a stamp file.
build_frameworks() {
    series=${KF_VERSION%.*}
    mkdir -p "$kf" "$work/src"
    for name in $FRAMEWORKS; do
        [ -f "$kf/.done-$name" ] && continue
        tarball=$work/src/$name-$KF_VERSION.tar.xz
        [ -f "$tarball" ] || curl -fsSL -o "$tarball" "https://download.kde.org/stable/frameworks/$series/$name-$KF_VERSION.tar.xz"
        rm -rf "$work/src/$name-$KF_VERSION" "$work/build-$name"
        tar -xJf "$tarball" -C "$work/src"
        cmake -S "$work/src/$name-$KF_VERSION" -B "$work/build-$name" -G Ninja \
            -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$kf" -DCMAKE_PREFIX_PATH="$qt;$kf" \
            -DBUILD_TESTING=OFF -DBUILD_QCH=OFF -DBUILD_PYTHON_BINDINGS=OFF -DBUILD_DESIGNERPLUGIN=OFF \
            -DWITH_WAYLAND=OFF -DWITH_X11=OFF -DUSE_BreezeIcons=OFF
        cmake --build "$work/build-$name"
        cmake --install "$work/build-$name"
        touch "$kf/.done-$name"
    done
}

build_telescreen() {
    (cd "$root" && QMAKE=$qt/bin/qmake env -u CARGO_TARGET_DIR cargo build --release -p telescreen-app)
    cp "$root/target/release/telescreen" "$appdir/usr/bin/telescreen"
    strip "$appdir/usr/bin/telescreen"
}

links_qt() {
    objdump -p "$1" | grep -q 'NEEDED.*libQt6Core'
}

make_appdir() {
    rm -rf "$appdir"
    mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps" \
        "$appdir/usr/share/plasma/plasmoids" "$appdir/usr/share/licenses/telescreen"
    sed 's/^Exec=.*/Exec=telescreen/' "$root/packaging/linux/dev.telescreen.Telescreen.desktop" \
        > "$appdir/usr/share/applications/dev.telescreen.Telescreen.desktop"
    if $preview; then
        sed -i 's/^Name=Telescreen$/Name=Telescreen preview/' "$appdir/usr/share/applications/dev.telescreen.Telescreen.desktop"
    fi
    cp "$root/assets/linux/telescreen.png" "$appdir/usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png"
    # The applet package for a later install into the home directory (UI-11).
    cp -R "$root/applet/dev.telescreen.menu" "$appdir/usr/share/plasma/plasmoids/"
    cp "$root/LICENSE" "$appdir/usr/share/licenses/telescreen/"
}

# Kirigami selects the KDE style at runtime, and Qt loads the Wayland EGL
# integration at runtime. The import scan of linuxdeploy-plugin-qt finds
# neither. Copy them, and later deploy the dependencies of each file, so that
# they never load the KDE Frameworks or Qt of the host.
copy_qt_runtime_parts() {
    lib=$kf/lib64
    [ -d "$lib" ] || lib=$kf/lib
    mkdir -p "$appdir/usr/qml/org/kde" "$appdir/usr/plugins"
    cp -R "$lib/qml/org/kde/desktop" "$lib/qml/org/kde/qqc2desktopstyle" "$appdir/usr/qml/org/kde/"
    cp -R "$lib/plugins/"* "$appdir/usr/plugins/"
    cp -R "$qt/plugins/wayland-graphics-integration-client" "$appdir/usr/plugins/"
    # The license texts of the bundled frameworks. Their source is the tarballs above.
    for name in $FRAMEWORKS; do
        [ -d "$work/src/$name-$KF_VERSION/LICENSES" ] && cp -R "$work/src/$name-$KF_VERSION/LICENSES" "$appdir/usr/share/licenses/$name"
    done
    cat > "$appdir/usr/share/licenses/README" <<NOTICE
This AppImage bundles Qt $QT_VERSION (LGPL-3.0, https://download.qt.io/archive/qt/)
and KDE Frameworks $KF_VERSION (LGPL, https://download.kde.org/stable/frameworks/),
as shared libraries. Each directory here has the license texts of one part.
NOTICE
}

run_linuxdeploy() {
    cd "$work"
    for tool in linuxdeploy linuxdeploy-plugin-qt; do
        [ -x "$tool-x86_64.AppImage" ] && continue
        curl -fsSLO "https://github.com/linuxdeploy/$tool/releases/download/continuous/$tool-x86_64.AppImage"
        chmod +x "$tool-x86_64.AppImage"
    done
    export APPIMAGE_EXTRACT_AND_RUN=1
    export LDAI_OUTPUT="$out/$1"
    set -- --appdir "$appdir" --executable "$2" --custom-apprun "$root/packaging/appimage/AppRun" \
        --desktop-file "$appdir/usr/share/applications/dev.telescreen.Telescreen.desktop" \
        --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/dev.telescreen.Telescreen.png"
    if $with_qt; then
        lib=$kf/lib64
        [ -d "$lib" ] || lib=$kf/lib
        export QMAKE=$qt/bin/qmake
        export LD_LIBRARY_PATH="$lib:$qt/lib"
        export QML_SOURCES_PATHS="$root/crates/telescreen-ui-linux/qml"
        export QML_MODULES_PATHS="$lib/qml"
        export EXTRA_QT_MODULES="waylandclient;svg"
        # The Wayland platform plugins have other names in other Qt versions.
        EXTRA_PLATFORM_PLUGINS=libqoffscreen.so
        for plugin in "$qt"/plugins/platforms/libqwayland*.so; do
            EXTRA_PLATFORM_PLUGINS="$EXTRA_PLATFORM_PLUGINS;$(basename "$plugin")"
        done
        export EXTRA_PLATFORM_PLUGINS
        # --deploy-deps-only does not go into subdirectories: give each copied file.
        for file in $(find "$appdir/usr/plugins" "$appdir/usr/qml/org/kde" -name '*.so'); do
            set -- "$@" --deploy-deps-only "$file"
        done
        set -- "$@" --plugin qt
    fi
    ./linuxdeploy-x86_64.AppImage "$@" --output appimage
}

install_qt
if $preview; then
    name=Telescreen-preview-x86_64.AppImage
    with_qt=true
    build_frameworks
    make_appdir
    mkdir -p "$appdir/usr/share/telescreen"
    cp -R "$root/crates/telescreen-ui-linux/qml" "$root/crates/telescreen-ui-linux/fixtures" "$appdir/usr/share/telescreen/"
    cp "$qt/bin/qml" "$appdir/usr/bin/qml"
    copy_qt_runtime_parts
    run_linuxdeploy "$name" "$appdir/usr/bin/qml"
else
    name=Telescreen-x86_64.AppImage
    make_appdir
    build_telescreen
    if links_qt "$appdir/usr/bin/telescreen"; then
        with_qt=true
        build_frameworks
        copy_qt_runtime_parts
    else
        with_qt=false
    fi
    run_linuxdeploy "$name" "$appdir/usr/bin/telescreen"
fi

echo "made $out/$name"
echo "glibc needed: $(find "$appdir" -type f \( -name '*.so*' -o -perm -u+x \) -exec objdump -T {} \; 2>/dev/null \
    | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1)"
