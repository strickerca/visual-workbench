# Back up, update, recover and remove Visual Workbench

This guide covers the current development Android app and complete Windows app
image. It describes source-backed procedures; uninstall, restore and installer
acceptance have not been performed for the current cloud build. An MSI, signed
release APK, driver or plugin package needs its own admitted delivery and removal
instructions. The current bounded Paint build installs no driver or certificate.

## Before an update or removal

1. Finish or explicitly discard open instruction, image-edit and output drafts.
   In a frozen capture, use Save and wait for its durable result. If **Retry same
   Save** appears, retry that operation before resuming or closing. A displayed
   preview or temporary frozen annotation is not a saved project backup.
2. Close capture/tunnel/remote views normally. If cleanup or input retirement is
   pending, retain the owning application and use its cleanup controls. A forced
   process termination is not proof that input, rendering or native work retired.
3. Preserve original inputs and needed exports. For editable recovery, preserve
   the complete project and its assets, not only an exported PNG or SQLite file.
   Check needed phone edits on a retained second copy before removing phone data;
   a connection label by itself is not a backup check.
4. Close the project/application normally before copying its project directory.
   Keep the original backup unchanged; restore into a separate working folder.

## Project locations and what to preserve

| Platform | Current storage | Backup boundary |
| --- | --- | --- |
| Windows | New projects use the actual Windows Documents folder's `Visual Workbench` directory. If Documents is recognized as cloud-backed, new projects use `%LOCALAPPDATA%\Visual Workbench\Projects`. A manually opened project can be elsewhere. | Record the actual project's folder. With the application closed, copy the entire folder, including `project.sqlite`, its accompanying SQLite files if present, and the complete `blobs` tree. Keep the copy out of an active cloud-synced SQLite location. |
| Android | Projects are under the application's private `files/projects` storage. Normal image Save/Share exports rendered images, not an editable project backup. | Preserve a checked editable copy on the paired computer before uninstalling or clearing app data. No user-facing full-project backup/restore UI or access to another app's private directory is promised by this guide. |

Project folders and captures can contain private content. Keep backups private;
do not attach them to public issue/evidence logs. Pairing identity, operation
counters, provider credentials and peer labels are separate app state. Copying a
project or app-image folder does not transfer those credentials or establish a
safe identity migration. This guide does not reset or copy them.

## Windows update and removal

Use a newly delivered complete app-image folder. Preserve its `app`, `runtime`,
native resources, runtime legal files and notices together. After the old app
and its owned views close normally, launch the new image from its own folder;
do not mix files from two builds. Check the delivered receipt against the exact
image being launched. An assembly `COMPLETE` marker is not installer or runtime
acceptance.

For removal of this folder-based development build, first preserve projects and
close the application normally, then remove only its delivered app-image folder.
Project/data folders are separate. Do not recursively delete Documents or
`%LOCALAPPDATA%\Visual Workbench` as an application-removal shortcut: they may
contain projects, identity and other retained state. The separate legacy-spelled
`%LOCALAPPDATA%\VisualWorkbench\transfer-work` path is temporary transfer storage,
not the project root; it is not a substitute for a project backup or an automatic
cleanup target while work is pending.

No registry, firewall, security setting, certificate or driver removal is part
of this procedure. If a future delivery is an MSI, use that delivery's reviewed
installer/removal instructions; the current cloud work has not produced or
tested one.

## Android update and removal

Prefer a normal same-application, same-signer update that preserves app data.
If Android reports a signing mismatch, keep the existing app and data while the
delivery is corrected. Do not uninstall as a workaround. Keep test/HIL packages
separate from the normal `com.visualworkbench.android` application.

Android uninstall or app-data clearing removes local projects and app state.
Proceed only after checking the needed independent copies and exports. This
guide does not install/uninstall an app, change a device/profile, clear data or
authorize a new signing identity. Pairing and provider setup after reinstallation
must follow the normal explicit flows; an image export does not restore them.

## Recovery checks

For a Windows project backup, make a new local working copy of the complete
closed project folder. Choose **File → Open project** and select `project.sqlite`
inside that working copy. Verify the original image, annotations, instructions,
layers, saved outputs and required history, then make a fresh export and inspect
it. Preserve the untouched backup while the current application performs any
normal schema migration. Do not force database edits, strip WAL files or merge
two project directories by hand.

If a project cannot reopen, keep the complete folder and the exact app/build
receipt. Retain the displayed error text and use the normal diagnostics export
with its redaction preview. Do not publish raw project content, credentials or
device identifiers to diagnose the failure. Restore/round-trip and A20 acceptance
for an actual delivered Windows/Android package remain pending.

Implementation references: [Windows project-location policy](../apps/desktop/src/main/kotlin/com/visualworkbench/desktop/DesktopState.kt),
[Desktop entry and retirement](../apps/desktop/src/main/kotlin/com/visualworkbench/desktop/Main.kt),
[Android project repository](../apps/android/src/main/kotlin/com/visualworkbench/android/editor/ProjectRepository.kt),
[database layout](../core/crates/vw-store/src/database.rs),
[archive asset layout](../core/crates/vw-store/src/archive.rs),
and [personal installation history](M4_KP_INSTALLATION.md).
