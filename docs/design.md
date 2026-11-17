# pockettui35: design

A single-tasking shell for the Waveshare PocketTerm35, built as a TUI.
Status: draft for discussion.

## Why

pt35-desktop runs sway, a daemon (`pt35d`), a bar and an overlay menu. Most of
its code exists to keep sway in line: binding juggling around the sway 1.10
segfault, window placement, workspace policy, reload recovery.

On a 640x480 screen one app at a time is the honest model. Drop the
multitasking and most of that code goes with it.

## Goals

- One app on screen at a time. Quit it and you are back in the launcher.
- A launcher written in Rust with Ratatui: apps, settings, status.
- GUI apps (Chromium, VLC, Mousepad) still work.
- Games (RetroArch) get every button, with no shell bindings in the way.
- Boots on Raspberry Pi OS Lite (Trixie), Pi 4B and Pi 5.
- Fast: the launcher draws its first frame in under 50 ms.

## Non-goals

- Window switching, several apps side by side, a taskbar.
- An overlay menu on top of a running app.
- Support for stock keyboard firmware. See Input.

## Hardware facts

Carried over from pt35-desktop's `docs/hardware-facts.md`. Read that first.

- Display: HDMI-A-1, native 640x480.
- Touch: GT911 on i2c-1 at 0x5d, `waveshare-35dpi-5b.dtbo`.
- Input: RP2040 at USB `1209:0001`. One HID keyboard (QWERTY, D-pad, face
  buttons) plus a 5-button HID mouse. No gamepad device.
- Patched firmware: A B X Y L R send F13-F18. Select and Start send F21 and
  F22.
- Brightness: `B<0-100>` on `/dev/ttyACM0`, answered with `PT35 B=<n>`. Set
  the tty raw in-process, never through `stty`.
- No battery reading. No backlight in sysfs.
- Network: `eth0` and `wlan0` under NetworkManager.

## The shape

Every app has one of three launch kinds.

| Kind | What runs | Examples |
|---|---|---|
| `tty` | in the launcher's terminal, the launcher suspended | htop, yazi, ncdu, darkwire, micro |
| `wayland` | a Wayland client | Chromium, VLC, Mousepad, RetroArch |
| `kms` | owns the screen through DRM, no compositor | mpv `--vo=drm`, RetroArch KMS |

The launcher waits for the app to exit and redraws.

How a launch runs:

- `exec` goes through `sh -c`, so it can use quoting, `~` and `$VARS`.
  `tty` and `kms` run it as is; `wayland` runs `cage -- sh -c <exec>`.
- The launcher leaves raw mode and the alternate screen, and clears the
  console, so the app starts on a clean screen.
- The app runs in its own process group, made the terminal's foreground
  group, as a shell runs a job. Ctrl+C and Ctrl+\\ reach the app, never the
  launcher. When it exits the launcher takes the terminal back.
- An app stopped with Ctrl+Z is resumed at once: there is no job control,
  and a stopped app would hang the launcher.
- The launcher blocks in `waitpid` while the app runs: no CPU use.
- `wayland` output (cage and the client) goes to
  `$XDG_STATE_HOME/pockettui35/launch.log`, overwritten per launch, so it
  stays off the console and a failed launch can be looked into.
- A non-zero exit, a signal, or a missing `cage` shows as a one-line message
  in the hint bar until the next key.
- The config is re-read on return, so items edited from inside an app show
  up at once.

## Where the launcher lives

**Decided: on the Linux console.** tty1 autologs in and runs the launcher
directly. No compositor runs while you sit in the launcher.

```
tty1 autologin
└─ pockettui35                 the launcher, on the console
   ├─ <tty app>                same terminal, launcher suspended
   ├─ cage -- <wayland app>    started on launch, exits with the app
   └─ <kms app>                takes DRM directly
```

A `wayland` app starts as `cage -- <app>`. A `kms` app takes DRM directly.
Both hand the screen back on exit, and the launcher redraws.

What we accept:

- **No touch in the launcher.** The console gives a TUI no mouse events. The
  launcher is text and buttons only. Touch still works inside `wayland` apps,
  through cage.
- **16 colours, a 512-glyph console font, no Nerd Font icons.**
- **Every GUI launch pays cage's start-up cost.** That is the price of
  nothing graphical running while idle, which is better for CPU and battery.

What we gain:

