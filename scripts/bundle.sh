#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
target_dir=${CARGO_TARGET_DIR:-"$root/target"}
project=${PROJECT:-"$root"}
action=${1:-bundle}
cd "$root"

case $action in
    bundle|--relaunch) ;;
    *)
        echo "usage: $0 [--relaunch]" >&2
        exit 2
        ;;
esac

platform=$(uname -s)
case $platform in
    Darwin)
        formats=${BUNDLE_FORMATS:-app}
        ;;
    Linux)
        formats=${BUNDLE_FORMATS:-appimage}
        ;;
    *)
        echo "Farcaster bundles support macOS and Linux" >&2
        exit 1
        ;;
esac

if [ "$platform" = "Linux" ] && [ "$action" = "--relaunch" ]; then
    case $formats in
        appimage|appimage,*|*,appimage|*,appimage,*) ;;
        *)
            echo "bundle-relaunch requires appimage in BUNDLE_FORMATS" >&2
            exit 2
            ;;
    esac
fi

mkdir -p "$target_dir/release"
if [ "$platform" = "Linux" ]; then
    # cargo-packager names Linux desktop entries after the main executable. Use
    # a tiny launcher named after our canonical app ID so Wayland shells can
    # resolve io.github.behzade.farcaster.desktop and its matching icon, while
    # keeping the application process and command named `farcaster`.
    CARGO_TARGET_DIR="$target_dir" cargo build --release --locked --bin farcaster
    launcher="$target_dir/release/io.github.behzade.farcaster"
    cat >"$launcher" <<'EOF'
#!/bin/sh
exec "$(dirname "$0")/farcaster" "$@"
EOF
    chmod 755 "$launcher"
    cp "$launcher" "$launcher."

    packager_config="$root/packaging/linux.toml"
    case ",$formats," in
        *,appimage,*)
            # linuxdeploy excludes libxcb as a host library and cannot detect
            # the Wayland and graphics libraries GPUI loads at runtime. NixOS
            # provides none of them in the global loader search path.
            libxcb=$(ldd "$target_dir/release/farcaster" | awk \
                '$1 == "libxcb.so.1" && $2 == "=>" { print $3; exit }')
            wayland_libdir=$(pkg-config --variable=libdir wayland-client)
            libwayland_client="$wayland_libdir/libwayland-client.so.0"
            libwayland_egl="$wayland_libdir/libwayland-egl.so.1"
            libvulkan="$(pkg-config --variable=libdir vulkan)/libvulkan.so.1"
            egl_libdir=$(pkg-config --variable=libdir egl)
            libegl="$egl_libdir/libEGL.so.1"
            libgl_dispatch="$egl_libdir/libGLdispatch.so.0"
            for library in "$libxcb" "$libwayland_client" "$libwayland_egl" \
                "$libvulkan" "$libegl" "$libgl_dispatch"; do
                if [ -z "$library" ] || [ ! -f "$library" ]; then
                    echo "could not locate AppImage runtime library: $library" >&2
                    exit 1
                fi
            done

            staged_libxcb="$target_dir/release/libxcb.so.1.appimage"
            staged_wayland_client="$target_dir/release/libwayland-client.so.0.appimage"
            staged_wayland_egl="$target_dir/release/libwayland-egl.so.1.appimage"
            staged_vulkan="$target_dir/release/libvulkan.so.1.appimage"
            staged_egl="$target_dir/release/libEGL.so.1.appimage"
            staged_gl_dispatch="$target_dir/release/libGLdispatch.so.0.appimage"
            generated_config=$(mktemp "$root/packaging/linux.XXXXXX.toml")
            cleanup_appimage_staging() {
                rm -f "$staged_libxcb" "$staged_wayland_client" \
                    "$staged_wayland_egl" "$staged_vulkan" "$staged_egl" \
                    "$staged_gl_dispatch" "$generated_config"
            }
            trap cleanup_appimage_staging EXIT HUP INT TERM
            cp -L "$libxcb" "$staged_libxcb"
            cp -L "$libwayland_client" "$staged_wayland_client"
            cp -L "$libwayland_egl" "$staged_wayland_egl"
            cp -L "$libvulkan" "$staged_vulkan"
            cp -L "$libegl" "$staged_egl"
            cp -L "$libgl_dispatch" "$staged_gl_dispatch"
            cat "$packager_config" >"$generated_config"
            cat >>"$generated_config" <<EOF

