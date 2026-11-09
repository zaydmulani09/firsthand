//! Generators: JSON Schema export and dashboard dev fixtures.

use anyhow::Result;
use fh_core::policy::{Policy, RepoConfig};
use fh_core::*;
use schemars::JsonSchema;
use serde_json::{Value, json};
use std::path::Path;

fn write(dir: &Path, name: &str, v: &Value) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    std::fs::write(dir.join(name), s)?;
    Ok(())
}

fn one<T: JsonSchema>(dir: &Path, name: &str) -> Result<()> {
    let s = schemars::schema_for!(T);
    write(dir, &format!("{name}.json"), &serde_json::to_value(s)?)
}

/// Export JSON Schema for every public data type.
pub fn schema(dir: &Path) -> Result<()> {
    one::<PacketBody>(dir, "PacketBody")?;
    one::<Manifest>(dir, "Manifest")?;
    one::<ManifestSig>(dir, "ManifestSig")?;
    one::<LedgerEntry>(dir, "LedgerEntry")?;
    one::<CommandBody>(dir, "CommandBody")?;
    one::<FileEditBody>(dir, "FileEditBody")?;
    one::<IngestBody>(dir, "IngestBody")?;
    one::<BriefDeliveredBody>(dir, "BriefDeliveredBody")?;
    one::<ToolOtherBody>(dir, "ToolOtherBody")?;
    one::<NoteBody>(dir, "NoteBody")?;
    one::<SnapshotBody>(dir, "SnapshotBody")?;
    one::<OpBody>(dir, "OpBody")?;
    one::<Claim>(dir, "Claim")?;
    one::<Probe>(dir, "Probe")?;
    one::<Scope>(dir, "Scope")?;
    one::<Task>(dir, "Task")?;
    one::<Decision>(dir, "Decision")?;
    one::<Attempt>(dir, "Attempt")?;
    one::<RepoAnchor>(dir, "RepoAnchor")?;
    one::<Policy>(dir, "Policy")?;
    one::<RepoConfig>(dir, "RepoConfig")?;
    one::<HookEvent>(dir, "HookEvent")?;
    one::<IngestedSession>(dir, "IngestedSession")?;
    one::<DriftReport>(dir, "DriftReport")?;
    one::<ClaimEval>(dir, "ClaimEval")?;
    one::<PacketTrust>(dir, "PacketTrust")?;
    eprintln!("gen-schema: wrote {}", dir.display());
    Ok(())
}