- Idle cost near zero: no compositor, no terminal emulator.
- `kms` apps work.
- Each GUI app gets its own cage. A crash takes down that app's cage, never
  the launcher.
- No dependence on cage's window stacking or focus behaviour.

### Rejected: one long-lived cage, the launcher in foot

The session would be `cage -- foot pockettui35`, with `wayland` apps started
as children in the same cage. It gives truecolor, Nerd Font icons and touch in
the launcher, and faster GUI launches. Rejected because the launcher does not
need touch or icons, a compositor and terminal would run the whole time,
`kms` apps could not run, and it relies on cage returning focus to foot,
which is untested.

## Input

`keyd` does the mapping, below the compositor. It works on the console, in
cage and in a KMS app alike. One config, no runtime binding changes.

Default layer:

| Button | Key | Notes |
|---|---|---|
| D-pad | arrows | as is |
| A | Enter | |
| B | Escape | |
| X | Tab | |
| Y | F10 | the app's menu bar in GTK, Firefox, LibreOffice |
| L, R | PageUp, PageDown | to decide |
| Start | launcher | see Getting out |
| Select | to decide | |

**Games.** An app with `raw_buttons = true` gets the face buttons and
Select unmapped (F13-F18, F21) for its run; RetroArch binds them directly.
Start (F22) keeps its hold-to-close in every mode.

- Before the launch the launcher runs `keyd bind 'main.f13 = f13' ...`, and
  `keyd bind reset` after the app exits.
- keyd's manual counts access to its socket as root, so the session user
  does not get it. The launcher runs `sudo -n pockettui35 keys raw|normal`,
  and the sudoers rule allows those two exact lines. The `keys` subcommand
  calls keyd and exits.
- A marker file (`$XDG_STATE_HOME/pockettui35/keys-raw`) exists while the
  buttons are raw. A launcher that crashed mid-game finds it at start-up and
  resets keyd.
- If keyd cannot be switched the game still starts, with the usual map, and
  the hint bar says why.
- `keyd bind` applies to the most recently active keyboard. The RP2040 is
  the only one, and its A press just launched the game.

To verify: RetroArch can bind F16 and above.

**Stock firmware is not supported.** On stock the face buttons send the
letters `a b x y l r`, the same codes as the QWERTY keys. No remapper can
tell them apart. The installer flashes the patched firmware, as `pt35-kbd`
does today.

### Getting out

cage has no key bindings, so the way out has to live outside the app.

- **Quit inside the app** is the normal path: B, Ctrl+Q, `q`.
- **Start held for one second** runs a keyd `command()`:
  `f22 = timeout(f22, 1000, command(pkill -USR1 -x pockettui35))`. A tap
  stays F22, for games. keyd runs the command as root; it only signals the
  launcher.
  - An app is running: the launcher is blocked in `waitpid`. SIGUSR1 is
    installed without `SA_RESTART`, so the wait returns at once, and the
    launcher sends SIGTERM to the app's process group, then SIGKILL after
    3 s. The hint bar says "<app> closed".
  - A script is running: it is stopped, as with B. A second hold kills it.
  - In the menu it does nothing.
  - Helper threads block SIGUSR1, so the signal always lands on the main
    thread, the one waiting.
  - To verify on the device: keyd's manual words `timeout()` ambiguously
    (a typo at the key sentence). The intent is
    `timeout(<tap>, <ms>, <hold>)`.
- Touch keeps working in `wayland` apps when the keyboard does not. The
  RP2040 can hang and take every button with it. This only helps if the app
  has a close control; the launcher itself has no touch.

## The launcher

One screen: a status bar, a search box, a menu, a hint bar. No tile grid; a
vertical menu uses the 640x480 screen better.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ wlan0 HomeNet ▂▄▆   balanced                           Fri 13 Nov  14:32     │
├──────────────────────────────────────────────────────────────────────────────┤
│ / _                                                                          │
├──────────────────────────────────────────────────────────────────────────────┤
│ Home                                                                         │
│                                                                              │
│   Recent                                                                     │
│ > Chromium                                                                   │
│   Terminal                                                                   │
│   ──────────────────────────────────────────                                 │
│   * Files                                                       (pinned)     │
│   Games                                                                      │
│   Sync notes                                                                 │
│   Settings                                                              >    │
│   Power                                                                 >    │
├──────────────────────────────────────────────────────────────────────────────┤
│ A open   B back   type to search                                             │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Status bar