[appimage.files]
"$staged_libxcb" = "/usr/lib/libxcb.so.1"
"$staged_wayland_client" = "/usr/lib/libwayland-client.so.0"
"$staged_wayland_egl" = "/usr/lib/libwayland-egl.so.1"
"$staged_vulkan" = "/usr/lib/libvulkan.so.1"
"$staged_egl" = "/usr/lib/libEGL.so.1"
"$staged_gl_dispatch" = "/usr/lib/libGLdispatch.so.0"
EOF
            packager_config=$generated_config
            ;;
    esac

    # appimagetool passes explicit timestamp flags to mksquashfs. Recent
    # mksquashfs rejects those flags when SOURCE_DATE_EPOCH is also inherited.
    unset SOURCE_DATE_EPOCH
    cargo packager --config "$packager_config" --formats "$formats" \
        --out-dir "$target_dir/release" --binaries-dir "$target_dir/release"
else
    CARGO_TARGET_DIR="$target_dir" cargo packager --release --formats "$formats" \
        --out-dir "$target_dir/release"
fi

production_bundle_identifier=io.github.behzade.farcaster
bundle_identifier=$production_bundle_identifier
if [ "$platform" = "Darwin" ]; then
    bundle="$target_dir/release/Farcaster.app"
    if [ ! -d "$bundle" ]; then
        echo "macOS bundle not found: $bundle" >&2
        exit 1
    fi
    identity=${CODESIGN_IDENTITY:--}
    if [ "$identity" = "-" ]; then
        # The default designated requirement for an ad-hoc signature contains
        # the binary hash. Give local builds a stable, separate identity so
        # macOS privacy grants survive rebuilds.
        bundle_identifier=$bundle_identifier.dev
        /usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $bundle_identifier" \
            "$bundle/Contents/Info.plist"
        codesign --force --sign - "$bundle/Contents/MacOS/farcaster"
        codesign --force --sign - \
            --requirements "=designated => identifier \"$bundle_identifier\"" "$bundle"
    else
        codesign --force --options runtime --timestamp --sign "$identity" \
            "$bundle/Contents/MacOS/farcaster"
        codesign --force --options runtime --timestamp --sign "$identity" "$bundle"
    fi
    codesign --verify --deep --strict "$bundle"
fi

if [ "$action" != "--relaunch" ]; then
    exit 0
fi

wait_for_linux_exit() {
    attempts=0
    while pgrep -x farcaster >/dev/null 2>&1; do
        if [ "$attempts" -ge 50 ]; then
            echo "Farcaster did not stop within five seconds" >&2
            exit 1
        fi
        attempts=$((attempts + 1))
        sleep 0.1
    done
}

case $platform in
    Darwin)
        for identifier in "$production_bundle_identifier" "$bundle_identifier"; do
            osascript -e "if application id \"$identifier\" is running then tell application id \"$identifier\" to quit" \
                >/dev/null 2>&1 || true
        done
        app_is_running() {
            osascript -e "application id \"$1\" is running" 2>/dev/null | grep -q true
        }
        attempts=0
        while app_is_running "$production_bundle_identifier" \
            || app_is_running "$bundle_identifier"; do
            if [ "$attempts" -ge 50 ]; then
                echo "Farcaster did not stop within five seconds" >&2
                exit 1
            fi
            attempts=$((attempts + 1))
            sleep 0.1
        done
        open -n "$bundle" --args "$project"
        ;;
    Linux)
        appimage=$(find "$target_dir/release" -maxdepth 1 -type f -iname '*farcaster*.AppImage' \
            -printf '%T@ %p\n' | sort -nr | sed -n '1s/^[^ ]* //p')
        if [ -z "$appimage" ]; then
            echo "Farcaster AppImage not found in $target_dir/release" >&2
            exit 1
        fi
        pkill -TERM -x farcaster >/dev/null 2>&1 || true
        wait_for_linux_exit
        nohup "$appimage" "$project" >/dev/null 2>&1 &
        ;;
esac
