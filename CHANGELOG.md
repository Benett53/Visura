# Changelog

## 0.3.0

Fixed:

- Linux, Wayland: the selection overlay could swallow every click and look like
  a frozen desktop. It waited for the window to cover the screen exactly, which
  never happened with display scaling or when the desktop kept it off a panel.
  It now works with whatever part of the screen it covers, goes full screen on
  a single monitor, and scales between the screenshot's pixels and XWayland's
  coordinates.
- Linux, Wayland: "Capture window" found no window. It now steps aside and asks
  the desktop: `spectacle -a` on KDE, `gnome-screenshot -w` on GNOME, the
  compositor on Sway and Hyprland.
- Linux: a screenshot tool that hangs is stopped after ten seconds instead of
  blocking Visura, and the desktop's own tool is tried first.
- Linux: Visura's own window is no longer offered as a capture target.

Added:

- Icons for every editor tool.
- More settings per tool: opacity, dashed lines, corner radius, arrow head size
  and heads at both ends, drop shadow, monospace text, the colour of black-out
  boxes, and a fixed aspect ratio for cropping.
- Each tool keeps its own settings, and the editor remembers them and the last
  tool between runs (`[editor]` in the configuration).
- The viewer and the editor open in the middle of the primary monitor.
- A note in the settings on Wayland about binding `visura --shot` instead of
  global shortcuts.

## 0.2.0

Added:

- Image viewer. Double-click a shot to open it; zoom with the mouse wheel,
  pan by dragging, arrow keys for the previous and next shot.
- Image editor, opened with `Edit` on a selected shot or from the right-click
  menu: rectangle, ellipse, arrow, line, pen, highlighter, text, step numbers,
  spotlight, blur, pixelate, black box and crop. Colour, width, fill and text
  size are adjustable, every object can be selected, moved, resized and
  restyled afterwards, with undo and redo. The result can be copied, saved
  over the file or saved as a copy. Every tool has a key; `F1` lists them.

Changed:

- Double-click opens the built-in viewer. The default program is still
  available from the right-click menu.

Fixed:

- Long notices wrapped into a narrow column.

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
