//! How the launcher is started, and what keeps Steam off the 1.6.1170 build.
//!
//! From daedric-online-linux (README Step 4, Fedora Atomic / Bazzite,
//! docs/the-atomic-steam-bridge.md):
//! - The umu launch is the default on every distro: `umu-run` with
//!   `UMU_USE_STEAM=1`, `PROTONPATH` = GE-Proton11-7, `GAMEID` = `umu-489830`,
//!   from inside `Program Files/DaedricOnline` with the bare exe name.
//! - The Steam / non-Steam shortcut is a fallback, only when umu failed:
//!   `path /var/home`, `steam_install_path ""`, an S_API load failure with
//!   `UMU_USE_STEAM=1` set, or a launcher that opens and never spawns Skyrim.
//! - Launching through Steam can update Skyrim off 1.6.1170 even with
//!   "only update when I launch". The guard: close Steam, `chmod 444`
//!   `<library>/steamapps/appmanifest_489830.acf` (`chmod 644` reverts),
//!   offline mode as a backup, and `downgrade-1.6.1170.sh` if it already moved.
//!
//! These reads do not move or change files.

use crate::field::{ci_child, steam_library_root, wine_prefix_from_asar};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const UMU_GAME_ID: &str = "umu-489830";
pub const STEAM_APP_ID: &str = "489830";
pub const PROTON_NAME: &str = "GE-Proton11-7";
pub const LAUNCHER_EXE: &str = "Daedric Online.exe";
pub const DOWNGRADE_SCRIPT: &str = "scripts/downgrade-1.6.1170.sh";

