# pockettui35

A single-tasking launcher for the Waveshare PocketTerm35, written in Rust
with [Ratatui](https://ratatui.rs). It runs on the Linux console of tty1:
pick an app, it takes the whole screen, quit it and you are back in the
menu. No compositor runs while you sit in the launcher.

> **Status:** feature-complete for a first version, but not yet run on the
> device. See [What still needs checking](#what-still-needs-checking).

```
 wifi Home   balanced                                   Sat 14 Nov  16:31
┌──────────────────────────────────────────────────────────────────────────┐
│/ type to search                                                          │
└──────────────────────────────────────────────────────────────────────────┘
│ Home                                                                     │
│  Recent                                                                  │
│> Chromium                                                                │
│  Wi-Fi                                                          Settings │
│  ────────────────────────────────────────────────────────────────────────│
│  Chromium                                                                │
│  Files                                                                   │
│  Games                                                                   │
│  Power                                                                 > │
│  Settings                                                              > │
│  Terminal                                                                │
└──────────────────────────────────────────────────────────────────────────┘
 A  launch   type to search
```

## Features

- **One menu tree.** Apps, scripts, settings and power actions, in folders.
  Pinned items first, then alphabetical. Home has a Recent group of the last
  five apps and scripts used, from any folder.
- **Search everything.** Type anywhere to fuzzy-search every item; each
  result shows its folder.
- **Three ways to run an app:**
  - `tty`: in the launcher's terminal (htop, yazi, nmtui).
  - `wayland`: under its own [cage](https://github.com/cage-kiosk/cage)
    (Chromium, VLC, RetroArch).
  - `kms`: straight on DRM (mpv `--vo=drm`).
- **Scripts with live output.** A `script` item streams its output into a
  launcher screen and shows how it ended.
- **Settings in place.** Volume, brightness and power profile change with
  Left/Right, right in the menu.
- **Status bar.** Connection, power profile, date and time.
- **Power.** Reboot and Shut down (press A twice), Screen off, About.
- **Always a way out.** Hold Start for a second to close any app.
- **Light on battery.** The launcher sleeps until a key or the minute tick,
  and redraws only what changed.

## Requirements

- Raspberry Pi 4B or 5 in a PocketTerm35, on Raspberry Pi OS Lite (Trixie).
- The **patched keyboard firmware**, which puts the face buttons on F13-F18,
  Select on F21 and Start on F22. Stock firmware sends the same keys for
  the A button and the `a` key, so no remapper can tell them apart. Flash
  it with `pt35-kbd` from pocketterm35-os.
- A Rust toolchain for the user (rustup), to build on the Pi.

## Install

From a checkout on the Pi:

```sh
sudo ./install.sh --dry-run --user "$USER"   # see what it would change
sudo ./install.sh --user "$USER"
sudo reboot
```

The installer:

- **Packages:** installs `cage keyd pipewire pipewire-alsa wireplumber`.
- **Files:** builds the launcher as you and installs it with its session
  wrapper.
- **Session:** sets up autologin on tty1, so the launcher starts at boot,
  and turns off any display manager (greetd, lightdm, gdm3, sddm).
- **Buttons:** installs the keyd map, moving an older keyd config for the
  same keyboard aside.
- **sudoers:** installs a rule allowing five exact command lines: the power
  profile and the game-button switch. It is checked with `visudo` first.
- **Groups:** adds you to `video`, `dialout`, `input`, `render` and `audio`.
- **Kernel:** hides the console cursor on the kernel command line, with a
  backup.

It refuses to run on anything but a Raspberry Pi unless you pass `--force`.
Remove everything with `sudo ./install.sh --uninstall`.

## Buttons

keyd maps the buttons to keys, so the same map works everywhere: the
console, cage and KMS apps.

| Button | Key | In the launcher |
|---|---|---|
| D-pad | arrows | move; Left/Right adjust a setting |
| A | Enter | open, launch, run; A again to confirm |
| B | Escape | back; clear the search; stop a script |
| X | Tab | |
| Y | F10 | (menu bar in GTK, Firefox, LibreOffice) |
| L, R | PageUp, PageDown | scroll script output |
| Start, held 1 s | | close the running app or stop the script |

The keyboard works as usual: typing searches, Backspace edits the search,
Home/End jump. **Ctrl+Q** quits the launcher to a maintenance shell; exit
the shell and the launcher comes back.

Apps with `raw_buttons = true`, such as RetroArch, get the face buttons and
Select as plain F13-F18 and F21 while they run. Holding Start still closes
them.

## Configuration

Items come from two TOML files, merged field by field:

1. `/usr/share/pockettui35/config.toml`, shipped
   ([`assets/config.toml`](assets/config.toml), which documents every
   field).
2. `~/.config/pockettui35/config.toml`, yours.

```toml
# A new app on Home.
[item.htop]
label = "htop"
type  = "app"
kind  = "tty"
exec  = "htop"

# A script, its output shown in the launcher.
[item.sync]
label = "Sync notes"
type  = "script"
exec  = "~/bin/sync-notes.sh"
pin   = true

# Change one field of a shipped item.
[item.files]
exec = "ranger"

# Remove a shipped item (on a folder: everything in it).
[item.games]
hidden = true
```

| Field | Meaning |
|---|---|
| `label` | text in the menu |
| `type` | `folder`, `app`, `script`, `value`, `choice` or `action` |
| `parent` | id of a folder; default Home |
| `pin` | sort first in its folder |
| `hidden` | remove the item |
| `kind` | apps: `tty`, `wayland` or `kms` |
| `exec` | apps and scripts: a command line, run through `sh -c` |
| `raw_buttons` | apps: unmapped face buttons (games) |
| `builtin` | `volume`, `brightness`, `power_profile`, `screen_off`, `reboot`, `shutdown`, `about` |
| `confirm` | actions: press A twice |

A broken item never stops the launcher. It is skipped, and the hint bar
counts the warnings. The config is read again each time an app exits, so
edits made from inside an app show up at once.

State lives in `~/.local/state/pockettui35/`:

- `history.toml`: the Recent group.
- `power-profile`: the profile to put back at boot.
- `launch.log`: output of the last `wayland` launch.

## Development

```sh
cargo run      # in any terminal; Ctrl+Q quits
cargo test
cargo clippy --all-targets
```

On a desktop, the system parts degrade quietly. Brightness shows `n/a`
without the keyboard. The power profile reads `custom` on a CPU driver
without the Pi's governors. Screen off does nothing outside the Linux
console.

The design, and why each choice was made, is in
[`docs/design.md`](docs/design.md).

## What still needs checking

These need the device, and are tracked as open questions in the design doc:

- Holding Start closes an app. keyd's manual words `timeout()`
  ambiguously; check with `sudo keyd monitor`.
- `keyd bind` works as used here in keyd 2.5.0, the version in Trixie.
- Screen off powers the panel down, rather than only drawing black.
- The power profiles' governors and the 60% battery-saving cap suit the
  Pi 4B and Pi 5.
- How long cage takes to start, and whether RetroArch can bind F16 and
  above.
