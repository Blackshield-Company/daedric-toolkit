//! What the launcher did to the install, as opposed to hash gates.
//!
//! Three reads, all informational except where they contradict the contract:
//! - `DaedricData/quarantine/<stamp>/` — foreign plugins pulled out of Data at Play.
//!   A quarantined file that is also a contract plugin is a failure. Foreign pulls are not.
//! - `Plugins.txt` — Skyrim SE load order (`*` = enabled). It does not live in the
//!   game directory. On Wine/Proton it is
//!   `<prefix>/drive_c/users/<user>/AppData/Local/Skyrim Special Edition/Plugins.txt`.
//!   The prefix is derived from the launcher asar (walk up to `drive_c`) and, if the
//!   game sits under a Steam library, from `steamapps/compatdata/*/pfx`.
//! - `DaedricData/installed-mods.json` — the launcher's own install ledger. It records
//!   mods the launcher unpacked, not the full collection. A short ledger is normal.

use crate::collection::{BuildManifest, CollectionLock};
use crate::extract::extract_module_json;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

const VANILLA: &[&str] = &[
    "skyrim.esm",
    "update.esm",
    "dawnguard.esm",
    "hearthfires.esm",
    "dragonborn.esm",
];

fn is_plugin_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
}

fn is_plugin_basename(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') {
        return false;
    }
    is_plugin_name(name)
}

/// Lowercased plugin basenames the hash contract pins. This is the Play-gate
/// set, not the load order — it includes engine masters the launcher never
/// writes into Plugins.txt.
pub fn plugin_set(lock: &CollectionLock, build: &BuildManifest) -> HashSet<String> {
    let mut set = HashSet::new();
    for name in build.plugins.keys().chain(lock.files.keys()) {
        if is_plugin_basename(name) {
            set.insert(name.to_ascii_lowercase());
        }
    }
    set
}

fn is_engine_master(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    matches!(
        n.as_str(),
        "skyrim.esm"
            | "update.esm"
            | "dawnguard.esm"
            | "hearthfires.esm"
            | "dragonborn.esm"
            | "_resourcepack.esl"
    ) || (n.starts_with("cc") && (n.ends_with(".esm") || n.ends_with(".esl")))
}

/// The launcher's Plugins.txt contract: `data/plugin-order.json` `plugins` array.
/// The surrounding object is a JS literal (template strings, trailing commas),
/// so only the array is parsed.
pub fn plugin_order_from_text(text: &str) -> Option<Vec<String>> {
    let json = extract_module_json(text, "data/plugin-order.json")?;
    let json = strip_trailing_commas(&json);
    let key = json.find("\"plugins\"")?;
    let start = json[key..].find('[')? + key;
    let slice = balanced_brackets(&json, start)?;
    let arr: Vec<Value> = serde_json::from_str(slice).ok()?;
    let names: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

fn balanced_brackets(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'[') {
        return None;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return std::str::from_utf8(&bytes[start..=i]).ok();
                }
            }
            _ => {}
        }
    }
    None
}

/// JS object literals allow a comma before `}` or `]`. serde_json does not.
fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            out.push(b as char);
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if b == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if b == b',' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                i += 1;
                continue;
            }
        }
        out.push(b as char);
        i += 1;
    }
    out
}

/// Plugins.txt expected set. Prefer the launcher's plugin-order module.
/// If that module is missing, use hash pins minus engine masters — those
/// masters are loaded by the game and are not written into Plugins.txt.
pub fn expected_load_order(asar_path: &Path, hash_plugins: &HashSet<String>) -> HashSet<String> {
    if let Ok(raw) = std::fs::read(asar_path) {
        let text = String::from_utf8_lossy(&raw);
        if let Some(order) = plugin_order_from_text(&text) {
            return order.into_iter().map(|n| n.to_ascii_lowercase()).collect();
        }
    }
    hash_plugins
        .iter()
        .filter(|n| !is_engine_master(n))
        .cloned()
        .collect()
}

/// Pull `modId` + `name` pairs out of `data/modlist.json` without trusting its shape.
pub fn names_from_modlist(value: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    collect_names(value, &mut out);
    out
}

