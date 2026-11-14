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

**Games.** An app profile with `raw_buttons = true` asks for the F13-F22 keys
unmapped. RetroArch binds them directly. keyd layers can change at run time
through `keyd bind`, so the launcher switches the layer around the launch and
switches it back on exit. To verify: RetroArch can bind F16 and above.

**Stock firmware is not supported.** On stock the face buttons send the
letters `a b x y l r`, the same codes as the QWERTY keys. No remapper can
tell them apart. The installer flashes the patched firmware, as `pt35-kbd`
does today.

### Getting out

cage has no key bindings, so the way out has to live outside the app.

- **Quit inside the app** is the normal path: B, Ctrl+Q, `q`.
- **Start held for one second** runs a keyd `command()` that asks the
  launcher to close the running app (SIGTERM, then SIGKILL after 3 s).
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
| Connection | NetworkManager over D-Bus (`zbus`) | on D-Bus signals, no polling |
| Power profile | launcher state / `scaling_governor` | only when changed |
| Date and time | system clock | timer aligned to the minute |

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

**Confirmation.** Reboot and Shut down ask "Press A again", and cancel after
3 s.

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
The launcher writes sysfs directly (through a small root helper or a polkit
rule). No `power-profiles-daemon`.

| Profile | Governor | Max frequency |
|---|---|---|
| performance | `performance` | hardware max |
| balanced | `schedutil` | hardware max |
| battery saving | `schedutil` | capped (value to measure) |

The mapping is a starting point, to verify on the device. The chosen profile
persists across reboots.

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

1. Pi OS Lite, `waveshare-35dpi-5b` overlay in `config.txt`.
2. greetd (or getty autologin) starts the session on tty1.
3. tty1 runs `pockettui35` directly, restarted if it exits.

## What comes over from pt35-desktop

- `firmware/` and `pt35-kbd`: the patched keyboard firmware and its flasher.
- `backlight.rs`: the serial brightness protocol, non-blocking and bounded.
- `scripts/pt35-ai`: the local AI agent, unchanged. DarkWire is a `tty` app.
- The `keyd` config, extended with the layers above.
- `boot/` overlay files and their checksums.

Left behind: sway, `pt35d`'s modes, placement and events, `pt35-bar`,
`pt35-menu`, the layer-shell UI crate.

## Open questions

1. Can RetroArch bind F16-F22? If not, `raw_buttons` maps them onto keys it
   can bind.
2. What do L, R and Select do in the default layer?
3. A D-pad mouse for apps that need a cursor: a uinput mouse in the launcher,
   or touch only?
4. File dialogs: GTK's built-in chooser without a portal. Good enough at
   640x480?
5. Console font: which one, at what size? Terminus 8x16 gives 80x30 at
   640x480.
6. How long does cage take to start on a Pi 4B? It sets the floor for every
   GUI launch.
7. Power profile mapping: confirm the governors and caps above on Pi 4B and
   Pi 5, and measure the difference.
