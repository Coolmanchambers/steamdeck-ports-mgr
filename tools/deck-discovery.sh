#!/usr/bin/env bash
# deck-discovery.sh: read-only inventory of a Steam Deck for the Deck Ports Manager fork.
#
# Run once, as the normal user (no sudo), in Konsole in Desktop Mode:
#   bash deck-discovery.sh [extra ROM folder ...]
#
# It prints a report and saves a copy to ~/deck-discovery-YYYYMMDD-HHMM.md.
# That report file is the ONLY thing this script creates. It never runs a program it finds,
# never prints file contents (apart from the first line of a tiny version file), never hashes or
# lists ROM file names, and opens shortcuts.vdf in read-only mode only.
#
# Part of https://github.com/Coolmanchambers/steamdeck-ports-mgr (GPL-3.0-or-later).

set -u
export LC_ALL=C
export PYTHONDONTWRITEBYTECODE=1
umask 077

SCRIPT_VERSION="1.0.0"

case "${1:-}" in
  -h|--help)
    cat <<'EOF'
Usage: bash deck-discovery.sh [ROM-folder ...]

Read-only inventory of this Steam Deck: system, storage, installed ports, save folders,
.o2r files, Steam shortcuts, tooling and ROM folder counts. Extra arguments are ROM folders
to count (existence and file counts only). The report is saved to ~/deck-discovery-*.md.
EOF
    exit 0
    ;;
esac

if [ "$(uname -s)" != "Linux" ]; then
  echo "This script is for the Steam Deck (Linux). Nothing was done." >&2
  exit 1
fi

if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
  echo "HOME is not set to a folder. Nothing was done." >&2
  exit 1
fi

EXTRA_ROM_DIRS=()
for a in "$@"; do
  # a leading dash would otherwise be read as an option by find
  case $a in -*) a="./$a" ;; esac
  EXTRA_ROM_DIRS+=("$a")
done
STAMP=$(date +%Y%m%d-%H%M)

# ---------------------------------------------------------------- helpers

hr()  { printf '\n## %s\n\n' "$1"; }
sub() { printf '\n### %s\n\n' "$1"; }
say() { printf '%s\n' "$*"; }

human() {
  awk -v b="${1:-0}" 'BEGIN {
    split("B KB MB GB TB", u, " "); i = 1
    while (b >= 1024 && i < 5) { b /= 1024; i++ }
    if (i == 1) printf "%d %s", b, u[i]; else printf "%.1f %s", b, u[i]
  }'
}

date_of() { stat -L -c '%y' -- "$1" 2>/dev/null | cut -c1-10; }

tree_size() {
  local k
  k=$(timeout 20 du -skD -- "$1" 2>/dev/null | cut -f1)
  if [ -n "$k" ]; then human $((k * 1024)); else printf 'size unknown'; fi
}

# dir_stats <dir> [-L]: prints "<files> <bytes> <newest-epoch> <find-exit-code>".
dir_stats() {
  local d=$1 follow=${2:-} raw rc
  # shellcheck disable=SC2086  # $follow is empty or exactly -L
  raw=$(timeout 30 find $follow "$d" -type f -printf '%s %T@\n' 2>/dev/null; echo "RC:$?")
  rc=${raw##*RC:}
  raw=${raw%RC:*}
  printf '%s' "$raw" | awk -v rc="$rc" '{ n++; s += $1; if ($2 > m) m = $2 } END { printf "%d %d %d %s", n, s, m, rc }'
}

describe_dir() {
  local n bytes newest rc when="n/a" note=""
  read -r n bytes newest rc <<<"$(dir_stats "$@")"
  if [ "${n:-0}" -gt 0 ]; then when=$(date -d "@$newest" +%F 2>/dev/null || echo "n/a"); fi
  if [ "${rc:-0}" = 124 ]; then note=" (scan timed out, numbers are partial)"; fi
  printf '%s files, %s, newest file %s%s' "${n:-0}" "$(human "${bytes:-0}")" "$when" "$note"
}

# top_names <dir> [cap]: first-level entry names only, never contents.
top_names() {
  local d=$1 cap=${2:-12} names total shown
  names=$(timeout 10 find -H "$d" -mindepth 1 -maxdepth 1 -printf '%f\n' 2>/dev/null | sort)
  if [ -z "$names" ]; then printf '(empty)'; return; fi
  total=$(printf '%s\n' "$names" | grep -c .)
  shown=$(printf '%s\n' "$names" | head -n "$cap" | paste -sd'|' - | sed 's/|/, /g')
  if [ "$total" -gt "$cap" ]; then shown="$shown, +$((total - cap)) more"; fi
  printf '%s' "$shown"
}

version_hint() {
  local p=$1 b=${1##*/} v="" f line
  v=$(printf '%s' "$b" | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)*' | head -n1)
  if [ -z "$v" ] && [ -d "$p" ]; then
    for f in version VERSION version.txt VERSION.txt .version; do
      if [ -f "$p/$f" ] && [ ! -L "$p/$f" ] && [ "$(stat -c %s -- "$p/$f" 2>/dev/null || echo 99999)" -le 1024 ]; then
        line=$(head -n1 -- "$p/$f" 2>/dev/null | tr -cd '[:print:]' | cut -c1-60)
        if [ -n "$line" ]; then v="$line (first line of $f)"; break; fi
      fi
    done
  fi
  if [ -n "$v" ]; then printf '%s' "$v"; else printf 'not known from the name'; fi
}

fs_symlink_note() {
  case "$1" in
    ext2|ext3|ext4|btrfs|xfs|f2fs) printf 'symlinks: yes' ;;
    exfat|vfat|msdos|ntfs|ntfs3|fuseblk) printf 'symlinks: NO (this format cannot hold them)' ;;
    *) printf 'symlinks: unknown for this format' ;;
  esac
}

