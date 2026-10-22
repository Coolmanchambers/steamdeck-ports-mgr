#!/usr/bin/env bash
# Builds steamdeck-ports-mgr-installer.desktop: a KDE/freedesktop launcher with tools/installer.sh
# embedded as comment lines. CI runs it in the package step and publishes the result next to the
# tar.gz; tools/test-installer.sh runs the built file the way its Exec line does.
#
#   bash tools/build-installer.sh <output .desktop file>
#
# How the file runs itself: the desktop-entry spec expands %k to the launcher's own path. Exec hands
# that path to bash, which pulls the lines between #BEGIN_DPM_INSTALLER and #END_DPM_INSTALLER back out
# (dropping the "#|" prefix) and runs them. Comment lines are ignored by every desktop-entry reader, so
# the script cannot change how the launcher itself is read. The Exec value follows the spec's quoting:
# one double-quoted argument where \", \$ are written \\" and \\$ (the string escape is undone first),
# and %k stands alone, unquoted. A POSIX shell parses that value to the same arguments, so it works
# whether the launcher splits Exec itself (KDE, GLib) or hands it to sh.
set -euo pipefail
OUT="${1:?output .desktop file}"
SRC="$(dirname "$(readlink -f "$0")")/installer.sh"
[ -f "$SRC" ] || { echo "missing $SRC" >&2; exit 1; }

{
    cat <<'EOF'
#!/usr/bin/env xdg-open
[Desktop Entry]
Type=Application
Version=1.5
Name=Install Deck Ports Manager
Comment=Installs or updates Deck Ports Manager in ~/Applications and opens it
Icon=system-software-install
Exec=bash -c "bash <(sed -n '/^#BEGIN_DPM_INSTALLER/,/^#END_DPM_INSTALLER/s/^#|//p' -- \\"\\$1\\") \\"\\$1\\"" steamdeck-ports-mgr-installer %k
Terminal=false
StartupNotify=false

# Deck Ports Manager installer: https://github.com/Coolmanchambers/steamdeck-ports-mgr
# Double-click this file in Desktop Mode and confirm the one security question; it downloads the newest
# build from that page's rolling release, checks its SHA-256, installs it in ~/Applications and opens it.
# Running it again updates an existing install without touching your ports, saves or settings.
# Everything it runs is the plain bash below (each line starts with "#|"); nothing else is downloaded
# and run. Source: tools/installer.sh in the repository.
#BEGIN_DPM_INSTALLER
EOF
    tr -d '\r' <"$SRC" | sed 's/^/#|/'
    echo "#END_DPM_INSTALLER"
} >"$OUT.tmp"
mv -f "$OUT.tmp" "$OUT"
chmod 644 "$OUT"
echo "built $OUT ($(wc -l <"$OUT") lines)"
