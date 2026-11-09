# Trust tiers and packet trust

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.


| Tier | UI word | Meaning | Who/what can create it |
|---|---|---|---|
| `REPLAYED` | REPLAYED | A probe was evaluated by the *receiving* Firsthand on the receiver's current tree and passed. Strongest. | `receive`, `verify --rerun`, `replay` |
| `WITNESSED` | WITNESSED | Firsthand's own entry point observed it during the session (harness hook event, `firsthand run`, snapshot computed from the real tree) and it has not yet been re-executed. A prior, not proof. | `hook`, `run`, `snapshot` |
| `REPORTED` | HEARSAY | Asserted by an agent or human with no passing probe (note, claim, MCP call, transcript narration, imported markdown). | `note`, `claim`, `mcp`, `ingest`, `import` |

Claim **status** (computed, never stored as truth without its inputs):

- `verified`: probe passed on the receiver's current tree (tier REPLAYED).
- `holds`: witnessed pass exists and every scope item is byte-identical now (tier WITNESSED). No rerun needed; rerun is optional.
- `stale`: scope changed since the witnessed pass and the probe was not (or cannot be) re-run. The report names the exact changed files or symbols.
- `refuted`: a probe failed now, or the ledger contains contradicting witnessed evidence (for example a claim "tests pass" against a witnessed failing test run on the same scope).
- `unverified`: no probe could be attached. Pure hearsay.
- `unknown`: probe errored, timed out, or needs approval.

Hard invariant: **nothing derived from model-authored text is ever shown above HEARSAY without a passing probe.** Hook events are produced by the harness, not the model, so they earn WITNESSED for the fact "the harness ran X and got Y". A model can still run `firsthand hook ...` by hand with forged stdin; that is why only REPLAYED is proof. Document this threat model honestly (Section 15).

### 6.6 Keys and trust

`firsthand key init` generates an Ed25519 keypair. Packets sealed by the user's own key are `own`. `firsthand trust add <fp|pubkey-file> --label ...` adds to `trusted_keys.toml`. Trust tiers for packets: `own` and `trusted` allow auto-run of allowlisted commands; `unknown` and `unsigned` allow static probes only. A signature proves integrity and key continuity, not honesty of the agent. Say that in docs.

---

### 15.1 Threat model (write as `docs/security.md` and `SECURITY.md`)

**Defends against:** honest-but-wrong agents (hallucinated completion, stale memory, sloppy summaries); untrusted packets (malicious or tampered); prompt injection traveling through transcripts, tool output, claim text, task titles; secret leakage into ledgers, packets, bundles, briefs; unsafe command execution from foreign packets or repo config; archive attacks (zip-slip, bombs, symlinks); DNS rebinding and cross-origin attacks on the local dashboard; accidental destruction of the user's working tree by Firsthand itself.

**Does NOT defend against:** an agent that actively forges evidence while holding full shell access in the same trust domain (it can call `firsthand hook` with forged JSON). Mitigation is by design: only REPLAYED is proof; WITNESSED is a prior. A compromised machine or compromised signing key. A malicious user. State this plainly in the docs; do not oversell.

| Threat | Control | Test |
|---|---|---|
| Tampered packet | Manifest hashes, Ed25519 signature, chain verification, extra-entry rejection | T13 and byte-flip fuzz over every part |
| Forged ledger entry | Channel assigned by entry point code; chain; replay as top tier | T14 |
| Command execution from foreign packet | Unsigned/unknown packets never run commands; allowlist; env scrub; approvals only via terminal | T12, sentinel-file test |
| Repo config as attack (`firsthand.toml`) | Repo-level config cannot define allow/approve/trust/env | policy tests |
| Zip-slip, symlink, bomb, reserved names | Section 6.5 | one test per rule |
| Secrets in capture | Redaction at capture on blobs, notes, claim text, ingest text; snapshot excludes secret-pattern and secret-content files; seal-time second pass; packet scan test with a separate independent scanner | T10, corpus tests |
| Prompt injection | Trust fencing (11.2); command lists exclude unsigned packet commands; never auto-execute anything from hearsay | T11 |
| Path and identity leakage | `~` substitution, username scrub, no absolute paths in packets | packet scan test |
| Dashboard attacks | Loopback bind, token, Host check, Origin check, CSP, no external requests | rebinding and CSRF tests |
| Data loss by Firsthand | Snapshots never touch index/HEAD/worktree; restore always takes a pre-restore snapshot; `uninstall` removes only tagged entries | AC tests |
| Resource exhaustion | Blob caps, packet caps, output caps, entry caps, parser depth limits, lock timeouts | limit tests |