| Field | Source | Updates |
|---|---|---|
| Connection | `nmcli -t -f TYPE,STATE,CONNECTION device` | each minute, on return from an app, on entering a folder |
| Power profile | cpufreq sysfs, matched to a profile | as Connection, and when changed |
| Date and time | system clock | timer aligned to the minute |

Connection reads `wired` when a cable has link (it carries the traffic
when both are up), else `wifi <name>`, else `offline`; `net ?` when nmcli
fails. A state that matches no profile reads `custom`.

The minute tick already wakes the launcher, so one `nmcli` call there adds
no wakeups. A network change shows within a minute, or at the next key.
D-Bus signals would be immediate but cannot wake the blocking input wait
either, and cost a large dependency.

No battery field. The PocketTerm35 has its own battery light.

### Menu

Everything is an **item** in one tree. Home holds apps, user items, and the
folders.

```
Home
├─ Chromium, Files, Terminal, Games ...   shipped and user items, mixed
├─ Settings >
│  ├─ Wi-Fi (nmtui)                       app, tty
│  ├─ Bluetooth (bluetoothctl)            app, tty
│  ├─ Volume          [■■■■■□□□] 60%      value (wpctl)
│  ├─ Brightness      [■■■■■■□□] 75%      value (serial)
│  ├─ Power profile   < balanced >        choice
│  ├─ Raspberry Pi config (raspi-config)  app, tty
│  └─ About
└─ Power >
   ├─ Screen off
   ├─ Reboot                              confirm
   └─ Shut down                           confirm
```

Item types:

| Type | A does | Example |
|---|---|---|
| `folder` | opens its children | Settings, Power |
| `app` | launches it with its `kind` (`tty`, `wayland`, `kms`) | Chromium, nmtui |
| `script` | runs a command, shows its output | `~/bin/sync-notes.sh` |
| `value` | nothing; Left/Right adjust it in place | Volume, Brightness |
| `choice` | nothing; Left/Right cycle it in place | Power profile |
| `action` | a built-in, optionally confirmed | Reboot, Shut down |

**Navigation.** Up/Down move. A opens or runs. B goes back; at Home it
clears the search. Left/Right only adjust `value` and `choice` items. The
hint bar names what each button does for the selected item. Each folder
remembers its cursor position. Coming back from a folder seen for the first
time selects that folder. The menu title is a breadcrumb: `Home / Settings`.

**Settings rows.** `value` and `choice` items show their state on the
right, also in search results:

```
  Brightness                                   [━━━━━━━━──]  80%
  Power profile                                     < balanced >
  Volume                                       [━━━━──────]  40%
```

- Volume: `wpctl` on `@DEFAULT_AUDIO_SINK@`, steps of 5%, capped at 100%.
  Shows `muted` when muted.
- Brightness: the serial protocol (`src/backlight.rs`, ported from
  pt35-desktop), steps of 10%, the firmware's 5% floor. If the keyboard
  does not answer once (stock firmware), it is not asked again until the
  launcher gets the screen back, so a key press never waits 400 ms. Shows
  `n/a`.
- Values are read after the first frame (the serial query must not delay
  it), on return from an app, and on entering a folder.

**Confirmation.** Reboot and Shut down ask "Press A again to reboot". Any
other key cancels; so do 3 s, after which the hint clears by itself.

**Power actions.** Reboot and Shut down run `systemctl reboot` and
`systemctl poweroff`: logind allows the active local session without a
password. Screen off blanks the console (`TIOCLINUX`, `TIOCL_BLANKSCREEN`),
which on KMS should power the panel down; the next key unblanks it and does
nothing else, so waking never launches or moves anything. About opens the
script screen with the version, board model, kernel, host name, address
and uptime.

**Sorting.** Each folder shows pinned items first (`pin = true`), then
everything else alphabetically. Home also has a **Recent** group on top: the
last 5 apps and scripts launched, from anywhere in the tree. Folders,
settings and power actions never enter it. A recent item that lives in a
folder shows that folder's name on the right. Recent items also stay in
their place in the main list, so the main list never reorders and can be
learned by position. Last-used times live in
`$XDG_STATE_HOME/pockettui35/history.toml` (default
`~/.local/state/...`). A missing or broken file just means an empty Recent.

### Power profile

Three profiles, no others: **performance**, **balanced**, **battery saving**.
Left and Right step through them in that order. No
`power-profiles-daemon`.

