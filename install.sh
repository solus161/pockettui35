#!/usr/bin/env bash
# pockettui35 installer: the launcher on tty1 of a PocketTerm35.
#
#   sudo ./install.sh [flags]          from a checkout
#
# Flags:
#   --dry-run      print what would happen, change nothing (no root needed)
#   --uninstall    remove pockettui35 and undo the session setup
#   --no-build     install target/release/pockettui35 as it is
#   --user NAME    the session user (default: $SUDO_USER)
#   --force        install on a machine that is not a Raspberry Pi
#
# Needs the patched keyboard firmware (F13-F22 on the buttons); flash it
# with pt35-kbd from pocketterm35-os first.
set -euo pipefail

SHARE=/usr/share/pockettui35
CMDLINE=/boot/firmware/cmdline.txt
BOOTCFG=/boot/firmware/config.txt
MARK_BEGIN='# pockettui35 >>>'
MARK_END='# pockettui35 <<<'
OLD_KEYD=/etc/keyd/pocketterm35.conf
DRY=0; UNINSTALL=0; BUILD=1; FORCE=0; TARGET_USER="${SUDO_USER:-}"

msg()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[!]\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[x]\033[0m %s\n' "$*" >&2; exit 1; }
run()  { if [ "$DRY" = 1 ]; then printf '    would run: %s\n' "$*"; else eval "$*"; fi; }

while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run)   DRY=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --no-build)  BUILD=0 ;;
    --force)     FORCE=1 ;;
    --user)      shift; TARGET_USER="${1:-}" ;;
    --user=*)    TARGET_USER="${1#--user=}" ;;
    -h|--help)   sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)           die "unknown flag: $1 (see --help)" ;;
  esac
  shift
done

SRC=$(cd "$(dirname "$0")" && pwd)
[ "$DRY" = 1 ] || [ "$(id -u)" = 0 ] || die "run as root: sudo $0 $*"
[ -n "$TARGET_USER" ] || die "no session user: pass --user NAME"
id "$TARGET_USER" >/dev/null 2>&1 || die "no such user: $TARGET_USER"
[ "$TARGET_USER" != root ] || die "the session user cannot be root"

# --------------------------------------------------------------- uninstall
if [ "$UNINSTALL" = 1 ]; then
  msg "removing pockettui35"
  run "rm -f /usr/bin/pockettui35 /usr/bin/pockettui35-session"
  run "rm -rf '$SHARE'"
  run "rm -f /etc/sudoers.d/pockettui35 /etc/profile.d/pockettui35.sh"
  run "rm -f /etc/systemd/system/getty@tty1.service.d/pockettui35-autologin.conf"
  run "rmdir --ignore-fail-on-non-empty /etc/systemd/system/getty@tty1.service.d 2>/dev/null || true"
  run "rm -f /etc/keyd/pockettui35.conf"
  if [ -f "$OLD_KEYD.pockettui35-off" ]; then
    msg "putting back $OLD_KEYD"
    run "mv '$OLD_KEYD.pockettui35-off' '$OLD_KEYD'"
  fi
  if [ -f "$CMDLINE" ] && grep -q 'vt.global_cursor_default=0' "$CMDLINE"; then
    run "sed -i '1s/ vt.global_cursor_default=0//' '$CMDLINE'"
  fi
  run "systemctl daemon-reload"
  run "keyd reload 2>/dev/null || true"
  # The touchscreen setup is the device's, not the launcher's: it stays.
  if [ -f "$BOOTCFG" ] && grep -qxF "$MARK_BEGIN" "$BOOTCFG"; then
    msg "the touch overlay stays in $BOOTCFG (between the '$MARK_BEGIN' lines)"
  fi
  msg "done. tty1 is a plain login again after a reboot."
  msg "a display manager disabled by the install (greetd, lightdm) stays off:"
  msg "  sudo systemctl enable greetd   # or lightdm, and set-default graphical.target"
  exit 0
fi

# ----------------------------------------------------------------- install
# This takes over tty1 and turns display managers off: not for a desktop.
if ! tr -d '\0' < /proc/device-tree/model 2>/dev/null | grep -q 'Raspberry Pi'; then
  [ "$FORCE" = 1 ] || [ "$DRY" = 1 ] \
    || die "this is not a Raspberry Pi; pass --force if you mean it"
  warn "not a Raspberry Pi"
fi

msg "installing packages"
run "DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      cage keyd pipewire pipewire-alsa wireplumber"

if [ "$BUILD" = 1 ]; then
  # A login shell, so a rustup toolchain in ~/.cargo/bin wins over Debian's.
  if [ "$DRY" = 0 ]; then
    su -l "$TARGET_USER" -c 'command -v cargo >/dev/null' \
      || die "cargo is not installed for '$TARGET_USER'; install rustup first"
  fi
  msg "building (a few minutes on a Pi 4)"
  run "su -l '$TARGET_USER' -c 'cd \"$SRC\" && cargo build --release'"
fi
[ "$DRY" = 1 ] || [ -x "$SRC/target/release/pockettui35" ] \
  || die "no target/release/pockettui35; build it or drop --no-build"

msg "installing files"
run "install -m755 '$SRC/target/release/pockettui35' /usr/bin/pockettui35"
run "install -m755 '$SRC/assets/bin/pockettui35-session' /usr/bin/pockettui35-session"
run "install -d '$SHARE' && install -m644 '$SRC/assets/config.toml' '$SHARE/config.toml'"

