//! The troubleshooting table, as reads.
//!
//! From daedric-online-linux (Step 6/7, Step 8, the troubleshooting table,
//! docs/the-texture-corruption-war.md, docs/the-linux-case-war.md):
//! - Vanilla `Skyrim - Textures*.bsa` with `.dds` entries that hold no DDS
//!   data: the launcher's downloader corrupted LZ4 blocks. Fix: the Step 7
//!   console depots + `downgrade-1.6.1170.sh`. Never the launcher's repair.
//! - Loose files under `textures/` / `meshes/` with capital letters and no
//!   lowercase twin. Linux is case-sensitive. Fix: `case-normalize.sh`.
//! - A behavior engine (Pandora / Nemesis) with no generated behavior output:
//!   A-poses. Fix: regenerate behaviors after every deploy.
//! - `SkyrimPrefs.ini` `[Display]`: `bFull Screen=1`, `bBorderless=0`,
//!   `iSize W` / `iSize H` set — or SSE Display Tweaks owns the window.
//!
//! These reads do not move or change files.

use crate::field::{ci_child, steam_library_root, wine_prefix_from_asar};
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const BSA_VERSION_SSE: u32 = 0x69;
const F_COMPRESSED: u32 = 0x4;
const F_EMBEDDED_NAMES: u32 = 0x100;
const F_SIZE_COMPRESS: u32 = 0x4000_0000;
const LZ4_FRAME_MAGIC: [u8; 4] = [0x04, 0x22, 0x4D, 0x18];
const CASE_SAMPLE: usize = 8;

const BEHAVIOR_ENGINES: &[&str] = &[
    "Pandora_Engine",
    "Pandora Behaviour Engine",
    "Pandora",
    "Nemesis_Engine",
];
const BEHAVIOR_OUTPUT: &str = "meshes/actors/character/behaviors/0_master.hkx";
const DISPLAY_TWEAKS: &str = "SKSE/Plugins/SSEDisplayTweaks.dll";

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub kind: String,
    pub name: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BsaVerdict {
    pub name: String,
    pub dds: usize,
    pub bad: usize,
    pub samples: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TroubleshootReport {
    pub textures: Vec<BsaVerdict>,
    pub case_misses: usize,
    pub case_samples: Vec<String>,
    pub behavior_engine: Option<String>,
    pub behavior_generated: bool,
    pub prefs: Option<PathBuf>,
    pub display_tweaks: bool,
    pub failed: usize,
    pub note: String,
    pub issues: Vec<Finding>,
}

pub fn read_troubleshoot(skyrim_root: &Path, asar_path: &Path) -> TroubleshootReport {
    let data = ci_child(skyrim_root, "Data").unwrap_or_else(|| skyrim_root.join("Data"));
    let mut issues = Vec::new();

    let textures = check_texture_bsas(&data);
    for v in &textures {
        if let Some(err) = &v.error {
            issues.push(Finding {
                kind: "bsa-unreadable".into(),
                name: v.name.clone(),
                detail: format!("{}: {err}. Reinstall it from the Step 7 depots.", v.name),
            });
        } else if v.bad > 0 {
            issues.push(Finding {
                kind: "lz4-corrupt".into(),
                name: v.name.clone(),
                detail: format!(
                    "{}: {} of {} textures hold no DDS data — the launcher downloader's LZ4 corruption. Run the Step 7 console depots, then scripts/downgrade-1.6.1170.sh. Never the launcher's repair.",
                    v.name, v.bad, v.dds
                ),
            });
        }
    }

    let (case_misses, case_samples) = if cfg!(windows) {
        (0, Vec::new())
    } else {
        case_misses(&data)
    };
    if case_misses > 0 {
        issues.push(Finding {
            kind: "case".into(),
            name: case_samples.first().cloned().unwrap_or_default(),
            detail: format!(
                "{case_misses} loose file(s) under textures/ or meshes/ have capital letters and no lowercase twin (e.g. {}). Linux is case-sensitive. Run scripts/case-normalize.sh (idempotent).",
                case_samples.first().map(String::as_str).unwrap_or("?")
            ),
        });
    }

    let behavior_engine = BEHAVIOR_ENGINES
        .iter()
        .find(|e| ci_child(&data, e).is_some_and(|p| p.is_dir()))
        .map(|e| e.to_string());
    let behavior_generated = resolve_ci(&data, BEHAVIOR_OUTPUT).is_some_and(|p| p.is_file());
    if let Some(engine) = &behavior_engine {
        if !behavior_generated {
            let tool = if engine.starts_with("Nemesis") {
                "Nemesis"
            } else {
                "Pandora"
            };
            issues.push(Finding {
                kind: "behavior".into(),
                name: engine.clone(),
                detail: format!(
                    "{tool} is installed but no generated behaviors exist ({BEHAVIOR_OUTPUT}). That is the A-pose. Run {tool}, update/generate, deploy its output, then rsync Data into the prefix."
                ),
            });
        }
    }

    let display_tweaks = resolve_ci(&data, DISPLAY_TWEAKS).is_some_and(|p| p.is_file());
    let prefs = find_prefs(skyrim_root, asar_path);
    if !display_tweaks {
        if let Some(path) = &prefs {
            issues.extend(check_prefs(path));
        }
    }

    let note = if issues.is_empty() {
        let mut bits = vec![format!(
            "{} textures BSA(s) clean",
            textures.iter().filter(|v| v.error.is_none()).count()
        )];
        if !cfg!(windows) {
            bits.push("no case misses".into());
        }
        if behavior_engine.is_some() {
            bits.push("behaviors generated".into());
        }
        bits.push(if display_tweaks {
            "SSE Display Tweaks owns the window".into()
        } else if prefs.is_some() {
            "fullscreen prefs set".into()
        } else {
            "SkyrimPrefs.ini not found".into()
        });
        bits.join(", ") + "."
    } else {
        format!("{} troubleshooting finding(s).", issues.len())
    };

    TroubleshootReport {
        textures,
        case_misses,
        case_samples,
        behavior_engine,
        behavior_generated,
        prefs,
        display_tweaks,
        failed: issues.len(),
        note,
        issues,
    }
}

fn check_texture_bsas(data: &Path) -> Vec<BsaVerdict> {
    let Ok(rd) = std::fs::read_dir(data) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|n| n.starts_with("skyrim - textures") && n.ends_with(".bsa"))
        })
        .collect();
    paths.sort();
    paths.iter().map(|p| check_bsa(p)).collect()
}

