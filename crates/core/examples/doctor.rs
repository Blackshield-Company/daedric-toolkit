//! Full install-doctor run against a live Skyrim SE + Daedric Online install.
//!
//! usage: doctor [asar_path] [skyrim_root]
//! or set DAEDRIC_ASAR and DAEDRIC_SKYRIM.

use daedric_core::{
    scan_build_gate, scan_install, verify_downloads, BuildManifest, CollectionLock, DllAllowlist,
    Manifest,
};
use std::path::PathBuf;
use std::time::Instant;

fn path_arg(nth: usize, env_key: &str, usage: &str) -> PathBuf {
    std::env::args()
        .nth(nth)
        .or_else(|| std::env::var(env_key).ok())
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            eprintln!("{usage}");
            std::process::exit(2);
        })
}

fn main() {
    let asar = path_arg(
        1,
        "DAEDRIC_ASAR",
        "usage: doctor [asar_path] [skyrim_root]  (or DAEDRIC_ASAR / DAEDRIC_SKYRIM)",
    );
    let root = path_arg(
        2,
        "DAEDRIC_SKYRIM",
        "usage: doctor [asar_path] [skyrim_root]  (or DAEDRIC_ASAR / DAEDRIC_SKYRIM)",
    );
    let data = root.join("Data");

    println!("== decode contracts ==");
    let lock = CollectionLock::from_asar(&asar).expect("collection-lock decode");
    let build = BuildManifest::from_asar(&asar).expect("build-manifest decode");
    let dlls = DllAllowlist::from_asar(&asar).expect("dll allowlist decode");
    let manifest = Manifest {
        files: lock.files.clone(),
    };
    println!(
        "  collection-lock: {} mods / {} file pins | build-manifest: {} plugins + {} loose | dlls: {}",
        lock.mods.len(),
        lock.files.len(),
        build.plugins.len(),
        build.loose_files.len(),
        dlls.dlls.len()
    );

    println!("\n== overlay drift ==");
    let overlay = daedric_core::check_overlay(&root, &dlls);
    println!(
        "  installed: {} ({}marker) | accepted: [{}] | verdict: {:?}{}",
        overlay.installed.as_deref().unwrap_or("none"),
        if overlay.legacy_marker { "legacy " } else { "" },
        overlay.accepted.join(", "),
        overlay.verdict,
        if overlay.behind_latest {
            " — BEHIND latest, launcher will push an update at next Play"
        } else {
            ""
        }
    );
    if matches!(overlay.verdict, daedric_core::OverlayVerdict::Drift) {
        println!("  note: gate failures below are version drift, not corruption.");
    }

    println!("\n== esp gate (collection-lock files:) ==");
    let t = Instant::now();
    let report = scan_install(&data, &manifest).expect("esp scan");
    println!(
        "  {} ok / {} failed in {:?} — gate {}",
        report.ok,
        report.failed,
        t.elapsed(),
        if report.passes_gate() {
            "PASSES"
        } else {
            "FAILS"
        }
    );
    for f in report
        .files
        .iter()
        .filter(|f| f.path.is_none() || !matches!(f.verdict, daedric_core::FileVerdict::Ok))
    {
        println!("    FAIL {:?} {}", f.verdict, f.esp);
    }

    println!("\n== build gate (plugins + loose + SKSE dlls) ==");
    let t = Instant::now();
    let gate = scan_build_gate(&data, &build, &dlls).expect("build gate scan");
    println!(
        "  {} ok / {} failed in {:?} — gate {}",
        gate.ok,
        gate.failed,
        t.elapsed(),
        if gate.passes_gate() {
            "PASSES"
        } else {
            "FAILS"
        }
    );
    for f in gate
        .files
        .iter()
        .take(20)
        .filter(|f| !matches!(f.verdict, daedric_core::verify::GateVerdict::Ok))
    {
        println!("    FAIL [{}] {:?} {}", f.kind, f.verdict, f.file);
    }

    println!("\n== downloads (archive md5 pins) ==");
    let dl = root.join("DaedricData").join("downloads");
    match verify_downloads(&dl, &lock) {
        Ok(rep) => {
            println!(
                "  {} ok / {} failed in {}",
                rep.ok,
                rep.failed,
                dl.display()
            );
            for a in rep.archives.iter().take(10).filter(|a| {
                !matches!(
                    a.verdict,
                    daedric_core::verify::ArchiveVerdict::Ok
                        | daedric_core::verify::ArchiveVerdict::Unpinned
                )
            }) {
                println!(
                    "    {:?} {} ({:?}/{:?})",
                    a.verdict, a.name, a.mod_id, a.file_id
                );
            }
        }
        Err(e) => println!("  skipped: {e}"),
    }

    println!("\n== quarantine / load order / ledger ==");
    let plugins = daedric_core::plugin_set(&lock, &build);
    let order_plugins = daedric_core::expected_load_order(&asar, &plugins);
    let quarantine = daedric_core::read_quarantine(&root, &plugins);
    println!(
        "  quarantine: {} foreign / {} contract hits — {}",
        quarantine.foreign, quarantine.contract_hits, quarantine.note
    );
    for batch in &quarantine.batches {
        for file in &batch.files {
            println!(
                "    {} {}{}",
                batch.at,
                file.name,
                if file.in_contract { " CONTRACT" } else { "" }
            );
        }
    }
    let order = daedric_core::read_load_order(&root, &asar, &order_plugins);
    println!(
        "  load order: found={} contract_ok={} failed={} — {}",
        order.found, order.contract_ok, order.failed, order.note
    );
    for issue in order.issues.iter().take(20) {
        println!("    {} {}", issue.kind, issue.name);
    }
    let mut names = std::collections::BTreeMap::new();
    for pin in &lock.mods {
        if let Some(id) = pin.mod_id {
            if !pin.name.trim().is_empty() {
                names.insert(id.to_string(), pin.name.clone());
            }
        }
    }
    let ledger = daedric_core::read_install_ledger(&root, &names);
    println!(
        "  ledger: recorded={} named={} — {}",
        ledger.recorded, ledger.named, ledger.note
    );

    println!("\n== play blockers ==");
    let runtime = daedric_core::read_runtime(&root, &lock.game_versions);
    println!(
        "  runtime: {} installed={:?} required={} loader={} failed={}",
        runtime.verdict,
        runtime.installed,
        runtime.required,
        runtime.loader_present,
        runtime.failed
    );
    println!("    {}", runtime.note);
    let release = daedric_core::read_release_book(&root);
    println!(
        "  release: {} servers={:?} failed={}",
        release.verdict, release.servers, release.failed
    );
    println!("    {}", release.note);
    let elsewhere = daedric_core::read_play_elsewhere(&root);
    println!(
        "  elsewhere: disabled={} parked={} failed={}",
        elsewhere.disabled, elsewhere.parked, elsewhere.failed
    );
    println!("    {}", elsewhere.note);
    let hazards = daedric_core::read_hazards(&root);
    println!(
        "  leftovers: known-bad={} foreign={} parked={} failed={}",
        hazards.known_bad.len(),
        hazards.foreign_plugins.len(),
        hazards.parked_folders.len(),
        hazards.failed
    );
    println!("    {}", hazards.note);
    for issue in hazards.issues.iter().take(12) {
        println!("    {} {}", issue.kind, issue.name);
    }

    println!("\n== launch / update guard / troubleshooting ==");
    let launch = daedric_core::read_launch(&root, &asar, None);
    println!(
        "  launch: method={} prefix={:?} proton={:?} failed={}",
        launch.method, launch.prefix, launch.proton, launch.failed
    );
    println!("    {}", launch.note);
    for line in &launch.recipe {
        println!("    | {line}");
    }
    for issue in &launch.issues {
        println!("    {} {}", issue.kind, issue.detail);
    }
    let guard = daedric_core::read_update_guard(&root, &asar);
    println!(
        "  update guard: manifest={:?} auto_update={:?} locked={:?} failed={}",
        guard.manifest, guard.auto_update, guard.locked, guard.failed
    );
    println!("    {}", guard.note);
    let fix = daedric_core::read_troubleshoot(&root, &asar);
    println!(
        "  troubleshooting: bsas={} case_misses={} behavior={:?}/{} display_tweaks={} failed={}",
        fix.textures.len(),
        fix.case_misses,
        fix.behavior_engine,
        fix.behavior_generated,
        fix.display_tweaks,
        fix.failed
    );
    println!("    {}", fix.note);
    for issue in fix.issues.iter().take(12) {
        println!("    {} {}", issue.kind, issue.detail);
    }
}