/// Realistic dev fixtures for the dashboard, built from fh-core types so they
/// always validate against the schemas. Dev-only inputs; shipped code never reads them.
pub fn fixtures(dir: &Path) -> Result<()> {
    let ids = IdGen::seeded("fixtures");
    let t0 = Ts::parse("2026-11-09T13:40:00.000Z")?;
    let oid = |n: u8| GitOid::parse(&format!("{:02x}", n).repeat(20)).expect("valid oid");
    let leg: Id<Leg> = ids.new_id(&t0);
    let pkt: Id<Pkt> = ids.new_id(&t0);
    let snap_id: Id<Snp> = ids.new_id(&t0);
    let lref = |seq: u64| LedgerRef {
        leg: leg.clone(),
        seq,
        hash: B3::of(&seq.to_le_bytes()),
    };
    let snapshot = SnapshotRef {
        id: snap_id.clone(),
        commit: oid(0x3f),
        tree: oid(0x8e),
        base: Some(oid(0x12)),
        ref_name: format!("refs/firsthand/snapshots/{snap_id}"),
        kind: SnapshotKind::Handoff,
        created_at: t0.clone(),
    };
    let mk_claim = |text: &str,
                    cat: ClaimCategory,
                    src: OriginSource,
                    scope: Scope,
                    probe: Option<Probe>,
                    seq: u64| Claim {
        id: ids.new_id(&t0),
        text: text.to_owned(),
        category: cat,
        origin: ClaimOrigin {
            source: src,
            agent: Some(AgentId::ClaudeCode),
            confidence_permille: 600,
        },
        scope,
        probe,
        results: vec![],
        created_at: t0.clone(),
        leg: leg.clone(),
        ledger_ref: lref(seq),
    };
    let claims = vec![
        mk_claim(
            "cargo test -p fh-core ok (142 passed)",
            ClaimCategory::CommandSucceeded,
            OriginSource::Witnessed,
            Scope::Repo { tree: oid(0x8e) },
            Some(Probe::LedgerVerification {
                kind: VerificationKind::Test,
                matcher: None,
                require: LedgerRequire::TestsAllPass,
            }),
            4,
        ),
        mk_claim(
            "src/auth.rs exists",
            ClaimCategory::FileCreated,
            OriginSource::TranscriptExtracted,
            Scope::Items {
                files: vec![ScopedFile {
                    path: "src/auth.rs".into(),
                    blob: Some(oid(0x44)),
                }],
                symbols: vec![],
            },
            Some(Probe::FileExists {
                path: "src/auth.rs".into(),
            }),
            9,
        ),
        mk_claim(
            "token refresh works",
            ClaimCategory::BugFixed,
            OriginSource::AgentDeclared,
            Scope::Items {
                files: vec![ScopedFile {
                    path: "src/auth/token.rs".into(),
                    blob: Some(oid(0x55)),
                }],
                symbols: vec![],
            },
            None,
            12,
        ),
        mk_claim(
            "all tests pass",
            ClaimCategory::TestsPass,
            OriginSource::TranscriptExtracted,
            Scope::Repo { tree: oid(0x8e) },
            Some(Probe::LedgerVerification {
                kind: VerificationKind::Test,
                matcher: None,
                require: LedgerRequire::TestsAllPass,
            }),
            20,
        ),
    ];
    let evals = vec![
        json!({"claim_id": claims[0].id, "status": "verified", "tier": "replayed", "causes": [], "evidence": lref(4), "result": null, "detail": "cargo test -p fh-core ok (142 passed)", "suggested_probe": null, "needs_approval": null}),
        json!({"claim_id": claims[1].id, "status": "holds", "tier": "witnessed", "causes": [], "evidence": null, "result": null, "detail": "scope unchanged since witnessed pass", "suggested_probe": null, "needs_approval": null}),
        json!({"claim_id": claims[2].id, "status": "unverified", "tier": "reported", "causes": [], "evidence": null, "result": null, "detail": "no probe attached", "suggested_probe": "a test that exercises token refresh", "needs_approval": null}),
        json!({"claim_id": claims[3].id, "status": "refuted", "tier": "witnessed", "causes": [], "evidence": lref(18), "result": null, "detail": "cargo test exited 101 at tree 8e8e8e8 (17 passed, 2 failed)", "suggested_probe": null, "needs_approval": null}),
    ];
    for e in &evals {
        let _: ClaimEval = serde_json::from_value(e.clone())?;
    }
    let drift = DriftReport {
        head_relation: HeadRelation::Ahead { n: 4 },
        commits_since_anchor: 4,
        files_changed: vec![
            PathChange {
                status: "M".into(),
                path: "src/auth/token.rs".into(),
                old_path: None,
            },
            PathChange {
                status: "A".into(),
                path: "docs/notes.md".into(),
                old_path: None,
            },
        ],
        symbols_changed: vec![SymbolChange {
            path: "src/auth/token.rs".into(),
            name: "refresh".into(),
            kind: SymbolKind::Fn,
            change: SymbolChangeKind::BodyChanged,
            old_path: None,
        }],
        claims_affected: vec![ClaimAffected {
            claim_id: claims[2].id.clone(),
            causes: vec![Cause::FileChanged {
                path: "src/auth/token.rs".into(),
                before: Some(oid(0x55)),
                after: Some(oid(0x56)),
            }],
        }],
        branch_changed: false,
        worktree_dirty_now: true,
        current_tree: oid(0x9a),
        notes: vec![],
    };
    write(dir, "claims.json", &serde_json::to_value(&claims)?)?;
    write(dir, "evals.json", &Value::Array(evals))?;
    write(dir, "drift.json", &serde_json::to_value(&drift)?)?;
    write(
        dir,
        "meta.json",
        &json!({"packet_id": pkt, "leg_id": leg, "snapshot": snapshot}),
    )?;
    eprintln!("gen-fixtures: wrote {}", dir.display());
    Ok(())
}