/// Every `.dds` entry in an SSE (v105) BSA must start with DDS data: raw,
/// behind a short frame, or an LZ4 frame when stored compressed.
pub fn check_bsa(path: &Path) -> BsaVerdict {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    match scan_bsa(path) {
        Ok((dds, bad, samples)) => BsaVerdict {
            name,
            dds,
            bad,
            samples,
            error: None,
        },
        Err(e) => BsaVerdict {
            name,
            dds: 0,
            bad: 0,
            samples: Vec::new(),
            error: Some(e),
        },
    }
}

fn scan_bsa(path: &Path) -> Result<(usize, usize, Vec<String>), String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hdr = [0u8; 36];
    f.read_exact(&mut hdr)
        .map_err(|_| "short header".to_string())?;
    if &hdr[..4] != b"BSA\0" {
        return Err("not a BSA".into());
    }
    let u32_at = |b: &[u8], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let version = u32_at(&hdr, 4);
    if version != BSA_VERSION_SSE {
        return Err(format!("unsupported BSA version 0x{version:x}"));
    }
    let folders_off = u32_at(&hdr, 8) as u64;
    let flags = u32_at(&hdr, 12);
    let folder_count = u32_at(&hdr, 16) as usize;
    let file_count = u32_at(&hdr, 20) as usize;
    let file_names_len = u32_at(&hdr, 28) as usize;
    if folder_count > 1 << 20 || file_count > 1 << 22 || file_names_len > 1 << 28 {
        return Err("implausible header".into());
    }

    f.seek(SeekFrom::Start(folders_off))
        .map_err(|e| e.to_string())?;
    let mut recs = vec![0u8; folder_count * 24];
    f.read_exact(&mut recs)
        .map_err(|_| "short folder records".to_string())?;
    let counts: Vec<usize> = (0..folder_count)
        .map(|i| u32_at(&recs, i * 24 + 8) as usize)
        .collect();

    let mut entries = Vec::with_capacity(file_count);
    for count in counts {
        let mut len = [0u8; 1];
        f.read_exact(&mut len)
            .map_err(|_| "short folder name".to_string())?;
        let mut name = vec![0u8; len[0] as usize];
        f.read_exact(&mut name)
            .map_err(|_| "short folder name".to_string())?;
        let folder = String::from_utf8_lossy(&name)
            .trim_end_matches('\0')
            .to_string();
        let mut recs = vec![0u8; count * 16];
        f.read_exact(&mut recs)
            .map_err(|_| "short file records".to_string())?;
        for i in 0..count {
            let sizefl = u32_at(&recs, i * 16 + 8);
            let off = u32_at(&recs, i * 16 + 12) as u64;
            entries.push((folder.clone(), sizefl, off));
        }
    }
    let mut names = vec![0u8; file_names_len];
    f.read_exact(&mut names)
        .map_err(|_| "short name block".to_string())?;
    let names: Vec<String> = names
        .split(|b| *b == 0)
        .take(entries.len())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .collect();
    if names.len() != entries.len() {
        return Err(format!(
            "name count mismatch {} vs {}",
            names.len(),
            entries.len()
        ));
    }

    let mut dds = 0;
    let mut bad = 0;
    let mut samples = Vec::new();
    for ((folder, sizefl, off), name) in entries.iter().zip(&names) {
        if !name.to_ascii_lowercase().ends_with(".dds") {
            continue;
        }
        dds += 1;
        if !entry_is_dds(&mut f, flags, *sizefl, *off) {
            bad += 1;
            if samples.len() < CASE_SAMPLE {
                samples.push(format!("{folder}\\{name}"));
            }
        }
    }
    Ok((dds, bad, samples))
}