| Profile | Governor | Max frequency |
|---|---|---|
| performance | `performance` | hardware max |
| balanced | `schedutil` (`ondemand` if absent) | hardware max |
| battery saving | `schedutil` (`ondemand` if absent) | 60% of max, rounded down to an offered frequency |

Every `policy*` under `/sys/devices/system/cpu/cpufreq` gets the same
settings. The current profile is read back from sysfs, not remembered, so
a change made elsewhere shows as `custom`.

**Root.** Writing cpufreq needs root. The launcher runs itself as
`sudo -n /usr/bin/pockettui35 set-power-profile <name>`; that subcommand
writes sysfs and exits, with no terminal. `assets/sudoers.d/pockettui35`
allows exactly those three command lines, without a password, to the
`pockettui35` group. Without the rule, the row says so in the hint bar.

**Persistence.** cpufreq resets on boot. The chosen profile is saved in
`$XDG_STATE_HOME/pockettui35/power-profile` and applied again at start-up,
after the first frame.

The mapping and the 60% cap are a starting point, to measure on the device.

### Search

- Typing any printable key starts a search. No focus step. A leading space
  is ignored; keys with Ctrl or Alt are never typed.
- Searches the whole tree, flattened: folders, apps, settings, power
  actions, user items. Labels only.
- Fuzzy match (`nucleo-matcher`), case-insensitive. Each space-separated
  word must match. No special syntax: `!`, `^`, `$` and `'` are plain
  characters.
- Ranked by match score; equal scores go to the most recently used, then
  alphabetical.
- Matched letters are highlighted. The search box shows the result count.
- Each result shows its folder on the right (none for Home), so duplicates
  stay apart:
  ```
  > Reboot                Power
    Raspberry Pi config   Settings
    Brightness  75%       Settings
  ```
- A on a result acts as in the menu and ends the search, except on a
  `value` or `choice`, which adjusts in place and keeps it. A `folder`
  result opens it; B from there lands on that folder in its parent.
- B, or Backspace to an empty query, ends the search and returns to the
  folder and row it started from.

### Script output

A `script` item keeps the screen: its output streams into a launcher
screen, framed and titled with the item's label.

- stdout and stderr share one pipe, so they stay in order. stdin is
  `/dev/null`. `TERM=dumb` asks tools for plain output; escape sequences
  that still arrive are stripped (no colour), tabs are expanded, and `\r`
  rewrites the current line, so progress counters end up as one line.
- Long lines are cut at the edge, not wrapped: one output line is one row,
  and scrolling stays exact. The last 10,000 lines are kept.
- New output follows the end. Up/Down and PageUp/PageDown scroll back,
  Home/End jump; the frame shows `v N more` while scrolled up.
- The frame's bottom edge shows the state: `running`, `stopping`, `done`,
  `failed, exit N`, `stopped by signal N`.
- B on a running script sends SIGTERM to its process group; a second B
  sends SIGKILL. B on a finished script closes the screen. A on a finished
  script clears the screen and runs it again.
- The script runs in its own process group, so stopping it reaches its
  children. One thread reads the pipe and another waits for the exit, so
  a background child holding the pipe open cannot hide the exit status.
- While a script runs, the main loop also wakes every 50 ms to take in
  output, and redraws only when something changed. Once it ends, the loop
  goes back to waking only for input and the clock.
- Quitting the launcher sends SIGTERM to a running script.

Scripts that need input use `type = "app"`, `kind = "tty"` instead.

### Drawing

Ratatui on the crossterm backend, on the Linux console.

- **Palette:** the 16 ANSI colours only. No RGB.
- **Glyphs:** ASCII and box-drawing only. Anything else must exist in the
  console font; test it on the device.
- **Event loop:** block in `crossterm::event::read()`. Redraw only on input
  or a state change. The only timer is the status-line clock, once a minute.
  No fixed-rate tick.
- **Writes:** Ratatui diffs its buffers and writes only changed cells. This
  matters because fbcon renders text on the CPU.
- **Cursor:** hidden (`crossterm::cursor::Hide`, and
  `vt.global_cursor_default=0` on the kernel command line), so its blink
  timer does not wake the CPU.
- **Hand-off:** before running a `tty`, `wayland` or `kms` app, leave the
  alternate screen and raw mode. Block in `wait()` while the app runs. On
  exit, re-enter, clear and redraw in full.
