# Intcad 0.1.3

This release brings the Assistant into the window, puts the built-in agent in the
installers, and adds accounts with a verified paid Review path.

## The Assistant

- The Assistant now sits at the bottom centre of the window, with a single message box
  and the conversation opening above it.
- Changing which agent answers no longer changes the size of the message box.
- Collapsing the Assistant leaves the compact status row readable, and the panel keeps
  its place when you resize the window.
- The agent's `skill/<name>` commands appear as you type `/` and can be picked without
  sending a message.

## The built-in agent

- The desktop installers carry the built-in Mastra agent and its Node runtime, so the
  Assistant can answer without a separate download.
- The agent is installed with the four modelling skills (modelling, editing, checking
  and mechanisms) and the project brief skill.
- Skills are copied into your own agent data folder on first start; nothing in your
  design files changes.

## Accounts and Review

- Sign in with an email address in the system browser, stay signed in across restarts,
  sign out, and switch accounts.
- The title bar and the Start page show the signed-in state, and account deletion
  instructions are reachable without leaving the app.
- Signed in, online and authorised, the paid Review capability uses your own account;
  anonymous and offline use of CAD, completion, save, export, undo and redo is
  unchanged.

## Packages and updates

- Linux, Windows and macOS packages now include the third-party licence texts and
  notices for everything they ship.
- Update checks and downloads move to the public distribution repository. Update
  checksum lists are signed with Minisign and are verified before an update is applied;
  a wrong, missing or tampered signature stops the update.
- The Windows installer replaces the previous installation and keeps a single entry in
  installed programs.

## Known limits in this release

- Windows Authenticode, Apple Developer ID signing and notarization are not part of
  this release.
- An installed 0.1.2 cannot update itself: its update check looks for a checksum file
  that no published release provides. Install 0.1.3 over it the same way you installed
  0.1.2; designs, preferences and the signed-in session are kept.

## Installing over 0.1.2

Download the 0.1.3 package for your platform and install it over the old copy. Your
designs, preferences and account session live in your user data and are not touched by
the installer.