fn entry_is_dds(f: &mut File, flags: u32, sizefl: u32, off: u64) -> bool {
    let mut pos = off;
    if flags & F_EMBEDDED_NAMES != 0 {
        let mut len = [0u8; 1];
        if f.seek(SeekFrom::Start(off)).is_err() || f.read_exact(&mut len).is_err() {
            return false;
        }
        pos = off + 1 + len[0] as u64;
    }
    let mut head = [0u8; 256];
    if f.seek(SeekFrom::Start(pos)).is_err() {
        return false;
    }
    let n = f.read(&mut head).unwrap_or(0);
    let head = &head[..n];
    if head.windows(4).any(|w| w == b"DDS ") {
        return true;
    }
    let compressed = (flags & F_COMPRESSED != 0) != (sizefl & F_SIZE_COMPRESS != 0);
    compressed && head.len() >= 8 && head[4..8] == LZ4_FRAME_MAGIC
}

/// Loose files under textures/ and meshes/ whose path has capitals and whose
/// all-lowercase path does not exist.
fn case_misses(data: &Path) -> (usize, Vec<String>) {
    let mut misses = 0;
    let mut samples = Vec::new();
    let Ok(rd) = std::fs::read_dir(data) else {
        return (0, samples);
    };
    for ent in rd.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        let low = name.to_ascii_lowercase();
        if (low == "textures" || low == "meshes") && ent.path().is_dir() {
            for file in walkdir::WalkDir::new(ent.path())
                .follow_links(false)
                .into_iter()
                .flatten()
                .filter(|e| e.file_type().is_file())
            {
                let Ok(rel) = file.path().strip_prefix(data) else {
                    continue;
                };
                let rel = rel.to_string_lossy().to_string();
                let lower = rel.to_ascii_lowercase();
                if rel == lower || data.join(&lower).exists() {
                    continue;
                }
                misses += 1;
                if samples.len() < CASE_SAMPLE {
                    samples.push(rel.replace('\\', "/"));
                }
            }
        }
    }
    (misses, samples)
}

const PREFS_PARTS: &[&str] = &[
    "Documents",
    "My Games",
    "Skyrim Special Edition",
    "SkyrimPrefs.ini",
];

