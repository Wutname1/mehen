# Bundled resources

Release builds place `mehen-setup.exe` here: the published copy from
`cdn.gitwyrm.com/mehen/Mehen-Setup.exe`, built from `src-tauri/bootstrapper`.
Run with `--updating`, it shows the "Updating Mehen" window that covers the gap
between the old app exiting and the new one starting while the installer runs
quietly. See `spawn_update_cover` in `src-tauri/src/self_update.rs`.

The binary is **not committed**. The release workflow downloads it, and
`npm run bootstrapper:build` makes a local one to copy here for a local build.

This file exists so the bundler's `resources/**/*` glob always matches
something. Without it, a dev build with no helper fails outright.
