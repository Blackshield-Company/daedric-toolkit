//! Play blockers the hash gate cannot see.
//!
//! Probed from the live launcher asar (readPeFileVersion / checkRuntime,
//! releaseLock.ts, skymp toggle):
//! - SkyrimSE.exe FileVersion, first three numbers, against `1.6.1170`.
//!   A newer or older EXE blocks Play. `skse64_loader.exe` missing does too;
//!   Play would install it, but it is not there now.
//! - `DaedricData/release-state.json` plus `release-lock.<server>.json`.
//!   Both absent is normal — the launcher only writes them after a verified
//!   lock. A `.tmp` left by `writeAtomic`, or a state/lock pair that does
//!   not match, is a half-finished update.
//! - `DaedricData/skymp-disabled/state.json` means play-elsewhere is on.
//!   The SkyMP client files are parked. Play will not reach the server.

use serde::Serialize;
use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const REQUIRED_RUNTIME: &str = "1.6.1170";
const FIXEDFILEINFO_SIGNATURE: u32 = 0xFEEF_04BD;
const SERVER_RE_MAX: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct PlayIssue {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReport {
    pub installed: Option<String>,
    pub required: String,
    /// `ok`, `too-new`, `too-old`, `outside`, `unknown`, `missing`.
    pub verdict: String,
    pub loader_present: bool,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<PlayIssue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseReport {
    /// `clear`, `recorded`, `torn`, `interrupted`.
    pub verdict: String,
    pub servers: Vec<String>,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<PlayIssue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElsewhereReport {
    pub disabled: bool,
    pub when: Option<String>,
    pub parked: u64,
    pub overlay_version: Option<String>,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<PlayIssue>,
}

pub fn accepted_runtimes(game_versions: &[String]) -> Vec<String> {
    let kept: Vec<String> = game_versions
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && parse_version(s).is_some())
        .collect();
    if kept.is_empty() {
        vec![REQUIRED_RUNTIME.to_string()]
    } else {
        kept
    }
}

pub fn read_runtime(skyrim_root: &Path, accepted: &[String]) -> RuntimeReport {
    let accepted = accepted_runtimes(accepted);
    let required = accepted.join(", ");
    let exe = find_named(skyrim_root, "SkyrimSE.exe");
    let loader_present = find_named(skyrim_root, "skse64_loader.exe").is_some();
    let mut issues = Vec::new();

    let (installed, verdict) = match exe.as_ref().and_then(|p| read_pe_file_version(p)) {
        Some(parts) => {
            let installed = format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3]);
            let verdict = runtime_verdict(&installed, &accepted);
            (Some(installed), verdict)
        }
        None if exe.is_none() => (None, "missing"),
        None => (None, "unknown"),
    };

    if verdict != "ok" {
        let detail = match verdict {
            "missing" => "SkyrimSE.exe is not in the game folder.".to_string(),
            "unknown" => "SkyrimSE.exe has no readable FileVersion.".to_string(),
            "too-new" => format!(
                "Skyrim is {}. Daedric Online needs {required}. A newer build will not load the script extender. Steam updated it: re-run scripts/downgrade-1.6.1170.sh (Step 7), then lock steamapps/appmanifest_489830.acf read-only with Steam closed.",
                installed.as_deref().unwrap_or("unreadable")
            ),
            "too-old" => format!(
                "Skyrim is {}. Daedric Online needs {required}.",
                installed.as_deref().unwrap_or("unreadable")
            ),
            _ => format!(
                "Skyrim is {}. Accepted: {required}.",
                installed.as_deref().unwrap_or("unreadable")
            ),
        };
        issues.push(PlayIssue {
            kind: verdict.to_string(),
            detail,
        });
    }
    if !loader_present {
        issues.push(PlayIssue {
            kind: "loader".into(),
            detail: "skse64_loader.exe is missing. Play would install it, but it is not there now."
                .into(),
        });
    }

    let note = if issues.is_empty() {
        format!(
            "Skyrim {} — the build Daedric Online is built for. Loader present.",
            installed.as_deref().unwrap_or(REQUIRED_RUNTIME)
        )
    } else {
        issues
            .iter()
            .map(|i| i.detail.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    };

    RuntimeReport {
        installed,
        required,
        verdict: verdict.to_string(),
        loader_present,
        failed: issues.len(),
        note,
        issues,
    }
}

pub fn read_release_book(skyrim_root: &Path) -> ReleaseReport {
    let dd = skyrim_root.join("DaedricData");
    let mut issues = Vec::new();
    let mut interrupted = false;

    if dd.join("release-state.json.tmp").is_file() {
        interrupted = true;
        issues.push(PlayIssue {
            kind: "interrupted".into(),
            detail: "release-state.json.tmp is still there. The launcher write did not finish."
                .into(),
        });
    }
    let lock_tmps = list_release_locks(&dd, true);
    for name in &lock_tmps {
        interrupted = true;
        issues.push(PlayIssue {
            kind: "interrupted".into(),
            detail: format!("{name} is still there. The lock write did not finish."),
        });
    }

    let state_path = dd.join("release-state.json");
    let parsed_state = if state_path.is_file() {
        match std::fs::read_to_string(&state_path) {
            Ok(text) => parse_release_state(&text),
            Err(e) => {
                issues.push(PlayIssue {
                    kind: "unreadable".into(),
                    detail: format!("release-state.json could not be read: {e}"),
                });
                None
            }
        }
    } else {
        None
    };
    let state_unreadable = state_path.is_file()
        && parsed_state.is_none()
        && !issues.iter().any(|i| i.kind == "unreadable");
    if state_unreadable {
        issues.push(PlayIssue {
            kind: "unreadable".into(),
            detail: "release-state.json is not a v1 book. The launcher would ignore it.".into(),
        });
    }
    let servers = parsed_state.clone().unwrap_or_default();
    let locks = list_release_locks(&dd, false);

    for server in servers.keys() {
        let expected = format!("release-lock.{server}.json");
        if !locks.iter().any(|n| n == &expected) {
            issues.push(PlayIssue {
                kind: "torn".into(),
                detail: format!("state records {server} but {expected} is missing."),
            });
        }
    }
    for name in &locks {
        let server = lock_server_from_name(name);
        match server {
            Some(server) if servers.contains_key(&server) => match lock_serial(&dd.join(name)) {
                Some(serial) => {
                    if let Some(seen) = servers.get(&server) {
                        if seen != &serial {
                            issues.push(PlayIssue {
                                kind: "serial".into(),
                                detail: format!(
                                    "{name} serial {serial} does not match state maxSerial {seen}."
                                ),
                            });
                        }
                    }
                }
                None => issues.push(PlayIssue {
                    kind: "unreadable".into(),
                    detail: format!(
                        "{name} is not a release envelope. The cached lock is incomplete."
                    ),
                }),
            },
            Some(server) => issues.push(PlayIssue {
                kind: "torn".into(),
                detail: format!("{name} has no matching state entry for {server}."),
            }),
            None => issues.push(PlayIssue {
                kind: "torn".into(),
                detail: format!("{name} is not a launcher lock name."),
            }),
        }
    }

    let server_names: Vec<String> = servers.keys().cloned().collect();
    let ignored_only = !interrupted
        && locks.is_empty()
        && server_names.is_empty()
        && !issues.is_empty()
        && issues.iter().all(|i| i.kind == "unreadable");
    let verdict = if interrupted {
        "interrupted"
    } else if ignored_only || (issues.is_empty() && server_names.is_empty()) {
        "clear"
    } else if !issues.is_empty() {
        "torn"
    } else {
        "recorded"
    };
    let note = if ignored_only {
        "release-state.json is unreadable. The launcher treats that as an empty book.".to_string()
    } else {
        match verdict {
            "clear" => {
                "No release book. The launcher has not recorded a verified lock.".to_string()
            }
            "recorded" => format!(
                "Recorded {}. A matched pair is not a stuck update.",
                server_names.join(", ")
            ),
            _ => issues
                .iter()
                .map(|i| i.detail.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        }
    };
    // An unreadable state with no locks is what the launcher treats as empty.
    // Do not fail Play for that alone.
    let failed = if verdict == "clear" { 0 } else { issues.len() };
    let issues = if verdict == "clear" {
        Vec::new()
    } else {
        issues
    };

    ReleaseReport {
        verdict: verdict.to_string(),
        servers: server_names,
        failed,
        note,
        issues,
    }
}

pub fn read_play_elsewhere(skyrim_root: &Path) -> ElsewhereReport {
    let park = skyrim_root.join("DaedricData").join("skymp-disabled");
    let state_path = park.join("state.json");
    let files_parked = dir_has_files(&park.join("files"));
    let mut issues = Vec::new();

    let parsed = std::fs::read_to_string(&state_path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&strip_bom(&text)).ok())
        .filter(|v| !v.is_null());

    let (disabled, when, parked, overlay_version) = if let Some(v) = parsed {
        let when = v.get("when").and_then(|x| x.as_str()).map(str::to_string);
        let parked = v.get("parked").and_then(value_u64).unwrap_or(0);
        let overlay_version = v
            .get("overlayVersion")
            .and_then(|x| x.as_str())
            .map(str::to_string);
        (true, when, parked, overlay_version)
    } else {
        (false, None, 0, None)
    };

    if disabled {
        let when_bit = when
            .as_deref()
            .map(|w| format!(" Parked {when}.", when = w))
            .unwrap_or_default();
        let overlay_bit = overlay_version
            .as_deref()
            .map(|v| format!(" Overlay {v}."))
            .unwrap_or_default();
        issues.push(PlayIssue {
            kind: "disabled".into(),
            detail: format!(
                "Play-elsewhere is on. SkyMP client files are parked ({parked} moved).{when_bit}{overlay_bit} Play will not reach the server."
            ),
        });
    } else if files_parked {
        issues.push(PlayIssue {
            kind: "parked".into(),
            detail: "Client files sit in skymp-disabled/files, but the toggle does not say disabled. The launcher would try to Play with them missing.".into(),
        });
    }

    let note = if issues.is_empty() {
        "SkyMP client is enabled.".to_string()
    } else {
        issues[0].detail.clone()
    };

    ElsewhereReport {
        disabled,
        when,
        parked,
        overlay_version,
        failed: issues.len(),
        note,
        issues,
    }
}

fn runtime_verdict(installed: &str, accepted: &[String]) -> &'static str {
    let Some(got) = parse_version(installed) else {
        return "unknown";
    };
    let got3 = first3(&got);
    let accepted3: Vec<[u32; 3]> = accepted
        .iter()
        .filter_map(|s| parse_version(s).map(|p| first3(&p)))
        .collect();
    if accepted3.is_empty() {
        return "unknown";
    }
    if accepted3.iter().any(|a| a == &got3) {
        return "ok";
    }
    if accepted3.iter().all(|a| cmp3(got3, *a) > 0) {
        return "too-new";
    }
    if accepted3.iter().all(|a| cmp3(got3, *a) < 0) {
        return "too-old";
    }
    "outside"
}

