# Changelog

## 0.1.3

Changed:

- A click in the overlay takes the whole window again; holding `Ctrl` takes
  the pane under the cursor. In programs covered by panes edge to edge, such
  as Explorer, the whole window was out of reach. `panes_first` restores the
  0.1.2 behaviour; `detect_areas` is no longer read.

Added:

- `--shot region|window|screen` on the command line. A running Visura takes
  the shot; otherwise Visura starts in the background and takes it.
- Shots older than a set number of days can be moved to the recycle bin
  automatically (`keep_days`, off by default).
- Linux: dragging shots out of the list into other programs over XDND.
- Linux: window outlines on Wayland under Hyprland and Sway.

Fixed:

- `--help` printed German text.

## 0.1.2

Fixed:

- Dragging from the very edge of the screen grabbed the overlay's invisible
  frame: the overlay moved or shrank into a picture of the desktop and the
  program stopped responding. The overlay now reports its whole area as
  content to Windows.
- A window left in the background was brought to the front after every shot.
  It now keeps its place in the window order, and the previously active
  window gets the keyboard back.
- A click in the overlay was lost when the mouse had not moved since the
  overlay opened. Buttons are now read from the system.

Added:

- Panes inside a window are outlined separately, such as the page in Chromium
  based browsers and Electron apps. `Ctrl` + click takes the whole window.
  Programs that draw everything into one surface, such as Firefox, still get
  the whole window. Setting: `detect_areas`.
- Setting to move the shots of the current session to the recycle bin when
  Visura quits (`delete_on_exit`, off by default).

## 0.1.1

Fixed:

- Shortcuts did nothing while the window was minimised or behind a full screen
  program. They were read once per drawn frame, and an idle window draws none;
  the press only arrived when something else happened to wake the window. The
  system now wakes the program itself, so a shortcut works whatever the window
  is doing. Taking a shot from a minimised window also leaves it minimised.
- The window came back as a stub in the corner after a capture that started
  from the task bar.
- The shortcut printed next to each sidebar entry overlapped the label and was
  cut off for anything longer than a single key. Removed; the settings screen
  shows shortcuts.

Changed:

- The outline around a detected window is dashed and moving, and slides to the
  next window rather than jumping.
- Anonymous file names keep the dated folders, so a shot is still easy to find.
  Only the name changes, and neither encoder writes a timestamp or EXIF.
- Theme and accent take effect when saved instead of on the next start.

## 0.1.0

First release.

- Region, window and full screen capture; only region has a shortcut out of
  the box (`Print`), the other two start unbound
- Selection overlay on a frozen frame: dimming, crosshair, window outlining,
  optional magnifier with a colour readout
- File names from the program and the date, or random characters when names
  should say nothing
- Recent shots in the main window, the full library in a window of its own
  with search and day headings
- Drag a shot out of the list; delete moves to the recycle bin
- Shortcuts are set by pressing the key combination, and can be left unset
- Dark, black and light themes
- Windows installer, per user, with an update path that keeps the settings
