/* Daedric Toolkit — install doctor frontend.
 * Talks to the Rust command surface via window.__TAURI__.core.invoke
 * (app.withGlobalTauri: true). No bundler, no keyboard-only flows.
 */

(function () {
  "use strict";

  var invoke = window.__TAURI__.core.invoke;

  var $ = function (id) {
    return document.getElementById(id);
  };

  var asarInput = $("asar-path");
  var rootInput = $("skyrim-root");
  var runBtn = $("run-doctor");
  var statusLine = $("status-line");

  function rememberPath(input, key) {
    var saved = localStorage.getItem(key);
    if (saved) input.value = saved;
    input.addEventListener("change", function () {
      localStorage.setItem(key, input.value.trim());
    });
  }
  rememberPath(asarInput, "daedric-asar");
  rememberPath(rootInput, "daedric-skyrim-root");

  function dataDir() {
    return rootInput.value.replace(/\/+$/, "") + "/Data";
  }

  // ---------- verdict decoding (serde externally-tagged enums) ----------

  function verdictName(v) {
    if (typeof v === "string") return v;
    return Object.keys(v)[0];
  }

  function isOkVerdict(v) {
    var n = verdictName(v);
    return n === "Ok" || n === "Unpinned";
  }

  function shortHash(h) {
    if (!h) return "—";
    return h.length > 16 ? h.slice(0, 12) + "…" : h;
  }

  function verdictDetail(v) {
    var n = verdictName(v);
    if (n === "Missing") return "not found on disk";
    if (n === "Unpinned") return "no md5 pin in contract";
    if (typeof v !== "object") return "";
    var p = v[n];
    if (n === "HashMismatch") {
      var s = "expected sha " + shortHash(p.expected) + " / got " + shortHash(p.actual);
      if (p.actual_size !== undefined) s += " (size " + p.actual_size + ")";
      return s;
    }
    if (n === "SizeMismatch") {
      return "expected " + p.expected + " bytes / got " + p.actual;
    }
    return "";
  }

  // ---------- rendering ----------

  function setBadge(id, state) {
    // state: "idle" | "pass" | "fail"
    var b = $(id);
    b.className = "badge badge-" + state;
    b.textContent = state.toUpperCase();
  }

  function renderPanel(prefix, report, entries, nameOf, kindOf, verdictOf) {
    $("ok-" + prefix).textContent = report.ok + " ok";
    $("fail-" + prefix).textContent = report.failed + " failed";
    setBadge("badge-" + prefix, report.failed === 0 ? "pass" : "fail");

    var list = $("list-" + prefix);
    list.textContent = "";
    var failures = entries.filter(function (e) {
      return !isOkVerdict(verdictOf(e));
    });

    if (failures.length === 0) {
      var li = document.createElement("li");
      li.className = "empty-note";
      li.textContent = report.failed === 0 ? "the gate holds — no failures" : "no failing entries";
      list.appendChild(li);
      return;
    }

    failures.forEach(function (e) {
      var li = document.createElement("li");
      var head = document.createElement("div");
      var name = document.createElement("span");
      name.className = "fail-name";
      name.textContent = nameOf(e);
      head.appendChild(name);
      var kind = kindOf(e);
      if (kind) {
        var k = document.createElement("span");
        k.className = "fail-kind";
        k.textContent = "[" + kind + "]";
        head.appendChild(k);
      }
      var vn = document.createElement("span");
      vn.className = "fail-verdict";
      vn.textContent = verdictName(verdictOf(e));
      head.appendChild(vn);
      li.appendChild(head);

      var detail = verdictDetail(verdictOf(e));
      var extra = e.canonical_entry ? " · fix from: " + e.canonical_entry : "";
      if (detail || extra) {
        var d = document.createElement("div");
        d.className = "fail-detail";
        d.textContent = detail + extra;
        li.appendChild(d);
      }
      list.appendChild(li);
    });
  }

  function renderEsp(report) {
    renderPanel(
      "esp",
      report,
      report.files,
      function (f) { return f.esp; },
      function () { return null; },
      function (f) { return f.verdict; }
    );
  }

  function renderGate(report) {
    renderPanel(
      "gate",
      report,
      report.files,
      function (f) { return f.file; },
      function (f) { return f.kind; },
      function (f) { return f.verdict; }
    );
  }

  function renderDownloads(report) {
    renderPanel(
      "dl",
      report,
      report.archives,
      function (a) { return a.name; },
      function (a) { return a.category || null; },
      function (a) { return a.verdict; }
    );
  }

  function fillList(id, rows, emptyText) {
    var list = $(id);
    list.textContent = "";
    if (!rows.length) {
      var li = document.createElement("li");
      li.className = "empty-note";
      li.textContent = emptyText;
      list.appendChild(li);
      return;
    }
    rows.forEach(function (row) {
      var li = document.createElement("li");
      var head = document.createElement("div");
      var name = document.createElement("span");
      name.className = "fail-name";
      name.textContent = row.name;
      head.appendChild(name);
      if (row.kind) {
        var k = document.createElement("span");
        k.className = "fail-kind";
        k.textContent = "[" + row.kind + "]";
        head.appendChild(k);
      }
      if (row.verdict) {
        var vn = document.createElement("span");
        vn.className = "fail-verdict";
        vn.textContent = row.verdict;
        head.appendChild(vn);
      }
      li.appendChild(head);
      if (row.detail) {
        var d = document.createElement("div");
        d.className = "fail-detail";
        d.textContent = row.detail;
        li.appendChild(d);
      }
      list.appendChild(li);
    });
  }

  function renderQuarantine(report) {
    $("ok-quar").textContent = report.foreign + " foreign";
    $("fail-quar").textContent = report.contract_hits + " contract hits";
    if (!report.found) setBadge("badge-quar", "idle");
    else setBadge("badge-quar", report.failed === 0 ? "pass" : "fail");
    var rows = [];
    (report.batches || []).forEach(function (batch) {
      (batch.files || []).forEach(function (file) {
        rows.push({
          name: file.name,
          kind: batch.at,
          verdict: file.in_contract ? "CONTRACT" : "FOREIGN",
          detail: file.from && file.from !== file.name ? file.from : "",
        });
      });
    });
    fillList("list-quar", rows, report.note || "nothing quarantined");
  }

  function renderLoad(report) {
    $("ok-load").textContent = report.contract_ok + " contract ok";
    $("fail-load").textContent = report.failed + " problems";
    if (!report.found) setBadge("badge-load", "idle");
    else setBadge("badge-load", report.failed === 0 ? "pass" : "fail");
    fillList(
      "list-load",
      (report.issues || []).map(function (issue) {
        return { name: issue.name, kind: issue.kind, verdict: "", detail: "" };
      }),
      report.note || "no load-order problems"
    );
  }

  function renderIssues(prefix, report, okText) {
    $( "ok-" + prefix ).textContent = okText;
    $( "fail-" + prefix ).textContent = report.failed + " problems";
    setBadge("badge-" + prefix, report.failed === 0 ? "pass" : "fail");
    fillList(
      "list-" + prefix,
      (report.issues || []).map(function (issue) {
        return { name: issue.detail, kind: issue.kind, verdict: "", detail: "" };
      }),
      report.note || "clear"
    );
  }

  function renderRuntime(report) {
    renderIssues("rt", report, report.installed || report.verdict || "—");
  }

  function renderRelease(report) {
    var label = report.servers && report.servers.length ? report.servers.join(", ") : report.verdict;
    renderIssues("rel", report, label || "—");
  }

  function renderElsewhere(report) {
    renderIssues("else", report, report.disabled ? "parked" : "enabled");
  }

  function renderHazards(report) {
    var bits = [
      (report.known_bad || []).length + " known-bad",
      (report.foreign_plugins || []).length + " foreign",
      (report.parked_folders || []).length + " parked",
    ];
    renderIssues("haz", report, bits.join(" · "));
  }

  function renderLaunch(report) {
    renderIssues("launch", report, report.method || "—");
    var lines = (report.recipe || []).slice();
    var umuFailed = (report.issues || []).some(function (i) { return i.kind === "umu-failed"; });
    if ((umuFailed || report.method === "steam-shortcut") && report.fallback && report.fallback.length) {
      lines.push("", "# fallback only — umu failed:");
      report.fallback.forEach(function (step, i) { lines.push("# " + (i + 1) + ". " + step); });
    }
    $("recipe-launch").textContent = lines.join("\n");
  }

  function renderGuard(report) {
    var label = report.locked === true ? "locked" : report.locked === false ? "unlocked" : "no manifest";
    if (report.auto_update) label += " · auto-update " + report.auto_update;
    renderIssues("guard", report, label);
  }

  function renderFix(report) {
    var bad = (report.textures || []).reduce(function (n, t) { return n + (t.bad || 0); }, 0);
    var bits = [
      (report.textures || []).length + " BSA · " + bad + " bad",
      report.case_misses + " case",
      report.display_tweaks ? "display tweaks" : "prefs",
    ];
    renderIssues("fix", report, bits.join(" · "));
  }

  function renderLedger(report) {
    $("ok-ledger").textContent = report.recorded + " recorded";
    $("fail-ledger").textContent = report.named + " named";
    setBadge("badge-ledger", report.found ? "pass" : "idle");
    fillList(
      "list-ledger",
      (report.mods || []).map(function (mod) {
        var bits = [mod.file_count + " files"];
        if (mod.plugin_count) bits.push(mod.plugin_count + " plugins");
        return {
          name: mod.name || ("mod " + mod.mod_id),
          kind: mod.mod_id,
          verdict: "",
          detail: bits.join(" · "),
        };
      }),
      report.note || "no ledger"
    );
  }

  function renderContracts(c) {
    $("contract-strip").classList.remove("hidden");
    $("c-mods").textContent = c.mods;
    $("c-pins").textContent = c.file_pins;
    $("c-plugins").textContent = c.plugins + " +" + c.loose_files + " loose";
    $("c-dlls").textContent = c.dlls;
    $("c-slug").textContent = c.slug || "—";
    $("c-rev").textContent = c.revision != null ? c.revision : "—";
    $("c-fileset").textContent = c.file_set_sha256
      ? c.file_set_sha256.slice(0, 24) + "…"
      : "—";
  }

  // Overlay verdict arrives as a unit-variant string ("Current", "Drift", …).
  function renderOverlay(o) {
    $("overlay-banner").classList.remove("hidden");
    var badge = $("overlay-badge");
    var note = "";
    if (o.verdict === "Current") {
      badge.className = "badge badge-pass";
      badge.textContent = "CURRENT";
      note = "install matches the contract's accepted overlay set.";
    } else if (o.verdict === "Drift") {
      badge.className = "badge badge-drift";
      badge.textContent = "DRIFT";
      note =
        "installed overlay is outside the accepted set — gate failures below " +
        "are version skew, not corruption. The launcher will push an update at next Play.";
    } else if (o.verdict === "NotInstalled") {
      badge.className = "badge badge-idle";
      badge.textContent = "NO OVERLAY";
      note = "no overlay marker found — fresh install or overlay never applied.";
    } else {
      badge.className = "badge badge-idle";
      badge.textContent = "UNKNOWN";
      note = "contract carries no accepted overlay set — nothing to compare against.";
    }
    $("overlay-installed").textContent =
      (o.installed || "—") + (o.legacy_marker ? " (legacy marker)" : "");
    $("overlay-accepted").textContent =
      o.accepted && o.accepted.length ? o.accepted.join(", ") : "—";
    $("overlay-note").textContent = note;
  }

  function panelError(prefix, err) {
    setBadge("badge-" + prefix, "fail");
    var list = $("list-" + prefix);
    list.textContent = "";
    var li = document.createElement("li");
    li.className = "empty-note";
    li.textContent = "scan failed: " + err;
    list.appendChild(li);
  }

  function setStatus(msg, isError) {
    statusLine.textContent = msg;
    statusLine.className = isError ? "status-line error" : "status-line";
  }

  function setBusy(busy) {
    runBtn.disabled = busy;
    ["scan-esp", "scan-gate", "scan-dl", "scan-quar", "scan-load", "scan-ledger", "scan-rt", "scan-rel", "scan-else", "scan-haz", "scan-launch", "scan-guard", "scan-fix"].forEach(function (id) {
      $(id).disabled = busy;
    });
    runBtn.textContent = busy ? "CONSULTING THE GATE…" : "RUN FULL DOCTOR";
  }

  // ---------- actions ----------

  function runFullDoctor() {
    setBusy(true);
    setStatus("decoding contracts and scanning the install…", false);
    invoke("run_full_doctor", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    })
      .then(function (report) {
        renderContracts(report.contracts);
        renderOverlay(report.overlay);
        renderEsp(report.esp);
        renderGate(report.gate);
        renderDownloads(report.downloads);
        renderQuarantine(report.quarantine);
        renderLoad(report.load_order);
        renderLedger(report.ledger);
        renderRuntime(report.runtime);
        renderRelease(report.release);
        renderElsewhere(report.elsewhere);
        renderHazards(report.hazards);
        renderLaunch(report.launch);
        renderGuard(report.update_guard);
        renderFix(report.troubleshoot);
        var driftNote =
          report.overlay.verdict === "Drift"
            ? " (overlay drift — failures are version skew)"
            : "";
        var totalFail =
          report.esp.failed +
          report.gate.failed +
          report.downloads.failed +
          (report.quarantine ? report.quarantine.failed : 0) +
          (report.load_order ? report.load_order.failed : 0) +
          (report.runtime ? report.runtime.failed : 0) +
          (report.release ? report.release.failed : 0) +
          (report.elsewhere ? report.elsewhere.failed : 0) +
          (report.hazards ? report.hazards.failed : 0) +
          (report.launch ? report.launch.failed : 0) +
          (report.update_guard ? report.update_guard.failed : 0) +
          (report.troubleshoot ? report.troubleshoot.failed : 0);
        setStatus(
          totalFail === 0
            ? "the gate holds. every contract passes." + driftNote
            : totalFail + " failure(s)." + driftNote,
          totalFail !== 0 && report.overlay.verdict !== "Drift"
        );
      })
      .catch(function (err) {
        setStatus("doctor failed: " + err, true);
      })
      .finally(function () {
        setBusy(false);
      });
  }

  function runSingle(btnId, prefix, cmd, args, render) {
    $(btnId).disabled = true;
    setBadge("badge-" + prefix, "idle");
    invoke(cmd, args)
      .then(render)
      .catch(function (err) {
        panelError(prefix, err);
      })
      .finally(function () {
        $(btnId).disabled = false;
      });
  }

  runBtn.addEventListener("click", runFullDoctor);

  $("scan-esp").addEventListener("click", function () {
    runSingle("scan-esp", "esp", "scan_esp_gate", {
      dataDir: dataDir(),
      asarPath: asarInput.value,
    }, renderEsp);
  });

  $("scan-gate").addEventListener("click", function () {
    runSingle("scan-gate", "gate", "scan_build_gate", {
      dataDir: dataDir(),
      asarPath: asarInput.value,
    }, renderGate);
  });

  $("scan-dl").addEventListener("click", function () {
    runSingle("scan-dl", "dl", "verify_downloads", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderDownloads);
  });

  $("scan-quar").addEventListener("click", function () {
    runSingle("scan-quar", "quar", "read_quarantine", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderQuarantine);
  });

  $("scan-load").addEventListener("click", function () {
    runSingle("scan-load", "load", "read_load_order", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderLoad);
  });

  $("scan-ledger").addEventListener("click", function () {
    runSingle("scan-ledger", "ledger", "read_install_ledger", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderLedger);
  });

  $("scan-rt").addEventListener("click", function () {
    runSingle("scan-rt", "rt", "read_runtime", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderRuntime);
  });

  $("scan-rel").addEventListener("click", function () {
    runSingle("scan-rel", "rel", "read_release_book", {
      skyrimRoot: rootInput.value,
    }, renderRelease);
  });

  $("scan-else").addEventListener("click", function () {
    runSingle("scan-else", "else", "read_play_elsewhere", {
      skyrimRoot: rootInput.value,
    }, renderElsewhere);
  });

  $("scan-haz").addEventListener("click", function () {
    runSingle("scan-haz", "haz", "read_hazards", {
      skyrimRoot: rootInput.value,
    }, renderHazards);
  });

  $("scan-launch").addEventListener("click", function () {
    runSingle("scan-launch", "launch", "read_launch", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderLaunch);
  });

  $("scan-guard").addEventListener("click", function () {
    runSingle("scan-guard", "guard", "read_update_guard", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderGuard);
  });

  $("scan-fix").addEventListener("click", function () {
    runSingle("scan-fix", "fix", "read_troubleshoot", {
      skyrimRoot: rootInput.value,
      asarPath: asarInput.value,
    }, renderFix);
  });

  // Decode the contract strip on load so the UI opens with context.
  invoke("decode_contracts", { asarPath: asarInput.value })
    .then(renderContracts)
    .catch(function () {
      /* asar may not exist on this machine — stay quiet, strip stays hidden */
    });
  invoke("check_overlay_drift", {
    skyrimRoot: rootInput.value,
    asarPath: asarInput.value,
  })
    .then(renderOverlay)
    .catch(function () {
      /* same — no local install, no banner */
    });
})();