fn mod_id_of(map: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["modId", "mod_id", "id"] {
        let Some(v) = map.get(key) else { continue };
        if let Some(n) = v.as_u64() {
            return Some(n.to_string());
        }
        if let Some(n) = v.as_i64() {
            if n >= 0 {
                return Some(n.to_string());
            }
        }
        if let Some(s) = v.as_str() {
            let s = s.trim();
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn collect_names(value: &Value, out: &mut BTreeMap<String, String>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_names(item, out);
            }
        }
        Value::Object(map) => {
            if let (Some(id), Some(name)) =
                (mod_id_of(map), map.get("name").and_then(|v| v.as_str()))
            {
                let name = name.trim();
                if !name.is_empty() {
                    out.entry(id).or_insert_with(|| name.to_string());
                }
            }
            for (k, v) in map {
                if k.chars().all(|c| c.is_ascii_digit()) {
                    if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                        let name = name.trim();
                        if !name.is_empty() {
                            out.entry(k.clone()).or_insert_with(|| name.to_string());
                        }
                    }
                }
                collect_names(v, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn ci_child(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        e.file_name()
            .to_str()
            .filter(|n| n.eq_ignore_ascii_case(name))
            .map(|_| e.path())
    })
}

pub(crate) fn wine_prefix_from_asar(asar: &Path) -> Option<PathBuf> {
    let mut cur = asar.parent();
    while let Some(dir) = cur {
        if dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("drive_c"))
        {
            return dir.parent().map(|p| p.to_path_buf());
        }
        cur = dir.parent();
    }
    None
}

pub(crate) fn steam_library_root(game: &Path) -> Option<PathBuf> {
    let mut cur = Some(game);
    while let Some(dir) = cur {
        if dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("common"))
        {
            let steamapps = dir.parent()?;
            if steamapps
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
            {
                return steamapps.parent().map(|p| p.to_path_buf());
            }
        }
        cur = dir.parent();
    }
    None
}

fn plugins_in_prefix(prefix: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(drive) = ci_child(prefix, "drive_c") else {
        return out;
    };
    let Some(users) = ci_child(&drive, "users") else {
        return out;
    };
    let Ok(entries) = std::fs::read_dir(users) else {
        return out;
    };
    for user in entries.flatten() {
        if !user.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let mut cur = user.path();
        let mut ok = true;
        for part in ["AppData", "Local", "Skyrim Special Edition", "Plugins.txt"] {
            match ci_child(&cur, part) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && cur.is_file() {
            out.push(cur);
        }
    }
    out
}

fn path_has_user(path: &Path, user: &str) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(user))
    })
}