fn parse_version(s: &str) -> Option<Vec<u32>> {
    let parts: Vec<u32> = s
        .split('.')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn first3(parts: &[u32]) -> [u32; 3] {
    [
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    ]
}

fn cmp3(a: [u32; 3], b: [u32; 3]) -> i32 {
    for i in 0..3 {
        if a[i] != b[i] {
            return if a[i] > b[i] { 1 } else { -1 };
        }
    }
    0
}

fn find_named(dir: &Path, name: &str) -> Option<PathBuf> {
    let want = name.to_ascii_lowercase();
    for ent in std::fs::read_dir(dir).ok()?.flatten() {
        if ent.file_name().to_string_lossy().to_ascii_lowercase() == want {
            return Some(ent.path());
        }
    }
    None
}

/// Launcher `readPeFileVersion`: VS_FIXEDFILEINFO out of the PE resource directory.
pub fn read_pe_file_version(path: &Path) -> Option<[u16; 4]> {
    let mut file = File::open(path).ok()?;
    let dos = read_at(&mut file, 0, 64)?;
    if u16_at(&dos, 0)? != 0x5A4D {
        return None;
    }
    let pe_off = u32_at(&dos, 60)? as u64;
    let coff = read_at(&mut file, pe_off, 24)?;
    if u32_at(&coff, 0)? != 0x4550 {
        return None;
    }
    let num_sections = u16_at(&coff, 6)? as u32;
    let opt_size = u16_at(&coff, 20)? as u64;
    if opt_size < 96 {
        return None;
    }
    let opt = read_at(&mut file, pe_off + 24, opt_size)?;
    let magic = u16_at(&opt, 0)?;
    let dir_base = if magic == 0x20B { 112 } else { 96 };
    let rsrc_at = dir_base + 2 * 8;
    if rsrc_at + 8 > opt.len() {
        return None;
    }
    let rsrc_rva = u32_at(&opt, rsrc_at)?;
    if rsrc_rva == 0 {
        return None;
    }
    let sec_bytes = read_at(&mut file, pe_off + 24 + opt_size, num_sections as u64 * 40)?;
    let mut sections = Vec::new();
    let mut rsrc_section = None;
    for i in 0..num_sections as usize {
        let s = i * 40;
        let section = Section {
            virtual_size: u32_at(&sec_bytes, s + 8)?,
            virtual_address: u32_at(&sec_bytes, s + 12)?,
            raw_size: u32_at(&sec_bytes, s + 16)?,
            raw_offset: u32_at(&sec_bytes, s + 20)?,
        };
        if section.virtual_address == rsrc_rva {
            rsrc_section = Some(section);
        }
        sections.push(section);
    }
    let rsrc_section = rsrc_section?;
    if rsrc_section.raw_size == 0 || rsrc_section.raw_size > 32 * 1024 * 1024 {
        return None;
    }
    let rsrc = read_at(
        &mut file,
        rsrc_section.raw_offset as u64,
        rsrc_section.raw_size as u64,
    )?;
    let (type_off, type_dir) = resource_child(&rsrc, 0, Some(16))?;
    if !type_dir {
        return None;
    }
    let (name_off, name_dir) = resource_child(&rsrc, type_off, None)?;
    if !name_dir {
        return None;
    }
    let (lang_off, lang_dir) = resource_child(&rsrc, name_off, None)?;
    if lang_dir {
        return None;
    }
    let data_rva = u32_at(&rsrc, lang_off)?;
    let data_size = u32_at(&rsrc, lang_off + 4)? as u64;
    if data_size == 0 || data_size > 1_048_576 {
        return None;
    }
    let data_off = rva_to_offset(&sections, data_rva)?;
    let version = read_at(&mut file, data_off, data_size)?;
    let mut i = 0;
    while i + 52 <= version.len() {
        if u32_at(&version, i)? == FIXEDFILEINFO_SIGNATURE {
            let ms = u32_at(&version, i + 8)?;
            let ls = u32_at(&version, i + 12)?;
            return Some([
                (ms >> 16) as u16,
                (ms & 0xFFFF) as u16,
                (ls >> 16) as u16,
                (ls & 0xFFFF) as u16,
            ]);
        }
        i += 4;
    }
    None
}

#[derive(Clone, Copy)]
struct Section {
    virtual_size: u32,
    virtual_address: u32,
    raw_size: u32,
    raw_offset: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Option<u64> {
    for s in sections {
        let span = s.virtual_size.max(s.raw_size);
        let end = s.virtual_address.saturating_add(span);
        if rva >= s.virtual_address && rva < end {
            return Some(s.raw_offset as u64 + (rva - s.virtual_address) as u64);
        }
    }
    None
}

fn resource_child(rsrc: &[u8], dir_offset: usize, wanted: Option<u32>) -> Option<(usize, bool)> {
    let named = u16_at(rsrc, dir_offset + 12)? as usize;
    let ids = u16_at(rsrc, dir_offset + 14)? as usize;
    let first = dir_offset + 16;
    for i in 0..(named + ids) {
        let e = first + i * 8;
        let name_or_id = u32_at(rsrc, e)?;
        let offset_to_data = u32_at(rsrc, e + 4)?;
        let is_named = (name_or_id & 0x8000_0000) != 0;
        if let Some(want) = wanted {
            if is_named || name_or_id != want {
                continue;
            }
        }
        let offset = (offset_to_data & 0x7FFF_FFFF) as usize;
        let is_dir = (offset_to_data & 0x8000_0000) != 0;
        return Some((offset, is_dir));
    }
    None
}

fn read_at(file: &mut File, offset: u64, len: u64) -> Option<Vec<u8>> {
    let len = usize::try_from(len).ok()?;
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn u16_at(buf: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(buf: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(at..at + 4)?.try_into().ok()?))
}

fn parse_release_state(text: &str) -> Option<std::collections::BTreeMap<String, i64>> {
    let v: Value = serde_json::from_str(&strip_bom(text)).ok()?;
    if v.get("v")?.as_i64()? != 1 {
        return None;
    }
    let servers = v.get("servers")?.as_object()?;
    let mut out = std::collections::BTreeMap::new();
    for (k, s) in servers {
        if !is_server_key(k) {
            continue;
        }
        let serial = s.get("maxSerial").and_then(value_i64)?;
        out.insert(k.clone(), serial);
    }
    Some(out)
}

fn is_server_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= SERVER_RE_MAX
        && k.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn lock_server_from_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("release-lock.")?;
    let server = rest.strip_suffix(".json")?;
    if is_server_key(server) {
        Some(server.to_string())
    } else {
        None
    }
}

fn list_release_locks(dd: &Path, tmp: bool) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(rd) = std::fs::read_dir(dd) else {
        return names;
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        let hit = if tmp {
            name.starts_with("release-lock.") && name.ends_with(".json.tmp")
        } else {
            name.starts_with("release-lock.") && name.ends_with(".json") && !name.ends_with(".tmp")
        };
        if hit && ent.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    names
}

fn lock_serial(path: &Path) -> Option<i64> {
    let text = std::fs::read_to_string(path).ok()?;
    let env: Value = serde_json::from_str(&strip_bom(&text)).ok()?;
    let body = env.get("body")?.as_str()?;
    let lock: Value = serde_json::from_str(body).ok()?;
    lock.get("serial").and_then(value_i64)
}

fn value_i64(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|f| f as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn value_u64(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_f64().map(|f| f as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

fn strip_bom(text: &str) -> String {
    text.trim_start_matches('\u{feff}').to_string()
}

fn dir_has_files(path: &Path) -> bool {
    fn walk(path: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(path) else {
            return false;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            if p.is_file() || (p.is_dir() && walk(&p)) {
                return true;
            }
        }
        false
    }
    walk(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_game(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("daedric-play-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("DaedricData")).unwrap();
        p
    }

    #[test]
    fn first_three_numbers_are_the_runtime_gate() {
        assert_eq!(
            runtime_verdict("1.6.1170.0", &[REQUIRED_RUNTIME.into()]),
            "ok"
        );
        assert_eq!(runtime_verdict("1.6.1170", &["1.6.1170.0".into()]), "ok");
        assert_eq!(
            runtime_verdict("1.6.1179.0", &[REQUIRED_RUNTIME.into()]),
            "too-new"
        );
        assert_eq!(
            runtime_verdict("1.6.659.0", &[REQUIRED_RUNTIME.into()]),
            "too-old"
        );
        assert_eq!(
            runtime_verdict("1.5.97.0", &[REQUIRED_RUNTIME.into()]),
            "too-old"
        );
    }

    #[test]
    fn empty_accepted_list_falls_back_to_the_launcher_constant() {
        assert_eq!(accepted_runtimes(&[]), vec![REQUIRED_RUNTIME.to_string()]);
    }

    #[test]
    fn missing_release_book_is_not_a_failure() {
        let root = temp_game("clear");
        let report = read_release_book(&root);
        assert_eq!(report.verdict, "clear");
        assert_eq!(report.failed, 0);
        assert!(report.note.contains("No release book"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn matched_release_pair_is_recorded() {
        let root = temp_game("recorded");
        let dd = root.join("DaedricData");
        std::fs::write(
            dd.join("release-state.json"),
            r#"{"v":1,"servers":{"play":{"maxSerial":4,"lockSha256":"abc","seenAt":"2026-09-30T00:00:00Z"}}}"#,
        )
        .unwrap();
        std::fs::write(
            dd.join("release-lock.play.json"),
            r#"{"v":1,"alg":"ed25519","keyId":"k","sig":"aa","body":"{\"server\":\"play\",\"serial\":4}"}"#,
        )
        .unwrap();
        let report = read_release_book(&root);
        assert_eq!(report.verdict, "recorded");
        assert_eq!(report.failed, 0);
        assert_eq!(report.servers, vec!["play".to_string()]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn leftover_tmp_is_an_interrupted_write() {
        let root = temp_game("tmp");
        std::fs::write(root.join("DaedricData/release-state.json.tmp"), "{").unwrap();
        let report = read_release_book(&root);
        assert_eq!(report.verdict, "interrupted");
        assert!(report.failed >= 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn state_without_a_lock_is_torn() {
        let root = temp_game("torn");
        std::fs::write(
            root.join("DaedricData/release-state.json"),
            r#"{"v":1,"servers":{"play":{"maxSerial":1}}}"#,
        )
        .unwrap();
        let report = read_release_book(&root);
        assert_eq!(report.verdict, "torn");
        assert!(report.failed >= 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn play_elsewhere_is_the_state_file() {
        let root = temp_game("else");
        let off = read_play_elsewhere(&root);
        assert!(!off.disabled);
        assert_eq!(off.failed, 0);

        let park = root.join("DaedricData/skymp-disabled");
        std::fs::create_dir_all(&park).unwrap();
        std::fs::write(
            park.join("state.json"),
            r#"{"when":"2026-09-30T00:00:00.000Z","overlayVersion":"4.99.608","parked":12}"#,
        )
        .unwrap();
        let on = read_play_elsewhere(&root);
        assert!(on.disabled);
        assert_eq!(on.parked, 12);
        assert_eq!(on.failed, 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parked_files_without_a_toggle_still_fail() {
        let root = temp_game("parked");
        let files = root.join("DaedricData/skymp-disabled/files/Data/SKSE/Plugins");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("MpClientPlugin.dll"), b"x").unwrap();
        let report = read_play_elsewhere(&root);
        assert!(!report.disabled);
        assert_eq!(report.failed, 1);
        assert_eq!(report.issues[0].kind, "parked");
        let _ = std::fs::remove_dir_all(root);
    }
}
