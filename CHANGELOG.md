# Changelog

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
