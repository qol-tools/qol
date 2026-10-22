<div align="center">

# Explorer

A keyboard-first file explorer, for [QoL Tray](../../apps/tray).

</div>

## Quick start

Install from the [QoL Tray](../../apps/tray) plugin store.

## About

`qol-explorer open` opens the window on the home folder, and `qol-explorer open <path>` opens it on that folder. Each window is titled by the folder it shows, as `<folder> - Explorer`.

The list follows the folder on disk, so files that appear, change or vanish show up while the window is open, and a folder that is renamed or removed, or sits under one that is, says it no longer exists. Folders on proc, sys and the other kernel filesystems send no change events, so they are read again every two seconds. Folder rows count their items, leaving out hidden ones until Ctrl+H shows them, and the counts of the rows around the screen follow changes inside those folders. A link shows the size of what it points to and follows changes to it.

The Kind column reads Folder, File, Link, Pipe, Socket, Device or Broken link, the last for a link whose target is gone or loops. Pipes, sockets, devices, broken links and files on the kernel filesystems show no size, since the size they report is not their content.

Explorer remembers the row or icon size and whether hidden entries show, and opens with them next time on every machine your profile syncs to. Both are also on Explorer's settings page. The window reopens at the size and place you left it.

Picks stay picked when you move to another folder, and the note beside the crumbs says how many sit in other folders. Enter opens a file in its default app, or says in red that no app could open it.

## Keys

| Key | Does |
| --- | --- |
| Up, Down | Move the cursor one row |
| Page Up, Page Down | Move the cursor one page |
| Home, End | Move the cursor to the first or last row |
| Shift with any of those | Pick the rows from where the range started to the cursor, so moving back shrinks it |
| Click | Move the cursor to the row; with Shift pick the rows from the cursor to it, with Ctrl add it to the picks; a double click opens it |
| Enter | Open the folder or file under the cursor |
| Backspace, Alt+Up | Go to the parent folder, cursor on the folder you left |
| Alt+Left, Alt+Right | Back and forward |
| Space | Pick or unpick the row under the cursor, or add a space to the letters typed when a name goes on with one |
| Ctrl+A | Pick every row |
| Esc | Drop the picks and the red note |
| Ctrl+H | Show or hide hidden entries, also while editing the path; hiding them drops their picks |
| Ctrl+G | Edit the path, selected so typing replaces it; Up and Down cycle through the suggested folders, Tab completes one, Enter or a click goes there |
| Ctrl+= or Ctrl++, Ctrl+- | Make the rows or icons one size bigger or smaller; the slider in the title bar does the same |
| Ctrl+1 to Ctrl+5 | Pick a size: compact, list, roomy, icons, big icons |
| Ctrl+wheel | Step the size up or down |
| Left, Right | In the icon sizes, move the cursor one icon; Up and Down move a whole row |
| Letters | Jump to the first name that starts with what you typed; a plain letter also matches its accented forms, an accented one only itself, and a letter that matches nothing is left out |

## License

PolyForm Noncommercial 1.0.0