#[derive(Debug, Clone, Serialize)]
pub struct LaunchIssue {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LaunchReport {
    /// `umu`, `steam-shortcut`, `windows`, or `unknown`.
    pub method: String,
    pub prefix: Option<PathBuf>,
    pub proton: Option<PathBuf>,
    pub log: Option<PathBuf>,
    /// The default launch, line by line, with this machine's paths.
    pub recipe: Vec<String>,
    /// The Steam-shortcut steps. Only for a failed umu launch.
    pub fallback: Vec<String>,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<LaunchIssue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateGuardReport {
    pub manifest: Option<PathBuf>,
    pub auto_update: Option<String>,
    pub state_flags: Option<u32>,
    /// True when the manifest is read-only (`chmod 444` / read-only attribute).
    pub locked: Option<bool>,
    pub shortcut: bool,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<LaunchIssue>,
}

/// What the launcher's prefix says about how it is started.
pub fn launch_method(asar_path: &Path) -> &'static str {
    let Some(prefix) = wine_prefix_from_asar(asar_path) else {
        return if looks_windows(asar_path) {
            "windows"
        } else {
            "unknown"
        };
    };
    if is_compatdata_prefix(&prefix) {
        return "steam-shortcut";
    }
    "umu"
}

fn looks_windows(path: &Path) -> bool {
    let s = path.to_string_lossy();
    cfg!(windows) || (s.len() > 2 && s.as_bytes()[1] == b':' && s.contains('\\'))
}

/// `.../steamapps/compatdata/<id>/pfx` — the prefix Steam builds for a shortcut.
fn is_compatdata_prefix(prefix: &Path) -> bool {
    let name = |p: Option<&Path>| {
        p.and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    };
    name(Some(prefix)) == "pfx" && name(prefix.parent().and_then(|p| p.parent())) == "compatdata"
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// Native and Flatpak Steam roots, plus the library that holds the game.
pub fn steam_roots(skyrim_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(h) = home() {
        for rel in [
            ".local/share/Steam",
            ".steam/steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        ] {
            let p = h.join(rel);
            if p.is_dir() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    if let Some(lib) = steam_library_root(skyrim_root) {
        if !out.contains(&lib) {
            out.push(lib);
        }
    }
    out
}

/// `compatibilitytools.d/GE-Proton11-7*` with the `version` file umu reads.
pub fn find_proton(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let Some(tools) = ci_child(root, "compatibilitytools.d") else {
            continue;
        };
        let Ok(rd) = std::fs::read_dir(&tools) else {
            continue;
        };
        let mut hits: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(is_wanted_proton)
                    && p.join("version").is_file()
            })
            .collect();
        hits.sort();
        if let Some(hit) = hits.into_iter().next() {
            return Some(hit);
        }
    }
    None
}

fn is_wanted_proton(name: &str) -> bool {
    let rest = match name.strip_prefix(PROTON_NAME) {
        Some(r) => r,
        None => return false,
    };
    // GE-Proton11-7 or GE-Proton11-7-x86_64, never GE-Proton11-70 or aarch64.
    rest.is_empty() || rest.eq_ignore_ascii_case("-x86_64")
}

/// The default launch with this machine's paths. Linux only.
pub fn umu_recipe(prefix: &Path, proton: &Path, steam_root: &Path) -> Vec<String> {
    let dir = prefix
        .join("drive_c")
        .join("Program Files")
        .join("DaedricOnline");
    vec![
        format!("cd \"{}\"", dir.display()),
        format!("export GAMEID={UMU_GAME_ID}"),
        format!("export UMU_ID={UMU_GAME_ID}"),
        "export UMU_USE_STEAM=1".into(),
        format!("export PROTONPATH=\"{}\"", proton.display()),
        format!(
            "export STEAM_COMPAT_CLIENT_INSTALL_PATH=\"{}\"",
            steam_root.display()
        ),
        format!("export SteamAppId={STEAM_APP_ID}"),
        format!("export SteamGameId={STEAM_APP_ID}"),
        format!("umu-run \"{LAUNCHER_EXE}\" --no-sandbox"),
    ]
}

/// The fallback, in order. Only after the umu launch has failed.
pub fn shortcut_steps() -> Vec<String> {
    vec![
        format!("{PROTON_NAME} must be in compatibilitytools.d. Restart Steam if it is not in the list."),
        format!("Steam → Games → Add a Non-Steam Game → Browse to Program Files/DaedricOnline/{LAUNCHER_EXE}. Not DaedricOnline-Setup.exe, not SkyrimSE.exe."),
        format!("Properties → Compatibility → Force the use of a specific Steam Play compatibility tool → {PROTON_NAME}."),
        "Launch options: %command% --no-sandbox".into(),
        format!("Close Steam fully, chmod 444 <library>/steamapps/appmanifest_{STEAM_APP_ID}.acf, start Steam, press Play on the shortcut."),
        "Never press Play on Skyrim Special Edition in the library. Steam offline mode is the backup guard.".into(),
    ]
}

/// The umu failure signatures from a launcher log. A lone
/// `path /home` game-drive warning is normal umu noise and is not reported.
pub fn classify_umu_log(text: &str) -> Vec<LaunchIssue> {
    let mut out = Vec::new();
    let var_home = text
        .lines()
        .any(|l| l.contains("unable to use parent for game drive") && l.contains("path /var/home"));
    let empty_path = text.contains("steam_install_path \"\"");
    let s_api_fail = text.lines().any(|l| {
        l.contains("SteamAPI_Init")
            && l.contains("Failed to load module")
            && l.contains("steamclient64")
    });
    if var_home || empty_path {
        out.push(LaunchIssue {
            kind: "umu-failed".into(),
            detail: format!(
                "umu failed: {}{}. Proton cannot derive a Steam install path (Fedora Atomic home). UMU_USE_STEAM=1 does not fill it. Use the Steam shortcut fallback and lock appmanifest_{STEAM_APP_ID}.acf first.",
                if var_home { "game drive path /var/home" } else { "" },
                match (var_home, empty_path) {
                    (true, true) => " + steam_install_path \"\"",
                    (false, true) => "steam_install_path \"\"",
                    _ => "",
                }
            ),
        });
    } else if s_api_fail {
        out.push(LaunchIssue {
            kind: "s-api".into(),
            detail: "[S_API] failed to load steamclient64.dll. Export UMU_USE_STEAM=1 and start Steam first. If it is already set, umu failed: use the Steam shortcut fallback.".into(),
        });
    }
    if text.contains("NtGdiCreateColorSpace") {
        out.push(LaunchIssue {
            kind: "proton".into(),
            detail: format!(
                "Wine aborted on NtGdiCreateColorSpace. GE-Proton11-6 or older. Use {PROTON_NAME}."
            ),
        });
    }
    if text.contains("Executable not found") {
        out.push(LaunchIssue {
            kind: "exe-path".into(),
            detail: format!("umu: Executable not found. cd into Program Files/DaedricOnline and pass the bare \"{LAUNCHER_EXE}\"."),
        });
    }
    out
}

fn default_log() -> Option<PathBuf> {
    home()
        .map(|h| h.join(".local/share/daedric-online/launcher-stdio.log"))
        .filter(|p| p.is_file())
}

pub fn read_launch(skyrim_root: &Path, asar_path: &Path, log: Option<&Path>) -> LaunchReport {
    let method = launch_method(asar_path);
    let prefix = wine_prefix_from_asar(asar_path);
    let roots = steam_roots(skyrim_root);
    let proton = find_proton(&roots);
    let mut issues = Vec::new();

    if method == "windows" {
        return LaunchReport {
            method: method.into(),
            prefix: None,
            proton: None,
            log: None,
            recipe: vec![format!(
                "Start {LAUNCHER_EXE} directly. umu and Proton are Linux only."
            )],
            fallback: Vec::new(),
            failed: 0,
            note: "Windows install. Start the launcher directly; it stays the only entry point."
                .into(),
            issues,
        };
    }

    if method == "umu" {
        let name = prefix
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name != UMU_GAME_ID {
            issues.push(LaunchIssue {
                kind: "prefix".into(),
                detail: format!(
                    "Launcher prefix is \"{name}\", not {UMU_GAME_ID}. Migrate it so the launcher and the game share one wineserver and one Steam bridge (Step 2)."
                ),
            });
        }
    }
    if proton.is_none() {
        issues.push(LaunchIssue {
            kind: "proton".into(),
            detail: format!(
                "{PROTON_NAME} not found in compatibilitytools.d (with its version file). Install the x86_64 build (Step 1)."
            ),
        });
    }

    let log = log.map(Path::to_path_buf).or_else(default_log);
    if let Some(text) = log.as_ref().and_then(|p| std::fs::read(p).ok()) {
        issues.extend(classify_umu_log(&String::from_utf8_lossy(&text)));
    }

    let steam_root = roots
        .iter()
        .find(|r| !r.to_string_lossy().contains("/steamapps/"))
        .cloned()
        .or_else(|| home().map(|h| h.join(".local/share/Steam")))
        .unwrap_or_else(|| PathBuf::from("~/.local/share/Steam"));
    let recipe_prefix = match (method, &prefix) {
        ("umu", Some(p)) => p.clone(),
        _ => home()
            .map(|h| h.join("Games/umu").join(UMU_GAME_ID))
            .unwrap_or_else(|| PathBuf::from("~/Games/umu").join(UMU_GAME_ID)),
    };
    let recipe_proton = proton.clone().unwrap_or_else(|| {
        steam_root
            .join("compatibilitytools.d")
            .join(format!("{PROTON_NAME}-x86_64"))
    });
    let recipe = umu_recipe(&recipe_prefix, &recipe_proton, &steam_root);

    let umu_failed = issues.iter().any(|i| i.kind == "umu-failed");
    let note = match method {
        "steam-shortcut" if umu_failed => "Steam shortcut fallback, and the log shows umu failed. Keep appmanifest_489830.acf locked.".to_string(),
        "steam-shortcut" => format!(
            "Launcher runs from a Steam compatdata prefix (the shortcut fallback). The umu launch is the default; use the shortcut only if umu failed, with appmanifest_{STEAM_APP_ID}.acf locked."
        ),
        _ if umu_failed => "umu launch failed. Fall back to the Steam shortcut (steps below), manifest locked first.".to_string(),
        _ if issues.is_empty() => format!(
            "umu launch: {UMU_GAME_ID}, {PROTON_NAME}, UMU_USE_STEAM=1. The default on every distro."
        ),
        _ => issues
            .iter()
            .map(|i| i.detail.as_str())
            .collect::<Vec<_>>()
            .join(" "),
    };

    LaunchReport {
        method: method.into(),
        prefix,
        proton,
        log,
        recipe,
        fallback: shortcut_steps(),
        failed: issues.len(),
        note,
        issues,
    }
}

/// `<library>/steamapps/appmanifest_489830.acf` for the library holding Skyrim.
pub fn find_app_manifest(skyrim_root: &Path) -> Option<PathBuf> {
    let lib = steam_library_root(skyrim_root)?;
    let steamapps = ci_child(&lib, "steamapps")?;
    ci_child(&steamapps, &format!("appmanifest_{STEAM_APP_ID}.acf")).filter(|p| p.is_file())
}

/// One `"key"  "value"` pair out of a Valve KeyValues file.
pub fn acf_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let mut parts = line.split('"').filter(|s| !s.trim().is_empty());
        let k = parts.next()?;
        if !k.eq_ignore_ascii_case(key) {
            return None;
        }
        parts.next().map(|v| v.to_string())
    })
}

