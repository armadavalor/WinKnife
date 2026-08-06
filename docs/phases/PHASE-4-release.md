# Phase 4 — LLM option + release

Prereq: Phase 3 accepted. Read ARCHITECTURE §4 Layer 2.

## Tasks
1. **LLM layer**: `LlmProvider` trait + Anthropic/OpenAI/Ollama adapters; keyring storage; settings UI with explicit privacy explanation ("metadata only, never file contents" + show-exact-payload preview button — this preview is a trust feature, mandatory). Integration points: "Explain" button on yellow items, unknown-dir classification suggestion (suggestion only — never changes bucket above rule ceiling).
2. Graceful degradation tests: no key / offline / provider error → rule-engine behavior, quiet failure with retry affordance.
3. **Release engineering**: NSIS installer + portable exe, versioning (`v0.1.0`), GitHub Release workflow on tag, artifact checksums in release notes, VirusTotal scan link step documented in `docs/RELEASING.md` (create).
4. **README overhaul** (English + `README.tr.md`): hero GIF (script the demo: scan→review→clean→restore in <20s), feature grid, safety/privacy section (quarantine, offline-first, non-goals list from BACKLOG.md), dev setup, contributing (rule-pack PR guide), Buy Me a Coffee link, `good first issue` labels seeded from BACKLOG Tier A.
5. Final audit vs CLAUDE.md hard rules: grep for raw deletes outside quarantine, network calls outside llm/, unexplained recommendations.

## Acceptance
Tag → CI produces installer + portable artifacts; fresh Windows VM: install, scan, clean, restore, uninstall cleanly. README renders with GIF. Audit findings: zero.