- **Screen blanking:** `consoleblank=` on the kernel command line, plus the
  Power screen's "screen off". To verify: the panel actually powers down.

### Config

TOML, two layers, lowest first:

1. Shipped: `/usr/share/pockettui35/config.toml`. If that file is absent, the
   copy built into the binary (`assets/config.toml`) is used.
2. User: `$XDG_CONFIG_HOME/pockettui35/config.toml`, default
   `~/.config/pockettui35/config.toml`.

`assets/config.toml` documents every field.

Shipped items and user items use the same schema. An item goes in Home
unless it names a `parent`.

```toml
[item.browser]
label = "Browser"
type  = "app"
kind  = "wayland"
exec  = "chromium --ozone-platform=wayland --force-device-scale-factor=0.75"

[item.files]
label = "Files"
type  = "app"
kind  = "tty"
exec  = "yazi"
pin   = true

[item.retroarch]
label       = "Games"
type        = "app"
kind        = "wayland"
exec        = "retroarch"
raw_buttons = true

[item.sync]
label = "Sync notes"
type  = "script"
exec  = "~/bin/sync-notes.sh"

[item.nmtui]
label  = "Wi-Fi"
type   = "app"
kind   = "tty"
exec   = "nmtui"
parent = "settings"
```

- **Custom items.** Install an app, add an `[item.*]` for it in
  `~/.config/pockettui35/`, and it shows up among the apps on Home.
- **Overrides.** A user item with a shipped item's id merges into it field
  by field, so `[item.files] exec = "ranger"` changes only the command. If
  the user entry changes `type`, it replaces the item instead.
- **Hiding.** `hidden = true` removes an item. On a folder it removes
  everything under it.
- **Built-ins.** `value`, `choice` and `action` items name a `builtin`:
  `volume`, `brightness`, `power_profile`, `screen_off`, `reboot`,
  `shutdown`, `about`. `confirm = true` asks for A twice.
- **Errors never stop the launcher.** A file that fails to parse is skipped.
  An invalid item (missing field, unknown field, wrong `builtin`) is
  skipped. An item whose `parent` is missing or not a folder shows in Home.
  Each problem becomes a warning, counted in the hint bar.
- **Reload.** The config is re-read each time the launcher regains the
  screen. No restart.
- **No `icon` field.** The console font has no Nerd Font glyphs.
- **v1: TOML editing only.** No in-launcher form for adding items.

Scaling is per app, as today: `--force-device-scale-factor` for Chromium,
`QT_SCALE_FACTOR` for Qt, `GDK_DPI_SCALE` for GTK3.

## Boot

1. Pi OS Lite with the touch overlay and USB host in `config.txt` (see
   Device setup below).
2. getty logs the session user in on tty1 with no prompt
   (`getty@tty1.service.d/pockettui35-autologin.conf`). The default target is
   `multi-user.target`; no display manager. The logind session this creates
   is what cage uses for DRM and input: no seatd, no greetd.
3. The login shell reads `/etc/profile.d/pockettui35.sh`, which on tty1, for
   that user only, runs `exec pockettui35-session`.
4. `pockettui35-session` runs the launcher:
   - A crash restarts it after a second. Five crashes in a row, each within
     30 s of starting, give up to a shell, so a broken install cannot loop
     for ever.
   - A clean exit (Ctrl+Q) drops to a login shell with `PT35_NO_LAUNCHER=1`,
     for maintenance. Leaving that shell ends the session; getty logs in
     again and the launcher is back.
5. At start-up the launcher draws its first frame, then puts back the saved
   power profile and resets keyd if a crash left the buttons raw.

### Device setup

The installer does this (see Installer below). By hand, verified on the
device 2026-11-16:

The touchscreen needs I²C and Waveshare's overlay. Without them no touch
device exists, and touch in `wayland` apps silently does nothing.

