# Visual Workbench quick start

This guide describes the current development app. Check `docs/evidence/` for
the tested build and outstanding checks. Current isolated app tests also run on
the S23 Ultra; neither automated phone suite establishes physical S Pen
performance or daily-use acceptance.

1. **Open an image.** On Windows, choose **File → Import file** (`Ctrl+I`)
   or drop a PNG, JPEG or WebP into the window. On the phone, choose **Photos**,
   **Browse files**, **Take photo**, or **New canvas**. Imports create projects;
   reopening a project restores its saved edits. Keep a copy of important
   originals while using development builds.

2. **Pair the devices.** Put them on the same local network. On Windows, choose
   **Workbench → Pair devices** (`Ctrl+P`), select the local address for that
   network, and press **Show QR**. On the phone, open **Pair a computer**, allow
   the camera, scan the QR, and press **Pair with this computer**. If the camera
   is unavailable, choose **Show eight-digit code** on Windows, enter its address
   and code on the phone, then compare the complete fingerprints on both screens
   before confirming. Pairing offers expire; create a new offer if necessary.

3. **Share the project.** Pairing and opening a shared project are separate
   actions. With an image open on Windows, select the paired phone, enter a
   listener address for the chosen network, and press **Share project**. On the
   phone, select that computer, enter the displayed session address and port,
   choose the corresponding connection type, and press **Receive computer
   project**. A complete local copy is saved on the phone. For an existing
   replica, use **Connect current project**. Wait for the connection status to
   settle before assuming that an edit has reached the other device.

4. **Mark it up.** Choose Pen, a shape, or Text. On Windows, `P` selects Pen,
   `V` selects objects, and `H` pans; `F` fits the image and `Ctrl+1` shows actual
   pixels. Drag a selected object to move it or its lower-right handle to resize
   it. Change the color and width in Properties. `Ctrl+Z` undoes an edit and
   `Ctrl+Shift+Z` redoes it. On the phone, the pen draws and finger gestures move
   the view. View sharing and following the other device are explicit controls.

5. **Export and hand off.** Choose **File → Export image** (`Ctrl+E`) on Windows
   or Export on the phone. Choose Marked or Clean, the area, and PNG/JPEG/WebP.
   Review the revision and dimensions, then **Save file…**. For Claude Code,
   attach the saved image in your own terminal and review it before sending.
   Windows also offers **Copy PNG + DIBV5** and **Prepare file drag**; after
   preparing a drag, use **Drag prepared PNG** on the transfer shelf. These
   actions transfer an image only when you choose them. Receiving applications
   can handle clipboard and file attachments differently.

If connecting fails, check that the displayed addresses belong to the selected
local network and that both devices are still paired. Reopening the connection
screen shows the current status. USB tethering must be enabled again after some
cable reconnects. Route changes and Windows firewall changes require a separate
review; do not apply an unrelated machine-wide fix. Disconnecting leaves saved
projects available offline. Revoking a pairing requires pairing again later.

The Phase 2 build adds foreground-window capture, region selection and masks,
instruction packages, local MCP tools and explicit image-edit requests. Configure
agent connections using [MCP setup](MCP_SETUP.md). Review capture grants and
prepared results before sending anything. Image providers require your own
configured credentials; local regression tests do not verify their output quality
or billing. The task evidence records remaining acceptance for each feature.
