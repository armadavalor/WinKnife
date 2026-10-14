# CLAUDE.md — Coding Agent Instructions

You are the implementing engineer for this project. The architecture, module specs, and phase plan are already decided — **do not redesign them**. Your job is to implement exactly what the current phase brief asks, well.

## Read order (before writing any code)

1. This file
2. `docs/ARCHITECTURE.md` — system design, trait contract, data model
3. `docs/CONVENTIONS.md` — code style, testing, commits
4. The phase brief you were assigned (`docs/phases/PHASE-N-*.md`)
5. The module spec(s) that phase references (`docs/modules/*.md`)

## Project identity

Open-source Windows "Swiss army knife" utility. Tauri v2 (Rust backend) + Svelte 5/TypeScript UI. Non-profit, portfolio project. Core promise: **analyze first, never delete without consent, everything reversible.**

## Hard rules (violating these is a failed task)

- **Never permanently delete user files.** All destructive actions go through the Quarantine service (`docs/ARCHITECTURE.md §6`). No `fs::remove_*` on user data outside quarantine purge. One documented exception: emptying the Windows recycle bin (`SHEmptyRecycleBinW`) is permanent by nature; its row must never start ticked, and the backend refuses it without the separate confirmation flag from the warning dialog (DECISIONS-NEEDED, Phase 4 release audit).
- **Protected paths are absolute.** `C:\Windows`, `C:\Program Files*`, user profile roots, and anything in `brain::protected_paths()` can never receive a Safe (green) rating or a delete action. Enforce in code, not just in rules data.
- **No network calls without explicit user opt-in.** The LLM layer is the only networked component and is off by default.
- **All scans are read-only.** `scan()` and `analyze()` must not modify the filesystem or registry.
- **Every Recommendation carries a human-readable `reason`.** No unexplained suggestions.
- Implement only what the assigned phase brief lists. If you spot something missing or contradictory, add a note under `docs/DECISIONS-NEEDED.md` instead of improvising scope.

## Definition of done (every task)

- `cargo build --release` and `cargo clippy -- -D warnings` pass
- `cargo test` passes; new logic has unit tests (rule scoring, path classification, and quarantine logic are mandatory test targets)
- UI builds: `npm run build` with no TS errors
- New module registered in the module registry and reachable from the sidebar
- `docs/CHANGELOG-DEV.md` gets a short entry: what was built, decisions taken, anything deferred

## When uncertain

Prefer the boring, reversible option. Record the dilemma in `docs/DECISIONS-NEEDED.md` and continue with the safest interpretation. Do not block waiting for answers unless the ambiguity is destructive-action-related — those you always escalate.

## Hafıza ve arşiv (docs/)
- Oturum başında "Proje hafızası" bloğu gelir (Armada of Claude modu). Gelmediyse önce `docs/DEVIR.md` oku.
- `docs/DEVIR.md`: nerede kaldık (iş bitince `/devir`). `docs/KARARLAR.md`: yalnız AKTİF satırlar geçerli. `docs/KANONIK.md`: varlıkların tek geçerli yolu. `docs/DERSLER.md`: yanlış-sandıklar ve dersler. `docs/LISANSLAR.md`: dışarıdan gelen her varlık/paketin kaynağı ve lisansı (eklerken satır).
- `_arsiv/` ve `<!-- durum: arşiv -->` taşıyan dosyalar geçersiz: okuma, kopyalama, örnek alma. Glob sonucunda çıkarsa yok say.
- Yön değişince: KARARLAR'a satır, eski varlık `_arsiv/YYYY-MM-ad/` (+NEDEN.md), KANONIK güncelle; tek commit. Temizlik, pivot, dondurma, tatbikat: `/temizlik`. Pre-commit hafıza kapısı `.githooks/` (bypass yalnız kullanıcı isterse: `--no-verify`).