msg "sudoers rule (power profile, button layers)"
run "visudo -cf '$SRC/assets/sudoers.d/pockettui35' >/dev/null"
run "install -m440 '$SRC/assets/sudoers.d/pockettui35' /etc/sudoers.d/pockettui35"

msg "keyd"
# A device id may be in one keyd config only.
if [ -f "$OLD_KEYD" ] && grep -q '1209:0001' "$OLD_KEYD"; then
  warn "$OLD_KEYD maps the same keyboard; moving it aside"
  run "mv '$OLD_KEYD' '$OLD_KEYD.pockettui35-off'"
fi
run "install -d /etc/keyd && install -m644 '$SRC/assets/keyd/pockettui35.conf' /etc/keyd/pockettui35.conf"
run "systemctl enable keyd >/dev/null 2>&1"
run "systemctl restart keyd"

msg "session for $TARGET_USER on tty1"
# video: the sudoers rule and DRM; dialout: the keyboard's serial console
# (brightness); input, render, audio: cage and PipeWire.
run "usermod -aG video,dialout,input,render,audio '$TARGET_USER'"
run "install -d /etc/systemd/system/getty@tty1.service.d"
run "sed 's/@USER@/$TARGET_USER/' '$SRC/assets/systemd/getty@tty1.service.d/pockettui35-autologin.conf' \
      > /etc/systemd/system/getty@tty1.service.d/pockettui35-autologin.conf"
run "sed 's/@USER@/$TARGET_USER/' '$SRC/assets/profile.d/pockettui35.sh' > /etc/profile.d/pockettui35.sh"

# Something else owning tty1 would fight the launcher for it.
for dm in greetd lightdm gdm3 sddm; do
  if systemctl is-enabled "$dm" >/dev/null 2>&1; then
    warn "$dm is enabled and would take the screen; disabling it"
    run "systemctl disable '$dm' >/dev/null 2>&1"
  fi
done
run "systemctl set-default multi-user.target >/dev/null"
run "systemctl daemon-reload"

# No blinking console cursor: its timer would wake the CPU twice a second.
if [ -f "$CMDLINE" ]; then
  if ! grep -q 'vt.global_cursor_default=0' "$CMDLINE"; then
    msg "kernel command line: vt.global_cursor_default=0"
    run "[ -e '$CMDLINE.pockettui35.bak' ] || cp '$CMDLINE' '$CMDLINE.pockettui35.bak'"
    run "sed -i '1s/\$/ vt.global_cursor_default=0/' '$CMDLINE'"
  fi
else
  warn "no $CMDLINE: add vt.global_cursor_default=0 to the kernel command line by hand"
fi

# Touchscreen (GT911) and USB host for the RP2040 keyboard. The 5b overlay
# is right on the Pi 4B too: Waveshare's overlays differ by the touch
# controller's I2C address (PocketTerm35 answers at 0x5d, only 5b has it),
# not by board. Lines already in effect are left alone; the rest go in one
# marked [all] block, so a board filter such as [cm5] cannot swallow them.
if [ -f "$BOOTCFG" ]; then
  msg "boot config: touchscreen overlay"
  run "(cd '$SRC/assets/boot' && sha256sum -c --quiet SHA256SUMS)"
  run "install -m644 '$SRC/assets/boot/waveshare-35dpi-5b.dtbo' '$(dirname "$BOOTCFG")/overlays/'"
  # What the file says without our own block, as a reinstall would leave it.
  outside=$(sed "/^$MARK_BEGIN\$/,/^$MARK_END\$/d" "$BOOTCFG")
  want=()
  for line in dtparam=i2c_arm=on dtoverlay=waveshare-35dpi-5b dtoverlay=dwc2,dr_mode=host; do
    printf '%s\n' "$outside" | grep -qx "[[:space:]]*$line[[:space:]]*" || want+=("$line")
  done
  # The 3b and 4b overlays probe 0x14 only; next to 5b they would claim the
  # same bus for nothing.
  if printf '%s\n' "$outside" | grep -qE '^dtoverlay=waveshare-35dpi-(3b|4b)'; then
    warn "a 3b/4b Waveshare overlay is set; commenting it out for 5b"
    run "[ -e '$BOOTCFG.pockettui35.bak' ] || cp '$BOOTCFG' '$BOOTCFG.pockettui35.bak'"
    run "sed -i -E 's/^dtoverlay=waveshare-35dpi-(3b|4b)/#pockettui35-off# &/' '$BOOTCFG'"
  fi
  if grep -qxF "$MARK_BEGIN" "$BOOTCFG" || [ ${#want[@]} -gt 0 ]; then
    run "[ -e '$BOOTCFG.pockettui35.bak' ] || cp '$BOOTCFG' '$BOOTCFG.pockettui35.bak'"
    run "sed -i '/^$MARK_BEGIN\$/,/^$MARK_END\$/d' '$BOOTCFG'"
  fi
  if [ ${#want[@]} -gt 0 ]; then
    if [ "$DRY" = 1 ]; then
      printf '    would append to %s:\n' "$BOOTCFG"
      printf '      %s\n' "$MARK_BEGIN" '[all]' "${want[@]}" "$MARK_END"
    else
      printf '%s\n' "$MARK_BEGIN" '[all]' "${want[@]}" "$MARK_END" >> "$BOOTCFG"
    fi
  else
    msg "  already set up"
  fi
else
  warn "no $BOOTCFG: set up the touch overlay by hand (see docs/design.md, Device setup)"
fi

msg "done. Reboot: the launcher starts on tty1."
msg "Ctrl+Q in the launcher drops to a shell; exit it to get the launcher back."