fn prefs_in_prefix(prefix: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(users) = ci_child(prefix, "drive_c").and_then(|d| ci_child(&d, "users")) else {
        return out;
    };
    let Ok(rd) = std::fs::read_dir(users) else {
        return out;
    };
    for user in rd.flatten() {
        if let Some(p) = resolve_parts(&user.path(), PREFS_PARTS) {
            out.push(p);
        }
    }
    out.sort_by_key(|p| !p.to_string_lossy().contains("steamuser"));
    out
}

/// SkyrimPrefs.ini: the launcher's prefix first (umu: `steamuser`), then
/// Steam compatdata for the game library, then the Windows Documents folder.
pub fn find_prefs(skyrim_root: &Path, asar_path: &Path) -> Option<PathBuf> {
    if let Some(prefix) = wine_prefix_from_asar(asar_path) {
        if let Some(p) = prefs_in_prefix(&prefix).into_iter().next() {
            return Some(p);
        }
    }
    if let Some(lib) = steam_library_root(skyrim_root) {
        let pfx = ci_child(&lib, "steamapps")
            .and_then(|s| ci_child(&s, "compatdata"))
            .and_then(|c| ci_child(&c, "489830"))
            .and_then(|a| ci_child(&a, "pfx"));
        if let Some(p) = pfx.and_then(|p| prefs_in_prefix(&p).into_iter().next()) {
            return Some(p);
        }
    }
    if cfg!(windows) {
        if let Some(profile) = std::env::var_os("USERPROFILE") {
            return resolve_parts(Path::new(&profile), PREFS_PARTS);
        }
    }
    None
}

