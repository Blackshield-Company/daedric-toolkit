//! daedric-core — the engine under the Daedric Toolkit.
//!
//! Born from field forensics (see daedric-online-linux/docs/the-mod-verification-gate.md):
//! the Daedric Online launcher bundles its mod-verification contract inside its Electron
//! `app.asar` — required mod list, canonical archive checksums, and a per-file sha256 map.
//! This crate decodes that contract and checks a real install against it.

pub mod asar;
pub mod collection;
mod de;
pub mod extract;
pub mod field;
pub mod hash;
pub mod hazards;
pub mod launch;
pub mod manifest;
pub mod overlay;
pub mod play;
pub mod scan;
pub mod troubleshoot;
pub mod verify;

pub use asar::Asar;
pub use collection::{BuildManifest, CollectionLock, DllAllowlist};
pub use field::{
    expected_load_order, names_from_modlist, plugin_set, read_install_ledger, read_load_order,
    read_quarantine, LedgerReport, LoadOrderReport, QuarantineReport,
};
pub use hazards::{read_hazards, HazardReport};
pub use launch::{read_launch, read_update_guard, LaunchReport, UpdateGuardReport};
pub use manifest::Manifest;
pub use overlay::{check_overlay, read_installed_overlay, OverlayReport, OverlayVerdict};
pub use play::{
    read_pe_file_version, read_play_elsewhere, read_release_book, read_runtime, ElsewhereReport,
    ReleaseReport, RuntimeReport,
};
pub use scan::{scan_install, FileVerdict, ScanReport};
pub use troubleshoot::{read_troubleshoot, TroubleshootReport};
pub use verify::{scan_build_gate, verify_downloads, BuildGateReport, DownloadsReport};