**Use `waveshare-35dpi-5b.dtbo` on the Pi 4B too.** Waveshare's overlays
differ by the GT911's I²C address, not by board. `4b` probes only `0x14`;
PocketTerm35 units answer at `0x5d`, which only `5b` declares. With `4b`
the driver probes an empty address and touch fails. The file is in
`assets/boot/`, with its checksum, copied unmodified from pocketterm35-os
(which vendors Waveshare's `3.5HDMI_E_DTBO.zip`).

```sh
sudo install -m644 waveshare-35dpi-5b.dtbo /boot/firmware/overlays/
sudo cp /boot/firmware/config.txt /boot/firmware/config.txt.bak
sudo sed -i 's/^#dtparam=i2c_arm=on/dtparam=i2c_arm=on/' /boot/firmware/config.txt
printf '[all]\ndtoverlay=waveshare-35dpi-5b\n' | sudo tee -a /boot/firmware/config.txt
sudo reboot
```

`config.txt` should then hold, uncommented and in effect for every board:

```
dtparam=i2c_arm=on
dtoverlay=waveshare-35dpi-5b
dtoverlay=dwc2,dr_mode=host    # USB host for the RP2040 keyboard
```

The `[all]` matters: appended at the end of the file without it, the line
can land inside a board filter such as `[cm5]` and be ignored. Check after
the reboot:

```sh
grep -i -A4 goodix /proc/bus/input/devices   # a Goodix touchscreen
dmesg | grep -i -E 'goodix|gt911'            # no I/O errors
```

cage then passes touch to the app with no further setup: `cage -- chromium`
takes taps and scrolls.

**Installer.** `sudo ./install.sh [--dry-run] [--user NAME] [--no-build]`,
and `--uninstall`.

- **Packages:** `cage keyd pipewire pipewire-alsa wireplumber`.
- **Build:** builds as the user, with rustup's cargo.
- **Installs:** the binary, the session wrapper, the shipped config, the
  keyd map, the sudoers rule (checked with `visudo` first), the getty
  drop-in and the profile hook.
- **Groups:** adds the user to `video` (sudoers rule, DRM), `dialout` (the
  keyboard's serial console), `input`, `render` and `audio`.
- **Conflicts:** moves an older keyd config for the same keyboard aside
  (keyd allows one config per device). Disables greetd, lightdm, gdm3 or
  sddm if enabled.
- **Kernel command line:** adds `vt.global_cursor_default=0` to
  `/boot/firmware/cmdline.txt`, with a backup.
- **Touchscreen:** checks and installs `waveshare-35dpi-5b.dtbo` into
  `/boot/firmware/overlays/`. Adds whichever of `dtparam=i2c_arm=on`, the
  overlay and `dtoverlay=dwc2,dr_mode=host` are not already set, in one
  `[all]` block between `# pockettui35 >>>` and `<<<` markers, with a
  backup. Comments out a `3b` or `4b` Waveshare overlay. Running it again
  changes nothing. `--uninstall` leaves this in place: it is the device's
  setup, not the launcher's.
- **Safety:** refuses to run on anything but a Raspberry Pi without
  `--force`, since it takes over tty1.

## What comes over from pt35-desktop

- `firmware/` and `pt35-kbd`: the patched keyboard firmware and its flasher.
- `backlight.rs`: the serial brightness protocol, non-blocking and bounded.
- `scripts/pt35-ai`: the local AI agent, unchanged. DarkWire is a `tty` app.
- The `keyd` config, extended with the layers above.
- The `5b` overlay and its checksum, in `assets/boot/`.

Left behind: sway, `pt35d`'s modes, placement and events, `pt35-bar`,
`pt35-menu`, the layer-shell UI crate.

## Open questions

1. Can RetroArch bind F16-F22? If not, `raw_buttons` maps them onto keys it
   can bind.
2. What do L, R and Select do in the default layer?
3. A D-pad mouse for apps that need a cursor: decided against for now,
   touch only. Touch works under cage once the overlay is in. If it comes
   back: cage has no bindings or IPC and keyd cannot emit pointer events,
   so it would be a uinput mouse in the launcher, fed by a keyd layer that
   R toggles.
4. File dialogs: GTK's built-in chooser without a portal. Good enough at
   640x480?
5. Console font: which one, at what size? Terminus 8x16 gives 80x30 at
   640x480.
6. How long does cage take to start on a Pi 4B? It sets the floor for every
   GUI launch.
7. Screen off: does `TIOCL_BLANKSCREEN` power the HDMI panel down, or only
   draw black? If only black, add a DPMS call or turn the backlight to its
   5% floor too.
8. Kernel idle blanking (`consoleblank=`): the key that wakes it also
   reaches the launcher and acts. Swallow it too?
9. Power profile mapping: confirm the governors and caps above on Pi 4B and
   Pi 5, and measure the difference.