fn is_read_only(path: &Path) -> Option<bool> {
    let meta = std::fs::metadata(path).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(meta.permissions().mode() & 0o222 == 0)
    }
    #[cfg(not(unix))]
    {
        Some(meta.permissions().readonly())
    }
}

fn lock_command(manifest: &Path, lock: bool) -> String {
    if cfg!(windows) {
        format!(
            "attrib {}R \"{}\"",
            if lock { "+" } else { "-" },
            manifest.display()
        )
    } else {
        format!(
            "chmod {} \"{}\"",
            if lock { "444" } else { "644" },
            manifest.display()
        )
    }
}

pub fn read_update_guard(skyrim_root: &Path, asar_path: &Path) -> UpdateGuardReport {
    let shortcut = launch_method(asar_path) == "steam-shortcut";
    let manifest = find_app_manifest(skyrim_root);
    let mut issues = Vec::new();

    let Some(path) = manifest.clone() else {
        return UpdateGuardReport {
            manifest: None,
            auto_update: None,
            state_flags: None,
            locked: None,
            shortcut,
            failed: 0,
            note: format!(
                "appmanifest_{STEAM_APP_ID}.acf not found next to the game. Not a Steam library install, or the game path is outside steamapps/common."
            ),
            issues,
        };
    };
    let text = std::fs::read(&path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let auto_update = acf_value(&text, "AutoUpdateBehavior");
    let state_flags = acf_value(&text, "StateFlags").and_then(|v| v.trim().parse::<u32>().ok());
    let locked = is_read_only(&path);

    if auto_update.as_deref().map(str::trim) != Some("2") {
        issues.push(LaunchIssue {
            kind: "auto-update".into(),
            detail: format!(
                "AutoUpdateBehavior is {}, not 2. Re-run {DOWNGRADE_SCRIPT} (Step 7) — it sets the hold.",
                auto_update.as_deref().unwrap_or("unset")
            ),
        });
    }
    if state_flags.is_some_and(|f| f & 2 != 0) {
        issues.push(LaunchIssue {
            kind: "update-queued".into(),
            detail: "Steam has an update queued for Skyrim (StateFlags UpdateRequired). Close Steam and lock the manifest before it runs, or use offline mode.".into(),
        });
    }
    if shortcut && locked == Some(false) {
        issues.push(LaunchIssue {
            kind: "unlocked".into(),
            detail: format!(
                "Steam shortcut launch with an unlocked manifest: Steam can update Skyrim off 1.6.1170 even with \"only update when I launch\". Close Steam fully, then {}. {} reverts it. Offline mode is the backup. If it already updated, re-run {DOWNGRADE_SCRIPT}.",
                lock_command(&path, true),
                lock_command(&path, false)
            ),
        });
    }

    let note = if issues.is_empty() {
        match locked {
            Some(true) => format!(
                "Manifest locked read-only, AutoUpdateBehavior=2. Steam cannot move the build. {} when you want an update.",
                lock_command(&path, false)
            ),
            _ => format!(
                "AutoUpdateBehavior=2. On the Steam shortcut fallback also lock it: close Steam, {}.",
                lock_command(&path, true)
            ),
        }
    } else {
        issues
            .iter()
            .map(|i| i.detail.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    UpdateGuardReport {
        manifest,
        auto_update,
        state_flags,
        locked,
        shortcut,
        failed: issues.len(),
        note,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("daedric-launch-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn umu_prefix_is_the_default_method() {
        let asar = Path::new(
            "/home/u/Games/umu/umu-489830/drive_c/Program Files/DaedricOnline/resources/app.asar",
        );
        assert_eq!(launch_method(asar), "umu");
        let shortcut = Path::new(
            "/var/home/u/.local/share/Steam/steamapps/compatdata/3141592/pfx/drive_c/Program Files/DaedricOnline/resources/app.asar",
        );
        assert_eq!(launch_method(shortcut), "steam-shortcut");
    }

    #[test]
    fn recipe_carries_the_proven_env() {
        let r = umu_recipe(
            Path::new("/p/umu-489830"),
            Path::new("/s/compatibilitytools.d/GE-Proton11-7-x86_64"),
            Path::new("/s"),
        );
        let all = r.join("\n");
        assert!(all.contains("export UMU_USE_STEAM=1"));
        assert!(all.contains("GAMEID=umu-489830"));
        assert!(all.contains("GE-Proton11-7-x86_64"));
        assert!(r
            .last()
            .unwrap()
            .ends_with("umu-run \"Daedric Online.exe\" --no-sandbox"));
        assert!(r[0].contains("drive_c/Program Files/DaedricOnline"));
    }

    #[test]
    fn proton_name_is_exact() {
        assert!(is_wanted_proton("GE-Proton11-7"));
        assert!(is_wanted_proton("GE-Proton11-7-x86_64"));
        assert!(!is_wanted_proton("GE-Proton11-70"));
        assert!(!is_wanted_proton("GE-Proton11-7-aarch64"));
        assert!(!is_wanted_proton("GE-Proton11-6-x86_64"));
    }

    #[test]
    fn atomic_signature_is_umu_failure_but_plain_home_is_noise() {
        let bad = "Proton: Error: unable to use parent for game drive, path /var/home\nsetup_steam_files steam_install_path \"\"\n";
        let found = classify_umu_log(bad);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, "umu-failed");
        let noise = "Proton: Error: unable to use parent for game drive, path /home\n";
        assert!(classify_umu_log(noise).is_empty());
        let s_api = "[S_API] SteamAPI_Init(): Failed to load module 'C:\\Program Files (x86)\\Steam\\steamclient64.dll'";
        assert_eq!(classify_umu_log(s_api)[0].kind, "s-api");
    }

    #[test]
    fn acf_values_parse() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"489830\"\n\t\"StateFlags\"\t\t\"6\"\n\t\"AutoUpdateBehavior\"\t\t\"2\"\n}\n";
        assert_eq!(acf_value(acf, "AutoUpdateBehavior").as_deref(), Some("2"));
        assert_eq!(acf_value(acf, "stateflags").as_deref(), Some("6"));
        assert_eq!(acf_value(acf, "missing"), None);
    }

    #[cfg(unix)]
    #[test]
    fn shortcut_with_unlocked_manifest_fails_and_444_passes() {
        use std::os::unix::fs::PermissionsExt;
        let lib = temp("guard");
        let game = lib.join("steamapps/common/Skyrim Special Edition");
        std::fs::create_dir_all(&game).unwrap();
        let acf = lib.join("steamapps/appmanifest_489830.acf");
        std::fs::write(
            &acf,
            "\"AppState\"\n{\n\t\"StateFlags\"\t\t\"4\"\n\t\"AutoUpdateBehavior\"\t\t\"2\"\n}\n",
        )
        .unwrap();
        let asar = lib.join(
            "steamapps/compatdata/123/pfx/drive_c/Program Files/DaedricOnline/resources/app.asar",
        );
        std::fs::set_permissions(&acf, std::fs::Permissions::from_mode(0o644)).unwrap();
        let r = read_update_guard(&game, &asar);
        assert!(r.shortcut);
        assert_eq!(r.failed, 1);
        assert_eq!(r.issues[0].kind, "unlocked");
        assert!(r.issues[0].detail.contains("chmod 444"));
        assert!(r.issues[0].detail.contains("chmod 644"));
        std::fs::set_permissions(&acf, std::fs::Permissions::from_mode(0o444)).unwrap();
        let r = read_update_guard(&game, &asar);
        assert_eq!(r.failed, 0);
        assert_eq!(r.locked, Some(true));
        std::fs::set_permissions(&acf, std::fs::Permissions::from_mode(0o644)).unwrap();
        let _ = std::fs::remove_dir_all(lib);
    }

    #[test]
    fn auto_update_off_two_points_at_the_downgrade() {
        let lib = temp("auto");
        let game = lib.join("steamapps/common/Skyrim Special Edition");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(
            lib.join("steamapps/appmanifest_489830.acf"),
            "\"AutoUpdateBehavior\"\t\t\"0\"\n\"StateFlags\"\t\t\"6\"\n",
        )
        .unwrap();
        let r = read_update_guard(&game, Path::new("/x/umu-489830/drive_c/a/app.asar"));
        let kinds: Vec<_> = r.issues.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, ["auto-update", "update-queued"]);
        assert!(r.issues[0].detail.contains("downgrade-1.6.1170.sh"));
        let _ = std::fs::remove_dir_all(lib);
    }
}