/// Locate Plugins.txt. Never searches the home directory — only the launcher
/// prefix implied by the asar, and Steam compatdata next to the game.
pub fn find_plugins_txt(skyrim_root: &Path, asar_path: &Path) -> Option<PathBuf> {
    let mut cands = Vec::new();
    if let Some(prefix) = wine_prefix_from_asar(asar_path) {
        cands.extend(plugins_in_prefix(&prefix));
    }
    if let Some(library) = steam_library_root(skyrim_root) {
        if let Some(compat) =
            ci_child(&library, "steamapps").and_then(|s| ci_child(&s, "compatdata"))
        {
            if let Ok(entries) = std::fs::read_dir(compat) {
                for app in entries.flatten() {
                    if let Some(pfx) = ci_child(&app.path(), "pfx") {
                        cands.extend(plugins_in_prefix(&pfx));
                    }
                }
            }
        }
    }
    if let Some(local) = ci_child(skyrim_root, "Data").and_then(|d| ci_child(&d, "Plugins.txt")) {
        cands.push(local);
    }
    cands
        .iter()
        .find(|p| path_has_user(p, "steamuser"))
        .cloned()
        .or_else(|| cands.into_iter().next())
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarantineFile {
    pub name: String,
    /// Data-relative path when the manifest recorded one, otherwise the basename.
    pub from: String,
    pub in_contract: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarantineBatch {
    pub at: String,
    pub files: Vec<QuarantineFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QuarantineReport {
    pub found: bool,
    pub batches: Vec<QuarantineBatch>,
    pub foreign: usize,
    /// Contract plugins the launcher moved out of Data. This is the only quarantine failure.
    pub contract_hits: usize,
    pub failed: usize,
    pub note: String,
}

fn data_relative(from: &str) -> String {
    let norm = from.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    if let Some(i) = lower.rfind("/data/") {
        let rel = norm[i + "/data/".len()..].trim_start_matches('/');
        if !rel.is_empty() {
            return rel.to_string();
        }
    }
    norm.rsplit('/').next().unwrap_or(&norm).to_string()
}

fn basename_of(path: &str) -> String {
    path.replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_string()
}

#[derive(serde::Deserialize)]
struct QuarantineManifest {
    #[serde(default)]
    files: Vec<QuarantineManifestFile>,
}

#[derive(serde::Deserialize)]
struct QuarantineManifestFile {
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
}

/// Read every quarantine batch. `contract_plugins` is lowercased basenames.
pub fn read_quarantine(skyrim_root: &Path, contract_plugins: &HashSet<String>) -> QuarantineReport {
    let root = skyrim_root.join("DaedricData").join("quarantine");
    if !root.is_dir() {
        return QuarantineReport {
            found: false,
            batches: Vec::new(),
            foreign: 0,
            contract_hits: 0,
            failed: 0,
            note: "no quarantine directory — the launcher has not pulled anything".into(),
        };
    }

    let mut stamps: Vec<String> = std::fs::read_dir(&root)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    stamps.sort();
    stamps.reverse();

    let mut batches = Vec::new();
    let mut foreign = 0usize;
    let mut contract_hits = 0usize;

    for stamp in stamps {
        let dir = root.join(&stamp);
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        if let Ok(text) = std::fs::read_to_string(dir.join("quarantine-manifest.json")) {
            if let Ok(manifest) = serde_json::from_str::<QuarantineManifest>(&text) {
                for entry in manifest.files {
                    let src = if !entry.from.is_empty() {
                        entry.from
                    } else {
                        entry.to
                    };
                    if src.is_empty() {
                        continue;
                    }
                    let name = basename_of(&src);
                    if name.is_empty() || name.eq_ignore_ascii_case("quarantine-manifest.json") {
                        continue;
                    }
                    let key = name.to_ascii_lowercase();
                    if !seen.insert(key.clone()) {
                        continue;
                    }
                    let in_contract = contract_plugins.contains(&key);
                    if in_contract {
                        contract_hits += 1;
                    } else {
                        foreign += 1;
                    }
                    files.push(QuarantineFile {
                        name,
                        from: data_relative(&src),
                        in_contract,
                    });
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.eq_ignore_ascii_case("quarantine-manifest.json") || !is_plugin_name(&name) {
                    continue;
                }
                let key = name.to_ascii_lowercase();
                if !seen.insert(key.clone()) {
                    continue;
                }
                let in_contract = contract_plugins.contains(&key);
                if in_contract {
                    contract_hits += 1;
                } else {
                    foreign += 1;
                }
                files.push(QuarantineFile {
                    name,
                    from: String::new(),
                    in_contract,
                });
            }
        }
        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        if !files.is_empty() {
            batches.push(QuarantineBatch { at: stamp, files });
        }
    }

    let note = if contract_hits > 0 {
        format!(
            "launcher quarantined {contract_hits} contract plugin(s) — that is a bad pull, not a foreign file"
        )
    } else if foreign == 0 {
        "quarantine directory is empty".into()
    } else {
        format!("pulled {foreign} foreign plugin(s). none were in the contract")
    };

    QuarantineReport {
        found: true,
        batches,
        foreign,
        contract_hits,
        failed: contract_hits,
        note,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadOrderIssue {
    pub name: String,
    /// `missing` | `disabled` | `foreign` | `stale`
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadOrderReport {
    pub found: bool,
    pub path: Option<PathBuf>,
    pub enabled: usize,
    pub disabled: usize,
    pub contract_ok: usize,
    pub issues: Vec<LoadOrderIssue>,
    pub failed: usize,
    pub note: String,
}

struct LoadLine {
    name: String,
    enabled: bool,
}

fn parse_plugins_txt(text: &str) -> Vec<LoadLine> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches('\r');
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let enabled = line.starts_with('*');
            let name = if enabled { &line[1..] } else { line }.trim();
            if name.is_empty() {
                return None;
            }
            Some(LoadLine {
                name: name.to_string(),
                enabled,
            })
        })
        .collect()
}

fn data_plugin_names(data: &Path) -> HashSet<String> {
    let mut set = HashSet::new();
    let Ok(entries) = std::fs::read_dir(data) else {
        return set;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if is_plugin_name(&name) {
            set.insert(name.to_ascii_lowercase());
        }
    }
    set
}

fn quarantined_names(skyrim_root: &Path) -> HashSet<String> {
    let report = read_quarantine(skyrim_root, &HashSet::new());
    report
        .batches
        .into_iter()
        .flat_map(|b| b.files)
        .map(|f| f.name.to_ascii_lowercase())
        .collect()
}

/// Compare Plugins.txt to the contract. Vanilla masters are ignored.
/// A missing or disabled contract plugin fails. An enabled extra still sitting
/// in Data fails (the launcher should have quarantined it). An enabled extra
/// whose file is already in quarantine is a stale load-order line and also fails.
/// An enabled extra that is on neither disk nor quarantine is listed as stale
/// but does not fail — a leftover line cannot load.
pub fn read_load_order(
    skyrim_root: &Path,
    asar_path: &Path,
    contract_plugins: &HashSet<String>,
) -> LoadOrderReport {
    let Some(path) = find_plugins_txt(skyrim_root, asar_path) else {
        return LoadOrderReport {
            found: false,
            path: None,
            enabled: 0,
            disabled: 0,
            contract_ok: 0,
            issues: Vec::new(),
            failed: 0,
            note: "Plugins.txt not found — looked in the launcher prefix and Steam compatdata"
                .into(),
        };
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return LoadOrderReport {
            found: false,
            path: Some(path),
            enabled: 0,
            disabled: 0,
            contract_ok: 0,
            issues: Vec::new(),
            failed: 0,
            note: "Plugins.txt found but unreadable".into(),
        };
    };

    let lines = parse_plugins_txt(&text);
    let on_disk = data_plugin_names(&skyrim_root.join("Data"));
    let quarantined = quarantined_names(skyrim_root);
    let vanilla: HashSet<&str> = VANILLA.iter().copied().collect();

    let mut enabled = 0usize;
    let mut disabled = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    let mut issues = Vec::new();

    for line in &lines {
        let key = line.name.to_ascii_lowercase();
        seen.insert(key.clone());
        if line.enabled {
            enabled += 1;
        } else {
            disabled += 1;
        }
        if contract_plugins.contains(&key) {
            if !line.enabled {
                issues.push(LoadOrderIssue {
                    name: line.name.clone(),
                    kind: "disabled".into(),
                });
            }
            continue;
        }
        if vanilla.contains(key.as_str()) || !line.enabled {
            continue;
        }
        if on_disk.contains(&key) {
            issues.push(LoadOrderIssue {
                name: line.name.clone(),
                kind: "foreign".into(),
            });
        } else if quarantined.contains(&key) {
            issues.push(LoadOrderIssue {
                name: line.name.clone(),
                kind: "stale".into(),
            });
        } else {
            issues.push(LoadOrderIssue {
                name: line.name.clone(),
                kind: "orphan".into(),
            });
        }
    }

    let mut contract_ok = 0usize;
    for plugin in contract_plugins {
        if seen.contains(plugin) {
            let enabled_line = lines
                .iter()
                .any(|l| l.enabled && l.name.eq_ignore_ascii_case(plugin));
            if enabled_line {
                contract_ok += 1;
            }
        } else {
            issues.push(LoadOrderIssue {
                name: plugin.clone(),
                kind: "missing".into(),
            });
        }
    }
    issues.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.name.cmp(&b.name)));
    let failed = issues.iter().filter(|i| i.kind != "orphan").count();
    let note = if failed == 0 {
        format!("{contract_ok} contract plugins enabled, no load-order problems")
    } else {
        format!("{failed} load-order problem(s) — missing, disabled, foreign, or stale")
    };

    LoadOrderReport {
        found: true,
        path: Some(path),
        enabled,
        disabled,
        contract_ok,
        issues,
        failed,
        note,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerMod {
    pub mod_id: String,
    pub name: String,
    pub at: Option<u64>,
    pub file_count: u64,
    pub plugin_count: usize,
    pub plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LedgerReport {
    pub found: bool,
    pub mods: Vec<LedgerMod>,
    pub recorded: usize,
    pub named: usize,
    pub note: String,
}

#[derive(serde::Deserialize)]
struct LedgerRaw {
    #[serde(default, deserialize_with = "crate::de::opt_u64_lenient")]
    at: Option<u64>,
    #[serde(default, deserialize_with = "crate::de::opt_u64_lenient")]
    files: Option<u64>,
    #[serde(default)]
    plugins: Vec<Value>,
    #[serde(default)]
    manifest: Vec<Value>,
}

fn plugin_label(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Object(map) => ["name", "file", "path"]
            .iter()
            .find_map(|k| map.get(*k).and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

/// Read the launcher install ledger. A short ledger is not a failure — the
/// file records what the launcher unpacked, not the collection.
pub fn read_install_ledger(skyrim_root: &Path, names: &BTreeMap<String, String>) -> LedgerReport {
    let path = skyrim_root.join("DaedricData").join("installed-mods.json");
    if !path.is_file() {
        return LedgerReport {
            found: false,
            mods: Vec::new(),
            recorded: 0,
            named: 0,
            note: "no install ledger — DaedricData/installed-mods.json is absent".into(),
        };
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return LedgerReport {
            found: true,
            mods: Vec::new(),
            recorded: 0,
            named: 0,
            note: "install ledger is present but unreadable".into(),
        };
    };
    let parsed: Result<BTreeMap<String, LedgerRaw>, _> = serde_json::from_str(&text);
    let Ok(map) = parsed else {
        return LedgerReport {
            found: true,
            mods: Vec::new(),
            recorded: 0,
            named: 0,
            note: "install ledger is present but not a mod-id map".into(),
        };
    };

    let mut mods: Vec<LedgerMod> = map
        .into_iter()
        .map(|(id, raw)| {
            let plugins: Vec<String> = raw.plugins.iter().filter_map(plugin_label).collect();
            let file_count = raw.files.unwrap_or(raw.manifest.len() as u64);
            let name = names.get(&id).cloned().unwrap_or_default();
            LedgerMod {
                mod_id: id,
                name,
                at: raw.at,
                file_count,
                plugin_count: plugins.len(),
                plugins,
            }
        })
        .collect();
    mods.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.mod_id.cmp(&b.mod_id))
    });
    let recorded = mods.len();
    let named = mods.iter().filter(|m| !m.name.is_empty()).count();
    let note = format!(
        "launcher recorded {recorded} mod(s), {named} named from the contract. this is not the full collection"
    );
    LedgerReport {
        found: true,
        mods,
        recorded,
        named,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "daedric-field-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn quarantine_marks_contract_hits_and_strips_wine_paths() {
        let root = scratch("quar");
        let batch = root
            .join("DaedricData")
            .join("quarantine")
            .join("2026-09-21T04-09-18-951Z");
        fs::create_dir_all(&batch).unwrap();
        fs::write(
            batch.join("quarantine-manifest.json"),
            r#"{
              "quarantinedAt": "2026-09-21T04-09-18-951Z",
              "files": [
                {"from": "Z:\\\\home\\\\someone\\\\Data\\\\FNIS.esp", "to": "Z:\\\\q\\\\FNIS.esp"},
                {"from": "Z:\\\\home\\\\someone\\\\Data\\\\RaceMenu.esp", "to": "Z:\\\\q\\\\RaceMenu.esp"}
              ]
            }"#,
        )
        .unwrap();
        let mut contract = HashSet::new();
        contract.insert("racemenu.esp".into());
        let report = read_quarantine(&root, &contract);
        assert!(report.found);
        assert_eq!(report.foreign, 1);
        assert_eq!(report.contract_hits, 1);
        assert_eq!(report.failed, 1);
        let hit = report.batches[0]
            .files
            .iter()
            .find(|f| f.in_contract)
            .unwrap();
        assert_eq!(hit.name, "RaceMenu.esp");
        assert_eq!(hit.from, "RaceMenu.esp");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn load_order_classifies_missing_disabled_foreign_and_ignores_vanilla() {
        let root = scratch("load");
        let prefix = root.join("prefix");
        let asar = prefix
            .join("drive_c")
            .join("Program Files")
            .join("DaedricOnline")
            .join("resources")
            .join("app.asar");
        fs::create_dir_all(asar.parent().unwrap()).unwrap();
        fs::write(&asar, b"x").unwrap();
        let plugins = prefix
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("AppData")
            .join("Local")
            .join("Skyrim Special Edition");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(
            plugins.join("Plugins.txt"),
            "# comment\n*Skyrim.esm\n*Good.esp\nDisabled.esp\n*Foreign.esp\n*Pulled.esp\n",
        )
        .unwrap();
        let data = root.join("game").join("Data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("Good.esp"), b"g").unwrap();
        fs::write(data.join("Foreign.esp"), b"f").unwrap();
        let q = root
            .join("game")
            .join("DaedricData")
            .join("quarantine")
            .join("stamp");
        fs::create_dir_all(&q).unwrap();
        fs::write(q.join("Pulled.esp"), b"p").unwrap();

        let mut contract = HashSet::new();
        contract.insert("good.esp".into());
        contract.insert("disabled.esp".into());
        contract.insert("absent.esp".into());
        let report = read_load_order(&root.join("game"), &asar, &contract);
        assert!(report.found);
        assert_eq!(report.contract_ok, 1);
        let kinds: Vec<_> = report.issues.iter().map(|i| i.kind.as_str()).collect();
        assert!(kinds.contains(&"disabled"));
        assert!(kinds.contains(&"missing"));
        assert!(kinds.contains(&"foreign"));
        assert!(kinds.contains(&"stale"));
        assert!(!report
            .issues
            .iter()
            .any(|i| i.name.eq_ignore_ascii_case("Skyrim.esm")));
        assert_eq!(report.failed, 4);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ledger_accepts_mixed_number_types_and_joins_names() {
        let root = scratch("ledger");
        fs::create_dir_all(root.join("DaedricData")).unwrap();
        fs::write(
            root.join("DaedricData").join("installed-mods.json"),
            r#"{
              "19080": {
                "at": 1789963553023.0,
                "files": "18",
                "plugins": [],
                "manifest": [{"path": "interface/racemenu/bottombar.swf", "size": 14745}],
                "v": 2
              },
              "9": {"at": 1, "files": 0, "plugins": ["Foo.esp"], "manifest": []}
            }"#,
        )
        .unwrap();
        let mut names = BTreeMap::new();
        names.insert("19080".into(), "RaceMenu".into());
        let report = read_install_ledger(&root, &names);
        assert!(report.found);
        assert_eq!(report.recorded, 2);
        assert_eq!(report.named, 1);
        let race = report.mods.iter().find(|m| m.mod_id == "19080").unwrap();
        assert_eq!(race.name, "RaceMenu");
        assert_eq!(race.at, Some(1789963553023));
        assert_eq!(race.file_count, 18);
        let unnamed = report.mods.iter().find(|m| m.mod_id == "9").unwrap();
        assert!(unnamed.name.is_empty());
        assert_eq!(unnamed.plugins, vec!["Foo.esp".to_string()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn modlist_names_tolerate_either_shape() {
        let v = serde_json::json!({
            "mods": [{"modId": 19080, "name": "RaceMenu"}, {"modId": "42", "name": "SkyUI"}]
        });
        let names = names_from_modlist(&v);
        assert_eq!(names.get("19080").map(String::as_str), Some("RaceMenu"));
        assert_eq!(names.get("42").map(String::as_str), Some("SkyUI"));
    }

    #[test]
    fn plugin_order_module_is_the_load_order_contract() {
        let js = r#"// data/plugin-order.json
var require_order = __commonJS({
  "data/plugin-order.json"(exports2, module2) {
    module2.exports = {
      plugins: ["RP_Nexus.esp", "Skyrim.esm"],
    };
  }
});"#;
        let order = plugin_order_from_text(js).unwrap();
        assert_eq!(
            order,
            vec!["RP_Nexus.esp".to_string(), "Skyrim.esm".to_string()]
        );
    }
}
