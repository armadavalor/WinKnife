<!-- durum: aktif -->
# Rule Pack Format (`rules/*.json`)

Community-editable data that drives the brain's Layer 1. Parser must reject malformed packs with a clear error listing pack name + field (loud failure beats silent skip).

```json
{
  "$schema": "./rule-pack.schema.json",
  "id": "windows-temp",
  "name": "Windows Temp Files",
  "version": 1,
  "module": "cleaner",
  "enabled_by_default": true,
  "description": "Temporary files created by Windows and applications.",
  "rules": [
    {
      "id": "user-temp-files",
      "paths": ["%TEMP%\\**"],
      "exclude": ["%TEMP%\\**\\*.lock"],
      "match": { "min_age_hours": 24 },
      "score": { "base": 90, "max_bucket": "safe" },
      "action": "quarantine",
      "reason": "Temporary file older than 24h in the user temp folder; applications recreate these as needed."
    }
  ]
}
```

Field notes:
- `paths`: glob patterns; env vars expanded case-insensitively. Absolute patterns outside user profile require `"requires_admin": true` on the rule.
- `match` (all optional, AND-ed): `min_age_hours`, `min_size_bytes`, `extensions: []`, `owner_process_absent: "chrome.exe"` (skip/downgrade if process running).
- `score.base`: 0–100 starting confidence before heuristics. `score.max_bucket`: ceiling — heuristics may lower a bucket, never raise above this. Allowed: `safe | needs_review | do_not_delete`.
- `reason`: user-facing, required, plain language. This string ships in the UI verbatim.
- Protected paths override everything here, and the override is a load-time/compile-time property rather than a runtime decision. Precisely:
  - A rule that **proposes an action** on a protected path is a **test failure**. `no_shipped_rule_targets_a_protected_path` asks the real brain to prove it refuses a file under that root, so this is checked against behaviour and not by inspection.
  - A rule that is **informational** about a protected path is **permitted**, and must be *structurally* incapable of acting: `"action": "info_only"` together with `"score": { "max_bucket": "do_not_delete" }`. The parser rejects the contradiction (`quarantine` with `max_bucket: do_not_delete`), and `Recommendation::validate` rejects a mutating action on a red item, so there is no combination of rule data that makes such a rule act.
  - This is not a loophole. `docs/PLAN.md` §3.1 defines the red bucket as information only, with no button — "there is this much here, I will not touch it, here is what will" is a better answer than silence. A rule that takes that road owes the user a `reason` that names the space involved and points at the tool that *will* reclaim it; see `rules/windows-update-leftovers.json`.

Ship `rule-pack.schema.json` (JSON Schema) alongside for editor validation and CI check (`cargo test` includes a validate-all-packs test). Community PR flow: new pack file + fixture test entry proving buckets.