fs_line() {
  local label=$1 path=$2 fstype avail total
  fstype=$(timeout 10 df --output=fstype -- "$path" 2>/dev/null | tail -n1 | tr -d ' ')
  avail=$(timeout 10 df -h --output=avail -- "$path" 2>/dev/null | tail -n1 | tr -d ' ')
  total=$(timeout 10 df -h --output=size -- "$path" 2>/dev/null | tail -n1 | tr -d ' ')
  printf -- '- %s `%s`: %s, %s free of %s; %s\n' "$label" "$path" "${fstype:-unknown}" "${avail:-?}" "${total:-?}" "$(fs_symlink_note "${fstype:-}")"
}

# ---------------------------------------------------------------- discovery inputs

# MEDIA_ROOT can be overridden only so the script can be tested away from a Deck.
MEDIA_ROOT=${DPM_MEDIA_ROOT:-/run/media}
SD_MOUNTS=()
for m in "$MEDIA_ROOT"/* "$MEDIA_ROOT"/*/*; do
  if [ -d "$m" ] && timeout 5 mountpoint -q -- "$m" 2>/dev/null; then
    # SteamOS keeps /run/media/mmcblk0p1 as a symlink to /run/media/deck/<label>; list each card once.
    real=$(readlink -f -- "$m")
    dup=0
    for r in "${SD_MOUNTS[@]+"${SD_MOUNTS[@]}"}"; do [ "$r" = "$real" ] && dup=1; done
    # a mount inside a card that is already a mount (for example a loop mount) is not a second card
    parent=${m%/*}
    if [ "$parent" != "$MEDIA_ROOT" ] && timeout 5 mountpoint -q -- "$parent" 2>/dev/null; then dup=1; fi
    [ "$dup" = 0 ] && SD_MOUNTS+=("$real")
  fi
done

STEAM_ROOTS=()
for c in "$HOME/.local/share/Steam" "$HOME/.steam/steam" "$HOME/.var/app/com.valvesoftware.Steam/.local/share/Steam"; do
  if [ -d "$c/userdata" ]; then
    real=$(readlink -f -- "$c")
    dup=0
    for r in "${STEAM_ROOTS[@]+"${STEAM_ROOTS[@]}"}"; do [ "$r" = "$real" ] && dup=1; done
    [ "$dup" = 0 ] && STEAM_ROOTS+=("$real")
  fi
done

mask_id() {
  if [ "${#1}" -le 4 ]; then printf '****'; else printf '****%s' "${1: -4}"; fi
}

STEAM_IDS=()
for root in "${STEAM_ROOTS[@]+"${STEAM_ROOTS[@]}"}"; do
  for u in "$root"/userdata/*/; do
    id=$(basename -- "$u")
    if [[ $id =~ ^[0-9]+$ ]] && [ "$id" != 0 ]; then STEAM_IDS+=("$id"); fi
  done
done

MASK=(cat)
if [ ${#STEAM_IDS[@]} -gt 0 ]; then
  MASK=(sed -u)
  for id in "${STEAM_IDS[@]}"; do
    # Only the ID folder names are rewritten, so dates, appids and file names are never touched.
    MASK+=(-e "s#/userdata/$id\\b#/userdata/$(mask_id "$id")#g" -e "s#/Steam Controller Configs/$id\\b#/Steam Controller Configs/$(mask_id "$id")#g")
  done
fi

# ---------------------------------------------------------------- report sections

print_header() {
  say "# Steam Deck discovery report"
  say ""
  say "- Script: deck-discovery.sh $SCRIPT_VERSION"
  say "- Run: $(date '+%Y-%m-%d %H:%M') as user \`$(id -un)\`"
  say "- Report file: \`$REPORT\`"
  say ""
  say "**What this run read:** system info files (/etc/os-release, df, mount list), folder and file"
  say "names, sizes and dates under the search places (hidden folders are skipped), the Steam"
  say "\`shortcuts.vdf\` (opened read-only; the report includes each shortcut's name, program path,"
  say "start folder and launch options, with values of anything that looks like a token or key hidden)"
  say "and \`config.vdf\` (only to look up compatibility tool names), the names inside Steam's \`grid/\`"
  say "folder, the first line of tiny version files, and \`flatpak list\`. Folders it cannot read are"
  say "skipped without a message."
  say ""
  say "**What it did NOT do:** write, copy, move or delete anything except this report file; run any"
  say "port or AppImage it found; print the contents of any save, config, ROM or state file; hash or"
  say "name any ROM file; use sudo; touch the network. Reading files can update their \"last accessed\""
  say "time and nothing else."
  say ""
  say "**Masking:** Steam user IDs are shown with only the last 4 digits."
}

section_system() {
  hr "1. System"
  local osr=/etc/os-release
  if [ -r "$osr" ]; then
    local k
    for k in PRETTY_NAME VERSION_ID BUILD_ID VARIANT_ID; do
      say "- $k: $(sed -n "s/^$k=//p" "$osr" | head -n1 | tr -d '"')"
    done
  else
    say "- /etc/os-release: not readable"
  fi
  say "- Kernel: $(uname -r)"
  say "- glibc: $(getconf GNU_LIBC_VERSION 2>/dev/null || echo unknown)"
  say "- Device model code: $(cat /sys/devices/virtual/dmi/id/product_name 2>/dev/null || echo unknown) (Jupiter = LCD Deck, Galileo = OLED Deck)"
  say "- XDG_SESSION_TYPE: ${XDG_SESSION_TYPE:-unset}"
  say "- XDG_CURRENT_DESKTOP: ${XDG_CURRENT_DESKTOP:-unset}"
  say "- DISPLAY set: $([ -n "${DISPLAY:-}" ] && echo yes || echo no); WAYLAND_DISPLAY set: $([ -n "${WAYLAND_DISPLAY:-}" ] && echo yes || echo no)"
  local rootopts
  rootopts=$(findmnt -no OPTIONS / 2>/dev/null | cut -d, -f1-2)
  say "- Root filesystem mount options: ${rootopts:-unknown} (ro = read-only root, normal for SteamOS)"
  if command -v python3 >/dev/null 2>&1; then
    say "- python3: $(python3 --version 2>&1)"
  else
    say "- python3: not found"
  fi
  if [ -z "${XDG_SESSION_TYPE:-}" ]; then
    say ""
    say "Note: XDG_SESSION_TYPE is unset. This usually means the script was not started from"
    say "Konsole in Desktop Mode (for example over SSH). The rest of the report is still valid."
  fi
}

section_storage() {
  hr "2. Storage"
  fs_line "Home (internal)" "$HOME"
  if [ ${#SD_MOUNTS[@]} -eq 0 ]; then
    say "- No SD card or external drive mounted under /run/media/."
  else
    local m
    for m in "${SD_MOUNTS[@]}"; do fs_line "Mounted under /run/media" "$m"; done
  fi
  say ""
  say "Reading guide: ext4 or btrfs can hold symlinks and Linux permissions. exFAT, FAT and NTFS cannot."
  say "Hard links never work between internal storage and a card (different filesystems)."
}

PORT_NAMES=(
  -iname 'soh' -o -iname 'soh[-_.]*' -o -iname '*shipwright*' -o -iname '*harkinian*'
  -o -iname '2s2h*' -o -iname '2ship*'
  -o -iname '*dusklight*' -o -iname '*twilit*realm*'
  -o -iname '*skyrecomp*' -o -iname '*severed*chain*' -o -iname '*courage*reborn*'
  -o -iname 'ports_launcher*' -o -iname 'ports.json' -o -iname 'ports.local.json'
  -o -iname '*.appimage' -o -iname '*.o2r' -o -iname '*.otr'
)

# run_find: find with a 60 s cap. If the cap is hit it appends a marker entry so callers can say
# the list is partial instead of silently reporting "nothing found".
TIMEOUT_MARKER="::scan-timed-out::"
run_find() {
  timeout 60 find "$@" 2>/dev/null
  # The leading NUL ends a path that find was cut off in the middle of.
  if [ $? -eq 124 ]; then printf '\0%s\0' "$TIMEOUT_MARKER"; fi
}

scan_ports() {
  run_find -H "$1" -xdev -maxdepth "$2" \
    \( -name '.*' -o -iname roms -o -iname steamapps -o -iname node_modules \
       -o -name compatdata -o -name shadercache \) -prune \
    -o \( "${PORT_NAMES[@]}" \) -print0 2>/dev/null
}

report_hit() {
  local p=$1 link="" size kind exec_note="" sib
  if [ -L "$p" ]; then link=" (symlink to \`$(readlink -- "$p")\`)"; fi
  if [ -d "$p" ]; then
    kind="folder"; size=$(tree_size "$p")
  else
    kind="file"; size=$(human "$(stat -L -c %s -- "$p" 2>/dev/null || echo 0)")
    if [ -x "$p" ]; then exec_note=", executable bit set"; fi
  fi
  say "- \`$p\`$link"
  say "  - $kind, $size, modified $(date_of "$p")$exec_note"
  say "  - version: $(version_hint "$p")"
  case "${p,,}" in
    *.appimage)
      for sib in "$p.home" "$p.config"; do
        if [ -d "$sib" ]; then say "  - portable data folder next to it: \`$sib\` ($(describe_dir "$sib"))"; fi
      done
      ;;
  esac
  if [ -d "$p" ]; then
    local sd
    for sd in saves Saves save Save gamedata GameData mods; do
      if [ -d "$p/$sd" ]; then say "  - has \`$sd/\`: $(describe_dir "$p/$sd")"; fi
    done
  fi
}

report_ports_launcher_dir() {
  local d=$1 f
  say "- \`$d\`"
  for f in ports.json ports.local.json state.json themes.json; do
    if [ -f "$d/$f" ]; then
      say "  - \`$f\`: $(human "$(stat -c %s -- "$d/$f" 2>/dev/null || echo 0)"), modified $(date_of "$d/$f") (contents not read)"
    fi
  done
  if [ -d "$d/Library" ]; then
    say "  - \`Library/\` ($(describe_dir "$d/Library")): $(top_names "$d/Library" 40)"
  else
    say "  - no \`Library/\` folder"
  fi
  if [ -d "$d/Saves Backup" ]; then
    say "  - \`Saves Backup/\`: $(describe_dir "$d/Saves Backup")"
  fi
}

O2R_FOUND=()

section_ports() {
  hr "3. Installed ports and AppImages"
  say "Searched (depth limited, hidden folders skipped, nothing executed): \`$HOME\`, \`~/Applications\`,"
  say "\`~/Games\`, \`~/Desktop\`, and every card or drive under /run/media/."
  say ""

  local hits=() p d
  mapfile -d '' hits < <(
    {
      scan_ports "$HOME" 5
      for d in Applications Games Desktop; do
        if [ -d "$HOME/$d" ]; then scan_ports "$HOME/$d" 7; fi
      done
      for d in "${SD_MOUNTS[@]+"${SD_MOUNTS[@]}"}"; do scan_ports "$d" 4; done
    } | sort -zu
  )

  local classes=("Ship of Harkinian" "2 Ship 2 Harkinian" "Dusklight" "SkyRecomp" "Severed Chains" "Courage Reborn" "Ports Launcher program" "Other AppImage")
  local hit_class=() hit_path=() lb partial=0 skipped=0
  declare -A PL_DIRS=()
  for p in "${hits[@]+"${hits[@]}"}"; do
    if [ "$p" = "$TIMEOUT_MARKER" ]; then partial=1; continue; fi
    [ -e "$p" ] || [ -L "$p" ] || continue
    case $p in *$'\n'*|*$'\r'*) skipped=$((skipped + 1)); continue ;; esac
    lb=$(printf '%s' "${p##*/}" | tr '[:upper:]' '[:lower:]')
    case $lb in
      *.appimage.home|*.appimage.config) continue ;;
      *.o2r|*.otr) O2R_FOUND+=("$p"); continue ;;
      ports.json|ports.local.json) PL_DIRS["${p%/*}"]=1; continue ;;
      ports_launcher*)
        if [ -f "${p%/*}/ports.json" ] || [ -f "${p%/*}/state.json" ]; then PL_DIRS["${p%/*}"]=1; fi
        hit_class+=("Ports Launcher program") ;;
      2s2h*|2ship*|*2ship2harkinian*|*2*ship*2*harkinian*) hit_class+=("2 Ship 2 Harkinian") ;;
      soh|soh[-_.]*|*shipwright*|*harkinian*) hit_class+=("Ship of Harkinian") ;;
      *dusklight*|*twilit*realm*) hit_class+=("Dusklight") ;;
      *skyrecomp*) hit_class+=("SkyRecomp") ;;
      *severed*chain*) hit_class+=("Severed Chains") ;;
      *courage*reborn*) hit_class+=("Courage Reborn") ;;
      *.appimage) hit_class+=("Other AppImage") ;;
      *) continue ;;
    esac
    hit_path+=("$p")
  done

  local c i shown cap=40
  for c in "${classes[@]}"; do
    sub "$c"
    shown=0
    for i in "${!hit_path[@]}"; do
      if [ "${hit_class[$i]}" = "$c" ]; then
        if [ "$shown" -ge "$cap" ]; then continue; fi
        report_hit "${hit_path[$i]}"
        shown=$((shown + 1))
      fi
    done
    if [ "$shown" -eq 0 ]; then say "Nothing found."; fi
    if [ "$shown" -ge "$cap" ]; then say "(list cut at $cap entries)"; fi
  done

  sub "Ports Launcher folders"
  if [ ${#PL_DIRS[@]} -eq 0 ]; then
    say "Nothing found."
  else
    for d in "${!PL_DIRS[@]}"; do report_ports_launcher_dir "$d"; done
  fi

  if [ "$partial" = 1 ]; then
    say ""
    say "**Warning:** at least one search hit its 60 second limit, so this list may be incomplete."
  fi
  if [ "$skipped" -gt 0 ]; then
    say ""
    say "Note: $skipped match(es) were left out because the name contains a line break."
  fi
}

section_save_folders() {
  hr "4. Save and config folders"
  say "Names, file counts and sizes only. No file contents are read."
  say ""
  local fixed=(
    .local/share/soh .config/soh .local/share/2s2h .config/2s2h
    .local/share/TwilitRealm .config/TwilitRealm .local/share/dusklight .config/dusklight
    .local/share/SkyRecomp .config/SkyRecomp .local/share/skyrecomp
    .local/share/SeveredChains .config/SeveredChains
    .local/share/CourageReborn .config/CourageReborn
    .config/Zelda64Recompiled .local/share/Zelda64Recompiled
  )
  local seen=() missing=() rel d found
  for rel in "${fixed[@]}"; do
    d="$HOME/$rel"
    if [ -e "$d" ]; then seen+=("$d"); else missing+=("$rel"); fi
  done
  # Anything else under .local/share or .config whose name looks like one of these ports.
  local extra
  while IFS= read -r -d '' extra; do
    found=0
    for d in "${seen[@]+"${seen[@]}"}"; do [ "$d" = "$extra" ] && found=1; done
    if [ "$found" = 0 ]; then seen+=("$extra"); fi
  done < <(find -H "$HOME/.local/share" "$HOME/.config" -mindepth 1 -maxdepth 1 \
            \( -iname '*soh*' -o -iname '*2s2h*' -o -iname '*2ship*' -o -iname '*harkinian*' \
               -o -iname '*shipwright*' -o -iname '*dusklight*' -o -iname '*twilit*' \
               -o -iname '*skyrecomp*' -o -iname '*severed*' -o -iname '*courage*' \
               -o -iname '*recomp*' -o -iname '*zelda*' -o -iname '*ports*launcher*' \) \
            -print0 2>/dev/null | sort -z)

  if [ ${#seen[@]} -eq 0 ]; then
    say "No matching save or config folders found."
  else
    for d in "${seen[@]}"; do
      local link=""
      if [ -L "$d" ]; then link=" (symlink to \`$(readlink -- "$d")\`)"; fi
      if [ -d "$d" ]; then
        say "- \`$d\`$link"
        say "  - $(describe_dir "$d" -L)"
        say "  - top level: $(top_names "$d" 12)"
      else
        say "- \`$d\`$link: exists but is not a folder"
      fi
    done
  fi
  say ""
  say "Checked and not present: ${missing[*]+"${missing[*]}"}"
}

section_o2r() {
  hr "5. .o2r and .otr files"
  say "Names, sizes and locations only."
  say ""
  local extra=() all=() p
  mapfile -d '' extra < <(
    run_find -H "$HOME/.local/share" "$HOME/.config" -maxdepth 6 \
      \( -name Steam -o -name flatpak -o -name Trash -o -name containers -o -name compatdata \) -prune \
      -o \( -iname '*.o2r' -o -iname '*.otr' \) -type f -print0
  )
  mapfile -d '' all < <(printf '%s\0' "${O2R_FOUND[@]+"${O2R_FOUND[@]}"}" "${extra[@]+"${extra[@]}"}" | sort -zu)
  local n=0 cap=80 partial=0
  for p in "${all[@]+"${all[@]}"}"; do
    [ -n "$p" ] || continue
    if [ "$p" = "$TIMEOUT_MARKER" ]; then partial=1; continue; fi
    [ -e "$p" ] || continue
    case $p in *$'\n'*|*$'\r'*) continue ;; esac
    if [ "$n" -ge "$cap" ]; then say "(list cut at $cap entries)"; break; fi
    say "- \`${p##*/}\`: $(human "$(stat -L -c %s -- "$p" 2>/dev/null || echo 0)"), modified $(date_of "$p"), in \`${p%/*}\`"
    n=$((n + 1))
  done
  if [ "$n" -eq 0 ]; then say "None found."; fi
  if [ "$partial" = 1 ]; then say ""; say "**Warning:** the search hit its 60 second limit, so this list may be incomplete."; fi
}

PYSRC=''
read -r -d '' PYSRC <<'PY'
import os, re, struct, sys, time, zlib

sys.stdout.reconfigure(encoding="utf-8", errors="replace")
vdf_path, grid_dir, cfg_path = sys.argv[1:4]

PORTS = [
    ("2 Ship 2 Harkinian", r"2s2h|2.?ship"),
    ("Ship of Harkinian", r"\bsoh\b|shipwright|ship.?of.?harkinian"),
    ("Dusklight", r"dusklight|twilit.?realm"),
    ("SkyRecomp", r"skyrecomp"),
    ("Severed Chains", r"severed.?chains?"),
    ("Courage Reborn", r"courage.?reborn"),
]

state = {"truncated": False, "error": None}


def parse(data):
    pos = 0
    n = len(data)

    def cstr():
        nonlocal pos
        end = data.find(b"\x00", pos)
        if end < 0:
            raise EOFError
        s = data[pos:end].decode("utf-8", "replace")
        pos = end + 1
        return s

    def num(fmt):
        nonlocal pos
        size = struct.calcsize(fmt)
        if pos + size > n:
            raise EOFError
        v = struct.unpack_from(fmt, data, pos)[0]
        pos += size
        return v

    def obj():
        nonlocal pos
        items = []
        try:
            while True:
                if pos >= n:
                    raise EOFError
                t = data[pos]
                pos += 1
                if t == 8:
                    return items
                key = cstr()
                if t == 0:
                    val = obj()
                elif t == 1:
                    val = cstr()
                elif t == 2:
                    val = num("<i")
                elif t == 3:
                    val = num("<f")
                elif t in (4, 6):
                    val = num("<I")
                elif t == 5:
                    end = data.find(b"\x00\x00", pos)
                    while end >= 0 and (end - pos) % 2:
                        end = data.find(b"\x00\x00", end + 1)
                    if end < 0:
                        raise EOFError
                    val = data[pos:end].decode("utf-16-le", "replace")
                    pos = end + 2
                elif t == 7:
                    val = num("<Q")
                elif t == 10:
                    val = num("<q")
                else:
                    state["error"] = "unknown value type 0x%02x at byte %d" % (t, pos - 1)
                    return items
                items.append((key, val))
                if state["error"]:
                    return items
        except EOFError:
            state["truncated"] = True
            return items

    return obj()


def field(items, name):
    for k, v in items:
        if k.lower() == name:
            return v
    return None


def compat_tools():
    try:
        with open(cfg_path, "r", errors="replace") as fh:
            txt = fh.read()
    except OSError:
        return None
    i = txt.find('"CompatToolMapping"')
    if i < 0:
        return {}
    j = txt.find("{", i)
    if j < 0:
        return {}
    depth, k = 0, j
    while k < len(txt):
        if txt[k] == "{":
            depth += 1
        elif txt[k] == "}":
            depth -= 1
            if depth == 0:
                break
        k += 1
    return dict(re.findall(r'"(-?\d+)"\s*\{\s*"name"\s*"([^"]*)"', txt[j:k]))


try:
    with open(vdf_path, "rb") as fh:
        data = fh.read()
except OSError as exc:
    print("Could not open shortcuts.vdf for reading: %s" % exc)
    sys.exit(0)

if not data:
    print("The file is empty (0 bytes), so there are no shortcuts.")
    sys.exit(0)

root = parse(data)
if state["truncated"]:
    print("Warning: the file ended early (Steam may have been writing it). Showing what was readable.")
if state["error"]:
    print("Warning: parse stopped: %s. Showing what was readable." % state["error"])

shortcuts = field(root, "shortcuts")
if not isinstance(shortcuts, list):
    print("No `shortcuts` block found in the file (%d bytes)." % len(data))
    sys.exit(0)

try:
    grid_files = sorted(os.listdir(grid_dir))
except OSError:
    grid_files = None
compat = compat_tools()
KIND = {"": "grid", "p": "portrait", "_hero": "hero", "_logo": "logo", "_icon": "icon"}
known = set()

def clean(s):
    return re.sub(r"[\x00-\x1f\x7f]", "?", s)


def text(entry, key):
    v = field(entry, key)
    return clean(v if isinstance(v, str) else ("" if v is None else str(v)))


SENSITIVE = re.compile(r"(\b[A-Za-z0-9_]*(?:TOKEN|KEY|SECRET|PASS|PASSWORD|AUTH|CRED)[A-Za-z0-9_]*=)(?:\"[^\"]*\"|'[^']*'|\S+)", re.I)


def show(pos, idx, entry):
    name = text(entry, "appname")
    exe = text(entry, "exe")
    startdir = text(entry, "startdir")
    opts = SENSITIVE.sub(r"\1<redacted>", text(entry, "launchoptions"))[:300]
    icon = text(entry, "icon")
    stored = field(entry, "appid")
    tags_block = field(entry, "tags")
    tags = [clean(str(v)) for _, v in tags_block] if isinstance(tags_block, list) else []
    played = field(entry, "lastplaytime")

    unquoted = exe[1:-1] if len(exe) >= 2 and exe[0] == exe[-1] == '"' else exe
    derived = zlib.crc32((exe + name).encode("utf-8")) | 0x80000000
    derived_unq = zlib.crc32((unquoted + name).encode("utf-8")) | 0x80000000

    blob = (name + " " + exe + " " + startdir + " " + opts).lower()
    looks = [label for label, rx in PORTS if re.search(rx, blob)]
    managed = []
    if re.search(r"emulation[/\\]|emudeck", blob):
        managed.append("EmuDeck")
    if re.search(r"steam-?rom-?manager|srm", blob):
        managed.append("Steam ROM Manager")

    print("")
    print("#### Shortcut %d: %s" % (pos, name or "(no name)"))
    print("- Index key in file: %s" % clean(str(idx)))
    print("- AppName: %s" % (name or "(empty)"))
    print("- Exe: `%s`" % exe)
    print("- StartDir: `%s`" % startdir)
    print("- LaunchOptions: %s" % (("`%s`" % opts) if opts else "(none)"))
    if icon:
        print("- Icon: `%s`" % icon)
    ids = []
    if isinstance(stored, int):
        uns = stored & 0xFFFFFFFF
        ids.append(uns)
        if uns == derived:
            how = "matches crc32(Exe + AppName) with Exe exactly as stored"
        elif uns == derived_unq:
            how = "matches crc32(Exe + AppName) with the quotes removed from Exe"
        else:
            how = "does NOT match crc32(Exe + AppName); Steam or another tool set it another way"
        print("- appid: %d (stored)" % uns)
        print("- appid check: %s" % how)
    else:
        print("- appid: none stored. Steam would derive %d from Exe + AppName (%d if quotes are removed)." % (derived, derived_unq))
    ids.append(derived)
    known.update(str(i) for i in ids)
    big = None
    if isinstance(stored, int):
        big = ((stored & 0xFFFFFFFF) << 32) | 0x02000000
        known.add(str(big))
    print("- tags: %s" % (", ".join(tags) if tags else "none"))
    if isinstance(played, int) and played > 0:
        print("- last played: %s" % time.strftime("%Y-%m-%d", time.localtime(played)))
    else:
        print("- last played: never or not recorded")
    if compat is None:
        print("- compatibility tool (Proton): config.vdf not readable")
    else:
        tool = None
        if isinstance(stored, int):
            tool = compat.get(str(stored & 0xFFFFFFFF)) or compat.get(str(stored))
        print("- compatibility tool (Proton): %s" % (tool if tool else "none set"))
    if looks:
        print("- looks like port: %s" % ", ".join(looks))
    if managed:
        print("- may be managed by: %s (its next run could rewrite this entry)" % ", ".join(managed))
    if grid_files is None:
        print("- art in grid/: grid folder not found")
    else:
        cand = [str(i) for i in ids] + ([str(big)] if big else [])
        rx = re.compile(r"^(%s)(p|_hero|_logo|_icon)?\.(png|jpe?g|webp|ico|svg|gif)$" % "|".join(cand))
        kinds = sorted({KIND[m.group(2) or ""] for m in (rx.match(f) for f in grid_files) if m})
        print("- art in grid/: %s" % (", ".join(kinds) if kinds else "none found"))

print("%d shortcut(s) in the file." % len(shortcuts))
for pos, (idx, entry) in enumerate(shortcuts, 1):
    if not isinstance(entry, list):
        continue
    try:
        show(pos, idx, entry)
    except Exception as exc:
        print("")
        print("#### Shortcut %d: could not be read (%s)" % (pos, exc))

if grid_files is not None:
    stray =[f for f in grid_files if not any(f.startswith(k) for k in known)]
    print("")
    print("grid/ holds %d file(s); %d do not belong to any shortcut listed above." % (len(grid_files), len(stray)))
PY

section_steam() {
  hr "6. Steam and shortcuts"
  if pgrep -x steam >/dev/null 2>&1; then
    say "- Steam is running now. (Normal; Steam rewrites shortcuts.vdf when it exits, so the future app must not write while it runs.)"
  else
    say "- Steam is not running now."
  fi
  say "- gamescope running (Game Mode or nested session): $(pgrep -x gamescope >/dev/null 2>&1 && echo yes || echo no)"
  if [ ${#STEAM_ROOTS[@]} -eq 0 ]; then
    say "- No Steam folder with a userdata/ folder was found."
    return
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    say "- python3 not found, so shortcuts.vdf cannot be parsed. File sizes and dates are listed only."
  fi
  local root ud id vdf grid cfg
  for root in "${STEAM_ROOTS[@]}"; do
    say "- Steam folder: \`$root\`"
    for ud in "$root"/userdata/*/; do
      ud=${ud%/}
      id=${ud##*/}
      if ! [[ $id =~ ^[0-9]+$ ]] || [ "$id" = 0 ]; then continue; fi
      cfg="$ud/config"
      vdf="$cfg/shortcuts.vdf"
      grid="$cfg/grid"
      sub "Steam user $(mask_id "$id")"
      if [ -f "$vdf" ]; then
        say "- \`$vdf\`: $(human "$(stat -c %s -- "$vdf" 2>/dev/null || echo 0)"), modified $(date_of "$vdf")"
        if command -v python3 >/dev/null 2>&1; then
          python3 -I -B -c "$PYSRC" "$vdf" "$grid" "$root/config/config.vdf" 2>&1
        fi
      else
        say "- No shortcuts.vdf for this user (no Non-Steam shortcuts added yet)."
      fi
    done
    local cc
    for cc in "$root"/steamapps/common/"Steam Controller Configs"/*/config; do
      if [ -d "$cc" ]; then
        sub "Steam Input layout folders (named by game or shortcut)"
        say "- $(top_names "$cc" 40)"
      fi
    done
  done
}

tool_probe() {
  local label=$1; shift
  local fixed=() pats=() mode=fixed a f
  for a in "$@"; do
    if [ "$a" = "--" ]; then mode=pats; continue; fi
    if [ "$mode" = fixed ]; then fixed+=("$a"); else pats+=("$a"); fi
  done
  local out=() roots=("$HOME" "${SD_MOUNTS[@]+"${SD_MOUNTS[@]}"}")
  for f in "${fixed[@]+"${fixed[@]}"}"; do
    if [ -e "$f" ]; then out+=("$f"); fi
  done
  if [ ${#pats[@]} -gt 0 ]; then
    local expr=() p r
    for p in "${pats[@]}"; do
      if [ ${#expr[@]} -gt 0 ]; then expr+=(-o); fi
      expr+=(-iname "$p")
    done
    for r in "${roots[@]}"; do
      while IFS= read -r -d '' f; do out+=("$f"); done < <(
        timeout 30 find -H "$r" -xdev -maxdepth 3 \
          \( -name '.*' -o -iname roms -o -iname steamapps \) -prune \
          -o \( "${expr[@]}" \) -print0 2>/dev/null | sort -z
      )
    done
  fi
  if [ ${#out[@]} -eq 0 ]; then
    say "- $label: not found"
  else
    say "- $label: found"
    for f in "${out[@]}"; do say "  - \`$f\` (modified $(date_of "$f"))"; done
  fi
}

section_tooling() {
  hr "7. Tooling that matters for later decisions"

  sub "Decky Loader"
  if [ -d "$HOME/homebrew" ]; then
    say "- \`~/homebrew\` exists."
    local svc
    svc=$(systemctl is-active plugin_loader 2>/dev/null)
    say "- plugin_loader service: ${svc:-unknown or not installed}"
    if [ -d "$HOME/homebrew/plugins" ]; then
      say "- plugins installed: $(top_names "$HOME/homebrew/plugins" 40)"
    else
      say "- no plugins folder"
    fi
  else
    say "- Decky Loader: not found (\`~/homebrew\` missing)"
  fi

  sub "Other Steam and shortcut tools"
  tool_probe "Steam ROM Manager" "$HOME/.config/steam-rom-manager" -- '*steam*rom*manager*'
  tool_probe "EmuDeck" "$HOME/Emulation" "$HOME/.config/EmuDeck" "$HOME/emudeck" -- '*emudeck*'
  tool_probe "BoilR" "$HOME/.config/boilr" "$HOME/.local/share/boilr" -- '*boilr*'
  tool_probe "SteamGridDB tools" -- '*steamgriddb*'
  local pdir
  for pdir in "$HOME/.steam/root/compatibilitytools.d" "$HOME/.local/share/Steam/compatibilitytools.d"; do
    if [ -d "$pdir" ]; then say "- Extra Proton builds in \`$pdir\`: $(top_names "$pdir" 20)"; fi
  done

  sub "Flatpak apps"
  if command -v flatpak >/dev/null 2>&1; then
    # flatpak list is read-only; on a host with no user installation it may create an empty
    # ~/.local/share/flatpak. Every Deck already has one.
    local apps total relevant
    apps=$(timeout 30 flatpak list --app --columns=application,version 2>/dev/null)
    total=$(printf '%s\n' "$apps" | grep -c .)
    relevant=$(printf '%s\n' "$apps" | grep -iE 'steam|rom|proton|emu|retroarch|heroic|lutris|bottles|boilr|grid|deck|appimage|dolphin|duckstation|ppsspp|cemu|rpcs3|ryujinx|yuzu|citra|pcsx' || true)
    say "- $total Flatpak app(s) installed. Relevant ones:"
    if [ -n "$relevant" ]; then printf '%s\n' "$relevant" | sed 's/^/  - /'; else say "  - none"; fi
  else
    say "- flatpak command not found"
  fi

  sub "Build and scripting tools on the Deck"
  local t
  for t in python3 curl git unzip 7z tar rustup cargo podman distrobox; do
    say "- $t: $(command -v "$t" 2>/dev/null || echo 'not found')"
  done
  if [ -d "$HOME/.cargo" ]; then say "- \`~/.cargo\` exists"; fi
  if [ -d "$HOME/.rustup" ]; then say "- \`~/.rustup\` exists"; fi
}

rom_folder_line() {
  local d=$1 label=${2:-}
  if [ -d "$d" ]; then
    say "- ${label:-\`$d\`}: $(describe_dir "$d" -L)"
  elif [ -e "$d" ]; then
    say "- \`$d\`: exists but is not a folder"
  else
    say "- \`$d\`: not found"
  fi
}

section_roms() {
  hr "8. ROM folders"
  say "Existence and file counts only. No file names are listed and nothing is hashed."
  say ""
  local base sys shown
  local bases=("$HOME/Emulation/roms" "$HOME/ROMs" "$HOME/Roms" "$HOME/roms" "$HOME/Games/roms")
  for base in "${SD_MOUNTS[@]+"${SD_MOUNTS[@]}"}"; do
    bases+=("$base/Emulation/roms" "$base/roms" "$base/ROMs")
  done
  for base in "${bases[@]}"; do
    if [ -d "$base" ]; then
      say "- \`$base\`"
      shown=0
      for sys in "$base"/*/; do
        [ -d "$sys" ] || continue
        sys=${sys%/}
        if [ "$shown" -ge 60 ]; then say "  - (list cut at 60 folders)"; break; fi
        say "  - ${sys##*/}: $(describe_dir "$sys" -L)"
        shown=$((shown + 1))
      done
      if [ "$shown" -eq 0 ]; then say "  - (no subfolders)"; fi
    fi
  done
  say ""
  if [ ${#EXTRA_ROM_DIRS[@]} -gt 0 ]; then
    say "Folders passed on the command line:"
    for base in "${EXTRA_ROM_DIRS[@]}"; do rom_folder_line "$base"; done
  else
    say "No extra folders were passed. To count another one, run the script again with its path:"
    say "\`bash deck-discovery.sh /run/media/mmcblk0p1/MyGames\`"
  fi
}

section_footer() {
  hr "9. End of report"
  say "Finished $(date '+%Y-%m-%d %H:%M'). Nothing on this Deck was changed; this report is the only new file."
}

main() {
  print_header
  section_system
  section_storage
  section_ports
  section_save_folders
  section_o2r
  section_steam
  section_tooling
  section_roms
  section_footer
}

# ---------------------------------------------------------------- run

REPORT=""
n=0
while [ "$n" -lt 50 ]; do
  cand="$HOME/deck-discovery-$STAMP.md"
  if [ "$n" -gt 0 ]; then cand="$HOME/deck-discovery-$STAMP-$n.md"; fi
  if (set -C; : >"$cand") 2>/dev/null; then REPORT=$cand; break; fi
  n=$((n + 1))
done

if [ -n "$REPORT" ]; then
  main 2>&1 | "${MASK[@]}" | tee "$REPORT"
  echo
  echo "Report saved to: $REPORT"
else
  REPORT="(could not create a report file in $HOME; output is on screen only)"
  main 2>&1 | "${MASK[@]}"
fi
