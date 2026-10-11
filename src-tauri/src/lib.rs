use daedric_core::{
    expected_load_order, names_from_modlist, plugin_set, read_hazards, read_install_ledger,
    read_launch, read_load_order, read_play_elsewhere, read_quarantine, read_release_book,
    read_runtime, read_troubleshoot, read_update_guard, Asar, BuildGateReport, BuildManifest,
    CollectionLock, DllAllowlist, DownloadsReport, ElsewhereReport, HazardReport, LaunchReport,
    LedgerReport, LoadOrderReport, Manifest, OverlayReport, QuarantineReport, ReleaseReport,
    RuntimeReport, ScanReport, TroubleshootReport, UpdateGuardReport,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

/// Load the verification contract from a launcher app.asar.
#[tauri::command]
fn load_manifest(asar_path: String) -> Result<Manifest, String> {
    Manifest::from_asar(&PathBuf::from(asar_path)).map_err(|e| e.to_string())
}

/// Scan a Skyrim Data directory against the manifest.
#[tauri::command]
fn scan_data_dir(data_dir: String, asar_path: String) -> Result<ScanReport, String> {
    let manifest = Manifest::from_asar(&PathBuf::from(asar_path)).map_err(|e| e.to_string())?;
    daedric_core::scan_install(&PathBuf::from(data_dir), &manifest).map_err(|e| e.to_string())
}

/// Decoded-contract summary: the counts and identity fields from all three
/// data modules embedded in the launcher's asar.
#[derive(Debug, Clone, Serialize)]
pub struct ContractSummary {
    pub mods: usize,
    pub file_pins: usize,
    pub plugins: usize,
    pub loose_files: usize,
    pub dlls: usize,
    pub slug: Option<String>,
    pub revision: Option<u64>,
    pub file_set_sha256: Option<String>,
}

fn summarize(lock: &CollectionLock, build: &BuildManifest, dlls: &DllAllowlist) -> ContractSummary {
    ContractSummary {
        mods: lock.mods.len(),
        file_pins: lock.files.len(),
        plugins: build.plugins.len(),
        loose_files: build.loose_files.len(),
        dlls: dlls.dlls.len(),
        slug: lock.slug.clone(),
        revision: lock.revision,
        file_set_sha256: lock.file_set_sha256.clone(),
    }
}

/// Decode all three embedded contracts and return just the summary strip.
#[tauri::command]
fn decode_contracts(asar_path: String) -> Result<ContractSummary, String> {
    let asar = PathBuf::from(asar_path);
    let lock = CollectionLock::from_asar(&asar).map_err(|e| e.to_string())?;
    let build = BuildManifest::from_asar(&asar).map_err(|e| e.to_string())?;
    let dlls = DllAllowlist::from_asar(&asar).map_err(|e| e.to_string())?;
    Ok(summarize(&lock, &build, &dlls))
}

/// The esp gate: every pinned plugin checked against the Data dir.
#[tauri::command]
fn scan_esp_gate(data_dir: String, asar_path: String) -> Result<ScanReport, String> {
    let manifest = Manifest::from_asar(&PathBuf::from(asar_path)).map_err(|e| e.to_string())?;
    daedric_core::scan_install(&PathBuf::from(data_dir), &manifest).map_err(|e| e.to_string())
}

/// The Play-gate: build-manifest plugins + loose files + SKSE DLL allowlist.
#[tauri::command]
fn scan_build_gate(data_dir: String, asar_path: String) -> Result<BuildGateReport, String> {
    let asar = PathBuf::from(asar_path);
    let build = BuildManifest::from_asar(&asar).map_err(|e| e.to_string())?;
    let dlls = DllAllowlist::from_asar(&asar).map_err(|e| e.to_string())?;
    daedric_core::scan_build_gate(&PathBuf::from(data_dir), &build, &dlls)
        .map_err(|e| e.to_string())
}

/// Verify downloaded mod archives (`<skyrim_root>/DaedricData/downloads`).
#[tauri::command]
fn verify_downloads(skyrim_root: String, asar_path: String) -> Result<DownloadsReport, String> {
    let lock = CollectionLock::from_asar(&PathBuf::from(asar_path)).map_err(|e| e.to_string())?;
    let downloads = PathBuf::from(skyrim_root)
        .join("DaedricData")
        .join("downloads");
    daedric_core::verify_downloads(&downloads, &lock).map_err(|e| e.to_string())
}

/// Overlay drift check: installed overlay marker vs the contract's accepted set.
/// Drift means "the launcher will push an update at next Play" — gate failures
/// on a drifting install are version skew, not corruption.
#[tauri::command]
fn check_overlay_drift(skyrim_root: String, asar_path: String) -> Result<OverlayReport, String> {
    let dlls = DllAllowlist::from_asar(&PathBuf::from(asar_path)).map_err(|e| e.to_string())?;
    Ok(daedric_core::check_overlay(
        &PathBuf::from(skyrim_root),
        &dlls,
    ))
}

struct FieldContext {
    plugins: HashSet<String>,
    order: HashSet<String>,
    names: BTreeMap<String, String>,
}

fn field_context(asar_path: &str) -> Result<FieldContext, String> {
    let asar = PathBuf::from(asar_path);
    let lock = CollectionLock::from_asar(&asar).map_err(|e| e.to_string())?;
    let build = BuildManifest::from_asar(&asar).map_err(|e| e.to_string())?;
    let plugins = plugin_set(&lock, &build);
    let order = expected_load_order(&asar, &plugins);
    let mut names = BTreeMap::new();
    if let Ok(archive) = Asar::open(&asar) {
        if let Ok(value) = archive.read_json::<Value>("data/modlist.json") {
            names = names_from_modlist(&value);
        }
    }
    for pin in &lock.mods {
        if let Some(id) = pin.mod_id {
            if !pin.name.trim().is_empty() {
                names.insert(id.to_string(), pin.name.clone());
            }
        }
    }
    Ok(FieldContext {
        plugins,
        order,
        names,
    })
}

#[tauri::command(rename = "read_quarantine")]
fn read_quarantine_cmd(skyrim_root: String, asar_path: String) -> Result<QuarantineReport, String> {
    let ctx = field_context(&asar_path)?;
    Ok(read_quarantine(&PathBuf::from(skyrim_root), &ctx.plugins))
}

#[tauri::command(rename = "read_load_order")]
fn read_load_order_cmd(skyrim_root: String, asar_path: String) -> Result<LoadOrderReport, String> {
    let ctx = field_context(&asar_path)?;
    Ok(read_load_order(
        &PathBuf::from(&skyrim_root),
        &PathBuf::from(asar_path),
        &ctx.order,
    ))
}

#[tauri::command(rename = "read_install_ledger")]
fn read_install_ledger_cmd(skyrim_root: String, asar_path: String) -> Result<LedgerReport, String> {
    let ctx = field_context(&asar_path)?;
    Ok(read_install_ledger(&PathBuf::from(skyrim_root), &ctx.names))
}

#[tauri::command(rename = "read_runtime")]
fn read_runtime_cmd(skyrim_root: String, asar_path: String) -> Result<RuntimeReport, String> {
    let accepted = CollectionLock::from_asar(&PathBuf::from(asar_path))
        .map(|lock| lock.game_versions)
        .unwrap_or_default();
    Ok(read_runtime(&PathBuf::from(skyrim_root), &accepted))
}

#[tauri::command(rename = "read_release_book")]
fn read_release_book_cmd(skyrim_root: String) -> Result<ReleaseReport, String> {
    Ok(read_release_book(&PathBuf::from(skyrim_root)))
}

#[tauri::command(rename = "read_play_elsewhere")]
fn read_play_elsewhere_cmd(skyrim_root: String) -> Result<ElsewhereReport, String> {
    Ok(read_play_elsewhere(&PathBuf::from(skyrim_root)))
}

#[tauri::command(rename = "read_launch")]
fn read_launch_cmd(skyrim_root: String, asar_path: String) -> Result<LaunchReport, String> {
    Ok(read_launch(
        &PathBuf::from(skyrim_root),
        &PathBuf::from(asar_path),
        None,
    ))
}

#[tauri::command(rename = "read_update_guard")]
fn read_update_guard_cmd(
    skyrim_root: String,
    asar_path: String,
) -> Result<UpdateGuardReport, String> {
    Ok(read_update_guard(
        &PathBuf::from(skyrim_root),
        &PathBuf::from(asar_path),
    ))
}

#[tauri::command(rename = "read_troubleshoot")]
fn read_troubleshoot_cmd(
    skyrim_root: String,
    asar_path: String,
) -> Result<TroubleshootReport, String> {
    Ok(read_troubleshoot(
        &PathBuf::from(skyrim_root),
        &PathBuf::from(asar_path),
    ))
}

#[tauri::command(rename = "read_hazards")]
fn read_hazards_cmd(skyrim_root: String) -> Result<HazardReport, String> {
    Ok(read_hazards(&PathBuf::from(skyrim_root)))
}

/// Everything the doctor reports in one run.
#[derive(Debug, Clone, Serialize)]
pub struct FullDoctorReport {
    pub esp: ScanReport,
    pub gate: BuildGateReport,
    pub downloads: DownloadsReport,
    pub contracts: ContractSummary,
    pub overlay: OverlayReport,
    pub quarantine: QuarantineReport,
    pub load_order: LoadOrderReport,
    pub ledger: LedgerReport,
    pub runtime: RuntimeReport,
    pub release: ReleaseReport,
    pub elsewhere: ElsewhereReport,
    pub hazards: HazardReport,
    pub launch: LaunchReport,
    pub update_guard: UpdateGuardReport,
    pub troubleshoot: TroubleshootReport,
}

/// Full install-doctor sweep: decode contracts, scan the esp gate, the
/// build gate, and the downloads folder.
#[tauri::command]
fn run_full_doctor(skyrim_root: String, asar_path: String) -> Result<FullDoctorReport, String> {
    let asar = PathBuf::from(asar_path);
    let root = PathBuf::from(skyrim_root);
    let data = root.join("Data");

    let lock = CollectionLock::from_asar(&asar).map_err(|e| e.to_string())?;
    let build = BuildManifest::from_asar(&asar).map_err(|e| e.to_string())?;
    let dlls = DllAllowlist::from_asar(&asar).map_err(|e| e.to_string())?;

    let contracts = summarize(&lock, &build, &dlls);
    let overlay = daedric_core::check_overlay(&root, &dlls);

    let manifest = Manifest {
        files: lock.files.clone(),
    };
    let esp = daedric_core::scan_install(&data, &manifest).map_err(|e| e.to_string())?;
    let gate = daedric_core::scan_build_gate(&data, &build, &dlls).map_err(|e| e.to_string())?;
    let downloads =
        daedric_core::verify_downloads(&root.join("DaedricData").join("downloads"), &lock)
            .map_err(|e| e.to_string())?;
    let plugins = plugin_set(&lock, &build);
    let mut names = BTreeMap::new();
    if let Ok(archive) = Asar::open(&asar) {
        if let Ok(value) = archive.read_json::<Value>("data/modlist.json") {
            names = names_from_modlist(&value);
        }
    }
    for pin in &lock.mods {
        if let Some(id) = pin.mod_id {
            if !pin.name.trim().is_empty() {
                names.insert(id.to_string(), pin.name.clone());
            }
        }
    }
    let quarantine = read_quarantine(&root, &plugins);
    let load_order = read_load_order(&root, &asar, &expected_load_order(&asar, &plugins));
    let ledger = read_install_ledger(&root, &names);
    let runtime = read_runtime(&root, &lock.game_versions);
    let release = read_release_book(&root);
    let elsewhere = read_play_elsewhere(&root);
    let hazards = read_hazards(&root);
    let launch = read_launch(&root, &asar, None);
    let update_guard = read_update_guard(&root, &asar);
    let troubleshoot = read_troubleshoot(&root, &asar);

    Ok(FullDoctorReport {
        esp,
        gate,
        downloads,
        contracts,
        overlay,
        quarantine,
        load_order,
        ledger,
        runtime,
        release,
        elsewhere,
        hazards,
        launch,
        update_guard,
        troubleshoot,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_manifest,
            scan_data_dir,
            decode_contracts,
            scan_esp_gate,
            scan_build_gate,
            verify_downloads,
            check_overlay_drift,
            read_quarantine_cmd,
            read_load_order_cmd,
            read_install_ledger_cmd,
            read_runtime_cmd,
            read_release_book_cmd,
            read_play_elsewhere_cmd,
            read_hazards_cmd,
            read_launch_cmd,
            read_update_guard_cmd,
            read_troubleshoot_cmd,
            run_full_doctor,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Daedric Toolkit");
}
