![daedric-toolkit](.github/preview.png)

# Daedric Toolkit

Install doctor for [Daedric Online](https://daedriconline.com). It reads the launcher's `app.asar` contract and checks a Skyrim Special Edition install against it: plugin hashes, loose-file pins, SKSE DLL allowlist, archive checksums, overlay-version drift, quarantined plugins, the Plugins.txt load order, the launcher's install ledger, the SkyrimSE.exe build, the release book, the play-elsewhere toggle, known-bad files, foreign client plugins, parked Anniversary Edition files, the launch method, the Steam update guard, and the Linux troubleshooting checks.

A failing gate is often version drift, not a corrupt file. If the installed overlay is older than the contract, the doctor says so instead of calling the files broken.

## Launch

The umu launch is the default on every distro: `umu-run` with `UMU_USE_STEAM=1`, `PROTONPATH` set to GE-Proton11-7, and the `umu-489830` prefix. The Launch panel prints that command with this machine's paths.

```bash
cd "$HOME/Games/umu/umu-489830/drive_c/Program Files/DaedricOnline"
export GAMEID=umu-489830 UMU_ID=umu-489830 UMU_USE_STEAM=1
export PROTONPATH="$HOME/.local/share/Steam/compatibilitytools.d/GE-Proton11-7-x86_64"
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$HOME/.local/share/Steam"
export SteamAppId=489830 SteamGameId=489830
umu-run "Daedric Online.exe" --no-sandbox
```

The Steam non-Steam shortcut is a fallback, only when umu fails: the log shows `path /var/home` or `steam_install_path ""`, or the launcher opens and Skyrim never spawns. A lone `path /home` warning is normal. On Windows, start `Daedric Online.exe` directly.

Launching through Steam can update Skyrim off 1.6.1170. Before using the shortcut, close Steam fully and lock the manifest:

```bash
chmod 444 "<library>/steamapps/appmanifest_489830.acf"   # chmod 644 reverts it
```

Steam offline mode is the backup. If Skyrim already updated, re-run `scripts/downgrade-1.6.1170.sh` from daedric-online-linux.

## Troubleshooting checks

| Finding | Fix |
|---|---|
| `.dds` entries in `Skyrim - Textures*.bsa` with no DDS data | LZ4 corruption from the launcher's downloader. Run the Step 7 console depots and `downgrade-1.6.1170.sh`. Never the launcher's repair. |
| Loose files under `textures/` or `meshes/` with capitals and no lowercase twin | `bash scripts/case-normalize.sh` (Linux only; idempotent) |
| Pandora or Nemesis installed, no `meshes/actors/character/behaviors/0_master.hkx` | A-poses. Regenerate behaviors, deploy, rsync Data into the prefix. |
| `SkyrimPrefs.ini` `[Display]` not `bFull Screen=1`, `bBorderless=0`, `iSize W`/`iSize H` | Set them, or install SSE Display Tweaks. |

The ini is read from the launcher prefix (`drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition/SkyrimPrefs.ini`), then Steam compatdata `489830`, then `%USERPROFILE%\Documents\My Games\Skyrim Special Edition\SkyrimPrefs.ini` on Windows.

## Build

Requires Rust and the Tauri 2 CLI.

```bash
cargo test -p daedric-core
npx --yes @tauri-apps/cli@2 build
```

The frontend is static HTML/CSS/JS in `frontend/dist`. No bundler. Path fields are empty until you fill them; the app remembers them locally after that.

## Field check

```bash
export DAEDRIC_ASAR="/path/to/DaedricOnline/resources/app.asar"
export DAEDRIC_SKYRIM="/path/to/Skyrim Special Edition"
cargo run -p daedric-core --example decode
cargo run -p daedric-core --example doctor
```

Paths are resolved case-insensitively, one component at a time. That matches Wine on a case-sensitive Linux filesystem.

## License

MIT. See [LICENSE](LICENSE). Tags before this change stay Apache-2.0.
