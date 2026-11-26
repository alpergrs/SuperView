<p align="center">
  <img src="crates/sv-app/assets/icon-256.png" width="128" alt="SuperView icon">
</p>

# SuperView

A fast picture viewer for Windows, in the spirit of ACDSee and IrfanView. It opens at once,
puts the picture first, and does everything from the keyboard. It is one `.exe` file with
nothing to install.

- **Viewer** — zoom, pan, fullscreen, slideshow, rotate, animations, multi-page TIFF.
- **Thumbnail browser** — folder tree, thumbnails, preview, sorting, filtering, search.
- **Many formats** — JPEG, PNG, GIF, BMP, WebP, TIFF, HEIC, AVIF, camera RAW and PSD.
- **Editing** — crop, resize, straighten, light and colour, sharpen, blur, noise reduction,
  with undo. RAW files can be developed from the sensor data.
- **File work** — rename, delete to the Recycle Bin, copy and move, an Explorer-compatible
  clipboard, batch convert / resize / rename, metadata editing, duplicate finder.

## Contents

- [Download and run](#download-and-run)
- [The viewer](#the-viewer)
- [The thumbnail view](#the-thumbnail-view)
- [Selecting part of a picture](#selecting-part-of-a-picture)
- [Editing and saving](#editing-and-saving)
- [Batch tools](#batch-tools)
- [Search and duplicates](#search-and-duplicates)
- [Formats](#formats)
- [Settings](#settings)
- [Command line](#command-line)
- [Where SuperView keeps its files](#where-superview-keeps-its-files)
- [Building from source](#building-from-source)
- [License](#license)

## Download and run

Download `superview.exe` from the [Releases](../../releases/latest) page, put it anywhere
and run it. Windows 10 and 11, 64-bit.

The file is not signed, so the first time Windows may show "Windows protected your PC".
Choose **More info → Run anyway**.

To make SuperView the program that opens your pictures, open **Settings (Ctrl+,) → Windows**
and choose to add it to Explorer. That adds:

- "Open with" for all supported picture types,
- "Open with SuperView" when you right-click a picture,
- "Slideshow with SuperView" when you right-click a folder.

Started on its own, SuperView shows the thumbnails of your Pictures folder (you can choose
another start folder in Settings). You can also drop a picture or a folder on the window.
Opening another picture while a window is open shows it in that window.

## The viewer

Opening a picture shows it alone, with the rest of its folder one key press away.
Right-click anywhere for the menu; **F1** lists all keys.

| Action | Keys and mouse |
|---|---|
| Next / previous picture | Right, PageDown / Left, Backspace, PageUp |
| A random picture | Space |
| First / last picture | Home / End |
| Zoom | Mouse wheel, `+`, `-` |
| Actual size (100%) / back to fit | `1` / `0` |
| Next fit mode | `F` |
| Move a zoomed-in picture | Drag |
| Fullscreen | F11 or middle-click |
| Thumbnail view | Enter, double-click or Esc |
| Rotate right / left | `R` / `L` |
| Flip horizontally / vertically | `H` / `V` |
| Save the rotation or the edits to the file | Ctrl+S |
| Save as / save for web | Ctrl+Shift+S |
| Edit panel | `E` |
| Crop | `C` |
| Undo / redo an edit | Ctrl+Z / Ctrl+Y |
| Rename | F2 |
| Delete to the Recycle Bin | Delete |
| Copy the picture / copy its path / paste a picture | Ctrl+C / Ctrl+Shift+C / Ctrl+V |
| Show the file in Explorer | Ctrl+Enter |
| Mark / unmark | `M` or Insert |
| Copy / move to a folder | F8 / F7 |
| Information (metadata) | `I` |
| Slideshow | `S` |
| Colour picker (a click copies the HEX value) | `P` |
| Next / previous page of a multi-page TIFF | Right / Left (also Ctrl+PageDown / Ctrl+PageUp) |
| Toolbar / status bar | Tab / `B` |
| Background behind transparent parts | `T` |
| Open a file | Ctrl+O |
| Settings | Ctrl+, |

Good to know:

- **Esc never closes the program.** It closes the open panel, stops the slideshow, leaves
  fullscreen, and then shows the thumbnails of the picture's folder.
- **End of the folder.** At the last (or first) picture a red line shows on that side. Keep
  pressing for half a second and the viewer wraps around to the other end.
- **Zoomed in**, dragging moves the picture, up to a little past its edges. A picture that
  fits the window is not moved; dragging selects a part of it instead (see below).
- **Slideshow.** The time per picture, random order and looping are in the right-click menu
  and in Settings. Turning the mouse wheel stops the slideshow.
- **Multi-page TIFF.** A book with the page number (such as 1 / 9) shows at the right.
  The Right and Left arrows, or the arrow buttons at the bottom, turn the pages; PageDown, PageUp and the mouse wheel go to the
  next and previous picture.
- **Transparency.** `T` switches what shows behind transparent pictures: checkerboard,
  black, white, or the normal window background.
- **Mouse wheel.** It zooms by default. Settings can make it go to the next and previous
  picture instead. Ctrl + wheel always does the other one.

## The thumbnail view

Enter, double-click or Esc in the viewer shows the folder: the folder tree on the left,
thumbnails in the middle and a preview on the right. Subfolders come before the pictures,
as in Explorer, and a `..` tile leads to the parent folder.

| Action | Keys and mouse |
|---|---|
| Move around | Arrow keys, PageUp / PageDown, Home / End |
| Open a picture in the viewer | Enter or double-click |
| Go into a folder | Double-click it, or select it and press Enter |
| Back to the folder you came from (or the parent folder) | Backspace or the mouse back button |
| Previous / next folder | Mouse back / forward buttons, Alt+Left / Alt+Right |
| Mark / unmark a picture | Space, `M`, Insert |
| Mark a range | Shift+click |
| Add or remove one picture | Ctrl+click |
| Mark all / unmark all | Ctrl+A / Ctrl+Shift+A |
| Clear the marks | Esc |
| Copy / move the marked files to a folder | F8 / F7 |
| Delete to the Recycle Bin | Delete |
| Copy / cut / paste files | Ctrl+C / Ctrl+X / Ctrl+V |
| Show the selected file or folder in Explorer | Ctrl+Enter |
| Refresh (read the folder again) | F5 |
| Thumbnail size | Ctrl + mouse wheel, or the Size slider |
| Search | Ctrl+F |

Commands work on the marked pictures, or on the selected one when nothing is marked.

**The top bar** has the parent-folder button, the Subfolders switch (shows the pictures of
all subfolders too), the Hidden files switch, sorting, the file-type filter, the thumbnail
size and buttons for the file commands and tools. The buttons are icons; point at one to see
its name and key. Settings → Toolbar can change them to words.

**The clipboard works with Explorer.** Ctrl+C and Ctrl+X put the marked files (or the
selected picture or folder) on the Windows clipboard as files, so you can paste them in
Explorer, and files copied in Explorer can be pasted here. Pasting never overwrites a file:
a name that is taken gets " (2)" added.

**Right-click a folder** (a tile, or a name in the tree) for Open, Open in Explorer,
Slideshow, Slideshow with subfolders, Copy, Cut and Paste here.

**Hidden and system files** are left out unless you switch them on. The switch starts on
when SuperView is opened with a hidden file or folder.

**With subfolders**, pictures come in the order of the folder tree: a folder's own pictures
first, then each subfolder by name, each followed by its own subfolders.

## Selecting part of a picture

Drag on a picture that fits the window to select a rectangle. When the picture is zoomed in,
dragging moves it instead; choose **Selection mode** in the right-click menu first. Drag the
rectangle's edges to adjust it. Hold Shift while dragging for a square. Esc clears it.

With a selection:

- **Ctrl+C** copies that part at the picture's full resolution.
- **Enter** crops the picture to it.
- **Right-click** offers Copy selection, Save selection as JPG, Save (overwrite the file
  with the selection), Crop to selection, Zoom to selection, Blur selection (to hide a face
  or a number plate) and Search in Google Lens. The Lens command copies the selection and
  opens Lens in your browser, where Ctrl+V pastes it. Nothing is sent until you paste.

## Editing and saving

`E` opens the edit panel:

- Auto-fix
- Crop, with fixed proportions if wanted
- Resize
- Straighten
- Brightness, contrast and gamma
- Saturation and colour balance
- Sharpen, blur and noise reduction

The sliders preview live, and every step can be undone (Ctrl+Z) and redone (Ctrl+Y).
Nothing touches the file until you save.

- **Ctrl+S** writes the picture over its file without asking. The file keeps its creation
  and modification dates and its metadata. A JPEG that was only rotated or flipped is saved
  without recompression when its size allows it; otherwise it is re-encoded at quality 95.
- **Ctrl+Shift+S** opens Save as: choose the format and quality, see the real file size
  before saving, and preview the picture at that quality. It also converts between formats
  and saves small files for the web (JPEG and WebP).

Unsaved edits are never lost by moving to another picture: SuperView stays on the picture
until you save them with Ctrl+S or discard them by pressing Esc twice.

**RAW develop.** On a camera RAW file the edit panel can read the sensor data instead of the
camera's embedded picture, with exposure, warmth, tint and highlight recovery. Save the
result with Save as; RAW files are never overwritten.

## Batch tools

These work on the marked files, or on every picture shown when nothing is marked.

- **Batch** (Ctrl+B) — convert to another format, resize (longest side, width, height or
  percent), rotate, auto-fix and remove metadata, in any combination. A progress bar shows
  the work and a Stop button ends it. Results go to the same folder or to one you choose.
  A file that keeps its format is replaced and keeps its dates; a converted file is added
  next to the original. An existing file in another target folder is never overwritten.
- **Batch rename** (Ctrl+R) — a pattern with `{name}`, `{n}` or `{n:3}` (a counter, here
  with three digits), `{date}` and `{time}`, with a preview of the new names. A pattern that
  would give two files the same name, or overwrite another file, is refused.
- **Information** (`I`) — shows the dimensions, format, file size, date and folder, the
  camera details, and the generation text (prompt and settings) that AI image generators
  store in PNG, JPEG and WebP files, with a Copy button. It also lets you change the
  description, artist, copyright and date taken, for one picture or for all marked ones
  (JPEG, PNG, WebP and TIFF). The picture is not recompressed and the file keeps its dates.

## Search and duplicates

- **Search** (Ctrl+F) adds a search row to the thumbnail view. Type words that must appear
  in the file name or folder path, in any order. You can add a size range in MB and a date
  range (`2024`, `2024-06` or `2024-06-15`). Switch on Subfolders to search everything below
  the folder. Esc closes the search.
- **Find duplicates** (Ctrl+D) finds files with exactly the same contents, or pictures that
  look the same although the files differ (resized, re-saved or converted copies). A slider
  sets how alike they must be. Results come in groups, largest file first. "Mark all but
  the largest of each group" marks the rest, which you can then delete or move. The search
  itself deletes nothing.

## Formats

| Format | Viewing | Saving |
|---|---|---|
| JPEG, PNG, BMP, WebP, TIFF | yes | Ctrl+S and Save as |
| GIF, animated PNG, animated WebP | yes, animations play | Save as another format |
| Multi-page TIFF | yes, page by page | Save as another format |
| HEIC / HEIF, AVIF | yes | Save as another format |
| Camera RAW (CR2, CR3, NEF, ARW, DNG, ORF, RW2, RAF, PEF, SRW and others) | yes | Save as another format |
| PSD (8-bit RGB) | yes, the flattened picture | Save as another format |

Save as writes JPEG, PNG, WebP, BMP and TIFF.

- **HEIC and AVIF** are decoded by Windows. HEIC needs "HEIF Image Extensions" and "HEVC
  Video Extensions" from the Microsoft Store; AVIF needs "AV1 Video Extension". Windows 11
  usually has them already.
- **Camera RAW** files show the full-size picture the camera stored inside the file, which
  is instant. Files without one are developed from the sensor data, which takes a few
  seconds.
- Pictures are turned the right way up from their EXIF orientation.

## Settings

Ctrl+, opens the settings.

- **General** — start folder, thumbnail cache, background behind transparent pictures,
  theme (light, dark or as Windows), what the mouse wheel does, fit mode and zoom when
  opening a picture, slideshow options, and what "Slideshow with SuperView" does (include
  subfolders, start playing by itself).
- **Keys** — give any command your own key.
- **Toolbar** — show a toolbar in the viewer and choose its buttons; icons or words in the
  thumbnail view.
- **Status bar** — choose the fields: file name, position in the folder, width × height,
  format, file size, date and zoom.
- **Windows** — add SuperView to Explorer, or remove it again.

## Command line

```
superview.exe <picture>              show the picture and the rest of its folder
superview.exe <folder>               show the thumbnails of that folder
superview.exe --recursive <folder>   show the folder with all its subfolders
superview.exe --slideshow <folder>   open the folder's first picture in the viewer
superview.exe --register             add SuperView to Explorer
superview.exe --unregister           remove it from Explorer
```

## Where SuperView keeps its files

- **Settings:** `%APPDATA%\SuperView\settings.toml`. Delete it to start afresh.
- **Thumbnail cache:** `%LOCALAPPDATA%\SuperView\cache`, one file per folder. Only folders
  whose pictures add up to more than 100 MB get one (the limit is adjustable); smaller
  folders are fast enough without. Settings shows the cache size and can clear it.
- Nothing is ever written into your picture folders.

## Building from source

You need Rust, CMake and NASM (NASM is for the fast JPEG code).

```
cargo build --release -p sv-app
cargo test --workspace
```

The result is `target\release\superview.exe`.

The code is a Rust workspace with two crates:

- `crates/sv-core` — decoding, thumbnails, editing, saving, batch work, search, duplicates,
  file operations and settings. No GUI.
- `crates/sv-app` — the window, built with [egui](https://github.com/emilk/egui).

## License

[MIT](LICENSE)

ACDSee and IrfanView are trademarks of their respective owners. SuperView is an independent
project and is not affiliated with or endorsed by them.
