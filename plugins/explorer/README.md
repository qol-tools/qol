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

Explorer remembers the row or icon size, whether hidden entries show and how the folder is sorted, and opens with them next time on every machine your profile syncs to. All of them are also on Explorer's settings page; a change there applies the next time Explorer opens. Drag any edge or corner to resize the window, down to 640 by 400, and it reopens where you left it at the size you left it. If Explorer is open when the tray restarts, it opens again afterwards. A wider window gives the sidebar and the Name column more room, up to a limit, so Kind and Size stay close to the names.

Picks stay picked when you move to another folder, and the note beside the crumbs says how many sit in other folders. Enter opens a file in its default app, or says in red that no app could open it.

## Pinned folders

Pinned folders lead the sidebar. A new install pins Home, Desktop, Documents, Downloads, Music, Pictures and Videos, and any of them can be unpinned, renamed and reordered.

Ctrl+D pins the folder you are in, or unpins it. Drag a folder between two pins, or above or below them, to pin it there: the folder turns into a floating pin row, the pins below make room and a tag under the pointer says where it lands, such as "pin 5th". Dropping a folder onto a pin itself still moves it into that folder, and the tag says "move into Documents". Right-click a pin, or press the ⋯ on the pin you are in, to unpin, move or rename it, or to open the sidebar settings. The ⋯ at the end of the title band pins the folder you are in, edits the path, shows or hides hidden files and opens Explorer's settings.

A usual folder such as Desktop or Music that is missing is hidden until it exists. Any other pinned folder that is not there, such as one on a drive that is unplugged, shows "not connected" until it comes back.

In Explorer's settings, Sidebar > Pinned folders reorders the pins from the keyboard. Enter or a click picks a pin up, Up and Down carry it, Enter puts it down and Esc puts it back. Ctrl+Up and Ctrl+Down carry it while Ctrl is held and put it down when Ctrl is released.

## Keys

| Key | Does |
| --- | --- |
| Up, Down | Move the cursor one row |
| Page Up, Page Down | Move the cursor one page |
| Home, End | Move the cursor to the first or last row |
| Shift with any of those | Pick the rows from where the range started to the cursor, so moving back shrinks it |
| Click | Move the cursor to the row; with Shift pick the rows from the cursor to it, with Ctrl add it to the picks; a double click opens it |
| Enter | Open the folder or file under the cursor |
| Shift+Enter | Choose the app to open the file under the cursor with, the default first; with picks of the same type it opens them all. Linux only for now |
| Backspace, Alt+Up | Go to the parent folder, cursor on the folder you left |
| Alt+Left, Alt+Right | Back and forward |
| Space | Pick or unpick the row under the cursor, or add a space to the letters typed when a name goes on with one |
| Ctrl+A | Pick every row |
| Esc | Drop the picks, a cut and the red note; while dragging, cancel the drag; while drawing a selection box, put the picks back as they were |
| Ctrl+C, Ctrl+X | Copy or cut the picks, or the row under the cursor when nothing is picked; cut rows fade until they are pasted |
| Ctrl+V | Paste into the folder you are in |
| Delete | Move the picks, or the row under the cursor, to the trash |
| Shift+Delete | Delete the picks, or the row under the cursor, for good after asking, since the trash cannot bring them back |
| Ctrl+Alt+N | Make an empty file named New file, then rename it |
| Ctrl+Shift+D | Copy the picks, or the row under the cursor, beside themselves as `name copy.ext`, then `name copy 2.ext` |
| F2 | Rename the row under the cursor, with the name before its extension selected |
| Ctrl+Shift+N | Make a new folder and rename it |
| Ctrl+Shift+T | Open your terminal in the folder you are in |
| Ctrl+Shift+C | Copy the full path of each pick, or of the row under the cursor, one per line |
| Ctrl+Z | Undo the last copy, duplicate, move, rename, trash, new file or new folder, putting back anything a copy replaced |
| Ctrl+J | Show or hide today's moves |
| Menu key, Shift+F10 | Open the row menu at the cursor row; with other rows picked it acts on them all, otherwise it picks the cursor row first |
| Alt+Enter | Show or hide properties for the row under the cursor, or a count and total size for the picks: kind, path, size (a folder's total is counted in the background), dates, permissions and owner on Linux and macOS or attributes on Windows, the link target, and a preview of png, jpg, gif, webp and svg images; Escape also closes it |
| Ctrl+H | Show or hide hidden entries, also while editing the path; hiding them drops their picks |
| Ctrl+Alt+1 to Ctrl+Alt+4 | Sort by name, size, date modified or kind; pressing the one you are already sorted by flips ascending and descending. Folders stay first, and the folder's menu marks the active sort |
| Ctrl+F | Open a filter field beside the crumbs that narrows the list to names containing what you type, ignoring case and accents; Up, Down, Page Up, Page Down and Enter still work the list, Esc clears the filter and closes it, and the note says how many entries it hides. Moving to another folder closes it, and picks the filter hides are dropped |
| Ctrl+D | Pin the folder you are in, or unpin it |
| Ctrl+, | Open Explorer's settings |
| Ctrl+G | Edit the path, selected so typing replaces it; Up and Down cycle through the suggested folders, Tab completes one, Enter or a click goes there |
| Ctrl+C, Ctrl+X, Ctrl+V | In the path, filter and rename fields, copy, cut and paste text through the system clipboard |
| Ctrl+Z, Ctrl+Shift+Z, Ctrl+Y | In those fields, undo a typed word, paste or cut, and redo it |
| Ctrl+= or Ctrl++, Ctrl+- | Make the rows or icons one size bigger or smaller; the slider in the title bar does the same |
| Ctrl+1 to Ctrl+5 | Pick a size: compact, list, roomy, icons, big icons |
| Ctrl+wheel | Step the size up or down |
| Left, Right | In the icon sizes, move the cursor one icon; Up and Down move a whole row |
| Letters | Jump to the first name that starts with what you typed; a plain letter also matches its accented forms, an accented one only itself, and a letter that matches nothing is left out |

Right-click a row or icon to pick it and open a menu with Open, Cut, Copy, Paste, Rename, Move to trash, New folder and Undo, each showing its key. Right-clicking a row that is already picked keeps the picks, so the menu acts on all of them; Rename needs a single row and Paste needs something cut or copied. Right-click on empty space in the list for Paste, New folder and Undo above the folder lines (pin, go to a path, hidden files, settings). Up and Down move through a menu, Enter runs the line and any other key closes it.

Press anywhere in the list and move at once to draw a selection box: every row or icon it touches is picked while you drag, Ctrl or Shift keeps the picks you had, Esc puts them back, and the list scrolls when you pull past its top or bottom edge. A click on empty space drops the picks. Press a row and hold still for half a second and a circle grows beside the pointer, then the files lift and you can drag them. Drag rows onto a folder, a place in the sidebar or a folder in the path to move them there; hold Ctrl to copy instead. Rows dragged to another drive, and files dropped in from another app, are copied. The target says "move here" or "copy here", or "no room" or "copy only" when the drive is too full or the files cannot be removed from where they are.

When a paste or a drop meets names that are already there, a sheet asks first and nothing moves until you apply it. Left and Right choose Keep this, Replace or Keep both for a row, with Shift for every row; Up and Down move between rows; Enter applies and Esc cancels. A replaced file goes to the trash.

## License

PolyForm Noncommercial 1.0.0
