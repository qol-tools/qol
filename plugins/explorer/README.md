<div align="center">

# Explorer

A keyboard-first file explorer, for [QoL Tray](../../apps/tray).

</div>

## Quick start

Install from the [QoL Tray](../../apps/tray) plugin store.

## About

`qol-explorer open` opens the window on the home folder, and `qol-explorer open <path>` opens it on that folder. The list follows the folder on disk, so files that appear, change or vanish show up while the window is open. Folder rows count their items, leaving out hidden ones until Ctrl+H shows them, and the counts follow changes inside those folders. Enter opens a file in its default app, or says in red that no app could open it.

## Keys

| Key | Does |
| --- | --- |
| Up, Down | Move the cursor one row |
| Page Up, Page Down | Move the cursor one page |
| Home, End | Move the cursor to the first or last row |
| Shift with any of those | Pick the rows the cursor passes |
| Enter | Open the folder or file under the cursor |
| Backspace, Alt+Up | Go to the parent folder, cursor on the folder you left |
| Alt+Left, Alt+Right | Back and forward |
| Space | Pick or unpick the row under the cursor |
| Ctrl+A | Pick every row |
| Esc | Drop the picks and the red note |
| Ctrl+H | Show or hide hidden entries |
| Ctrl+G | Edit the path; Up and Down highlight a suggested folder, Tab completes it, Enter or a click goes there |
| Letters | Jump to the first name that starts with what you typed |

## License

PolyForm Noncommercial 1.0.0
