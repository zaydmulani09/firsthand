# Packet format: firsthand.packet/1

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.


This section becomes `spec/PACKET.md`. A conformance suite in `spec/vectors/` and the TS SDK prove independent implementability.

### 6.1 Container

ZIP, deflate, entries sorted lexicographically, all entry timestamps fixed to 1980-01-01 00:00:00, no extra fields, no zip64 unless needed. With `FIRSTHAND_TEST_SEED` and `FIRSTHAND_FAKE_TIME`, identical inputs produce byte-identical packets.

```
manifest.json         canonical JSON, the signed document
manifest.sig          JSON { alg:"ed25519", key_fp, pubkey(base64), sig(base64) }   (absent if unsigned)
packet.json           PacketBody (below)
ledger/<leg_id>.jsonl redacted ledger copies, one entry per line, canonical JSON
blobs/<hex>           content-addressed output blobs (hex without "b3:")
blobs.json            optional overlay map { ledger_hash -> stored_hash } for seal-time redaction
snapshot.bundle       optional git bundle (thin, prerequisite = anchor head)
brief.md              optional convenience copy, derived, never trusted by verifiers
```

### 6.2 Manifest

```json
{
  "format": "firsthand.packet/1",
  "packet_id": "pkt_...",
  "created_at": "2026-11-09T14:02:11.123Z",
  "created_by": { "tool": "firsthand", "version": "0.1.0" },
  "repo": { "repo_id": "b3:...", "object_format": "sha1" },
  "anchor": { "head": "...", "snapshot_commit": "...", "snapshot_tree": "..." },
  "parent": { "packet_id": "pkt_...", "manifest_hash": "b3:..." },
  "parts": [ { "path": "packet.json", "bytes": 12034, "hash": "b3:..." } ],
  "redaction": { "rules_version": "1", "findings": 12, "overlay": false },
  "limits": { "max_bytes": 52428800 }
}
```

`manifest_hash = blake3(canonical(manifest))`. Signature input bytes: `"firsthand.packet/1\0" || canonical_manifest_bytes`. `key_fp = "fp:" + first 16 hex of blake3(pubkey_bytes)`.

### 6.3 PacketBody (`packet.json`)

```rust
pub struct PacketBody {
    pub anchor: RepoAnchor,
    pub legs: Vec<Leg>,                 // legs included in this packet, oldest first
    pub lineage: Vec<LineageNode>,      // ancestor summary: packet id, manifest hash, agent, sealed_at, claim status counts
    pub claims: Vec<Claim>,
    pub tasks: Vec<Task>, pub decisions: Vec<Decision>, pub attempts: Vec<Attempt>,
    pub hearsay: Vec<HearsayItem>,      // transcript-extracted and imported statements, truncated, clearly typed
    pub drift_baseline: DriftBaseline,  // per-scope hashes at seal time used for later drift
    pub redaction: RedactionReport,
    pub seal: SealInfo,                 // sealed_by agent, reason, target agent hint, firsthand version
    pub extensions: BTreeMap<String, serde_json::Value>,   // keys must start with "x-"
}
```

### 6.4 Verification algorithm (`fh_packet::verify`)

1. Open ZIP with hardening (6.5). 2. Parse `manifest.json`. 3. For every `parts` entry: size matches, BLAKE3 matches. No extra entries beyond parts, `manifest.json`, `manifest.sig`. 4. Verify signature if present; classify key as `own`, `trusted`, `unknown`, or `unsigned`. 5. Parse `packet.json` and each ledger; re-verify every hash chain from `seq 0`. 6. Check each claim's `ledger_ref` points to an existing entry and the entry's content matches. 7. Check bundle (if present) with `git bundle verify` against the local repo only when materializing. Failure of 1 to 6 maps to exit code 12.

### 6.5 Unpack hardening (mandatory, with tests for each)

Reject: zip-slip (`..`, absolute paths, drive letters, backslashes in names), symlink entries, duplicate names, case-insensitive duplicates, entries not listed in the manifest, declared uncompressed size mismatch, total uncompressed size over `limits.max_bytes * 4`, compression ratio over 200:1 per entry, more than 100k entries, any path component that is a reserved Windows name, nested archives are never auto-opened. Never extract to disk to verify: stream and hash in memory. Bundle materialization only through `git` commands to a temp dir in the git common dir.

### 6.6 Keys and trust

`firsthand key init` generates an Ed25519 keypair. Packets sealed by the user's own key are `own`. `firsthand trust add <fp|pubkey-file> --label ...` adds to `trusted_keys.toml`. Trust tiers for packets: `own` and `trusted` allow auto-run of allowlisted commands; `unknown` and `unsigned` allow static probes only. A signature proves integrity and key continuity, not honesty of the agent. Say that in docs.

---