/// `[Display]` keys, case-insensitive, spaces inside key names kept.
pub fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_section = line
                .trim_matches(|c| c == '[' || c == ']')
                .eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim().eq_ignore_ascii_case(key) {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

pub fn check_prefs(path: &Path) -> Vec<Finding> {
    let text = std::fs::read(path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut want = |key: &str, ok: &dyn Fn(&str) -> bool, fix: &str| {
        let got = ini_value(&text, "Display", key);
        if !got.as_deref().is_some_and(ok) {
            out.push(Finding {
                kind: "display".into(),
                name: key.to_string(),
                detail: format!(
                    "SkyrimPrefs.ini [Display] {key} is {}. Set {fix} — or install SSE Display Tweaks.",
                    got.as_deref().unwrap_or("unset")
                ),
            });
        }
    };
    want("bFull Screen", &|v| v == "1", "bFull Screen=1");
    want("bBorderless", &|v| v == "0", "bBorderless=0");
    let size = |v: &str| v.parse::<u32>().is_ok_and(|n| n > 0);
    want("iSize W", &size, "iSize W to your monitor width");
    want("iSize H", &size, "iSize H to your monitor height");
    out
}

fn resolve_parts(root: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for part in parts {
        cur = ci_child(&cur, part)?;
    }
    Some(cur)
}

fn resolve_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let parts: Vec<&str> = rel.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    resolve_parts(root, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "daedric-trouble-{label}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("Data")).unwrap();
        p
    }

    /// One folder, `.dds` payloads as given. Uncompressed archive.
    fn write_bsa(path: &Path, files: &[(&str, &[u8])]) {
        let folder = b"textures\\x\0";
        let names: Vec<u8> = files
            .iter()
            .flat_map(|(n, _)| n.bytes().chain(std::iter::once(0)))
            .collect();
        let folders_off = 36u32;
        let recs_start = folders_off + 24;
        let data_start =
            recs_start + 1 + folder.len() as u32 + 16 * files.len() as u32 + names.len() as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"BSA\0");
        for v in [
            0x69u32,
            folders_off,
            0x3,
            1,
            files.len() as u32,
            folder.len() as u32,
            names.len() as u32,
            2,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&[0u8; 8]);
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.push(folder.len() as u8);
        out.extend_from_slice(folder);
        let mut off = data_start;
        for (_, body) in files {
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&off.to_le_bytes());
            off += body.len() as u32;
        }
        out.extend_from_slice(&names);
        for (_, body) in files {
            out.extend_from_slice(body);
        }
        std::fs::write(path, out).unwrap();
    }

    #[test]
    fn garbage_dds_in_a_textures_bsa_is_lz4_corruption() {
        let root = temp("bsa");
        write_bsa(
            &root.join("Data/Skyrim - Textures0.bsa"),
            &[
                ("a.dds", b"DDS |good"),
                ("b.dds", b"\xde\xad\xbe\xefgarbage"),
            ],
        );
        let r = read_troubleshoot(&root, Path::new("/nowhere/app.asar"));
        assert_eq!(r.textures.len(), 1);
        assert_eq!(r.textures[0].dds, 2);
        assert_eq!(r.textures[0].bad, 1);
        assert_eq!(r.textures[0].samples, ["textures\\x\\b.dds"]);
        let hit = r.issues.iter().find(|i| i.kind == "lz4-corrupt").unwrap();
        assert!(hit.detail.contains("Step 7"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn clean_bsa_passes() {
        let root = temp("bsa-ok");
        write_bsa(
            &root.join("Data/Skyrim - Textures1.bsa"),
            &[("a.dds", b"DDS |x")],
        );
        let r = read_troubleshoot(&root, Path::new("/nowhere/app.asar"));
        assert_eq!(r.textures[0].bad, 0);
        assert_eq!(r.failed, 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn capital_loose_file_without_twin_points_at_case_normalize() {
        let root = temp("case");
        let d = root.join("Data/Textures/Armor");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("Iron.dds"), b"x").unwrap();
        let r = read_troubleshoot(&root, Path::new("/nowhere/app.asar"));
        assert_eq!(r.case_misses, 1);
        assert!(r
            .issues
            .iter()
            .any(|i| i.kind == "case" && i.detail.contains("case-normalize.sh")));
        let low = root.join("Data/textures/armor");
        std::fs::create_dir_all(&low).unwrap();
        std::fs::write(low.join("iron.dds"), b"x").unwrap();
        assert_eq!(
            read_troubleshoot(&root, Path::new("/nowhere/app.asar")).case_misses,
            0
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn engine_without_output_is_the_a_pose() {
        let root = temp("behavior");
        std::fs::create_dir_all(root.join("Data/Nemesis_Engine")).unwrap();
        let r = read_troubleshoot(&root, Path::new("/nowhere/app.asar"));
        assert!(r
            .issues
            .iter()
            .any(|i| i.kind == "behavior" && i.detail.contains("A-pose")));
        let beh = root.join("Data/meshes/actors/character/behaviors");
        std::fs::create_dir_all(&beh).unwrap();
        std::fs::write(beh.join("0_master.hkx"), b"x").unwrap();
        let r = read_troubleshoot(&root, Path::new("/nowhere/app.asar"));
        assert!(r.behavior_generated);
        assert!(!r.issues.iter().any(|i| i.kind == "behavior"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn prefs_in_umu_prefix_need_exclusive_fullscreen() {
        let root = temp("prefs");
        let prefix = root.join("umu-489830");
        let dir = prefix.join("drive_c/users/steamuser/Documents/My Games/Skyrim Special Edition");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SkyrimPrefs.ini"),
            "[Display]\nbFull Screen=0\nbBorderless=1\niSize W=2560\niSize H=1080\n",
        )
        .unwrap();
        let asar = prefix.join("drive_c/Program Files/DaedricOnline/resources/app.asar");
        let r = read_troubleshoot(&root, &asar);
        let names: Vec<_> = r
            .issues
            .iter()
            .filter(|i| i.kind == "display")
            .map(|i| i.name.as_str())
            .collect();
        assert_eq!(names, ["bFull Screen", "bBorderless"]);
        std::fs::write(
            dir.join("SkyrimPrefs.ini"),
            "[Display]\nbFull Screen=1\nbBorderless=0\niSize W=2560\niSize H=1080\n",
        )
        .unwrap();
        assert_eq!(read_troubleshoot(&root, &asar).failed, 0);
        std::fs::write(dir.join("SkyrimPrefs.ini"), "[Display]\nbFull Screen=0\n").unwrap();
        let tweaks = root.join("Data/SKSE/Plugins");
        std::fs::create_dir_all(&tweaks).unwrap();
        std::fs::write(tweaks.join("SSEDisplayTweaks.dll"), b"x").unwrap();
        let r = read_troubleshoot(&root, &asar);
        assert!(r.display_tweaks);
        assert_eq!(r.failed, 0);
        let _ = std::fs::remove_dir_all(root);
    }
}
