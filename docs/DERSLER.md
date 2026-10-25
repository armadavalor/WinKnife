<!-- durum: aktif -->
# Dersler

Oturuma otomatik verilmez; `/temizlik` ve büyük kararlar öncesi okunur. Aynı ders ikinci kez yaşanırsa CLAUDE.md "YAPMA" bölümüne terfi eder ve burada `→ CLAUDE.md` diye işaretlenir.

<!-- Biçim:
- YYYY-MM-DD | YANLIŞ-SANDIK | "X sanıyorduk, aslında Y" | nasıl anlaşıldı
- YYYY-MM-DD | DERS | ne oldu → bundan sonra ne yapılır
-->

- 2026-08-06 | DERS | Faz 1'de üç ayrı hata aynı kökten: aynı yolun iki yazımını metin olarak karşılaştırmak → her karşılaştırma `core::path_compare` (K-003)
- 2026-08-07 | YANLIŞ-SANDIK | "UI'da çok hata var" sanıyorduk, aslında tek kök: `$effect(() => store.load())` + guard'ın `$state` okuması sonsuz effect/IPC döngüsü | 6c55787 canlı CDP ile
- 2026-10-13 | YANLIŞ-SANDIK | "Backend OS-owned toggle'ı reddetmiyor" sanıyorduk, aslında Rust camelCase serileştiriyor, TS snake_case okuyordu | PR #24, canlı CDP 26/26 disabled
- 2026-10-13 | DERS | Tauri npm paketi crate minor'undan ileri gidince `tauri build` reddediyor → npm sürümünü crate'e kilitle (K-010)
- 2026-10-03 | DERS | Worker'ın "yeşil" raporu kanıt değil → adversarial review + probe; karantina yaması ancak 4 turda SHIP aldı
- 2026-10-14 | DERS | `git commit -a` Armada'nın otomatik DEVIR bloğunu (kullanıcı istekleri, araç kimlikleri) public dala taşıdı → public repoda yalnız adlandırılmış dosyaları stage et, push öncesi `git show --stat` bak
- 2026-10-18 | DERS | Tek satırlık "test:" commit'i (7102773) eski worktree'den #42'nin 12 dosyasını geri aldı, #43 merge'ünde fark edilmedi → merge öncesi `git diff --stat origin/main...<dal>` beklenen dosya listesiyle karşılaştır; başlıkla uyuşmayan silme = dur
- 2026-10-26 | DERS | Merge'ü biten oturumların worktree'leri ve `src-tauri/target`'ları (her biri 6-19 GB) yerinde kaldı; T: 5 GB'a düştü, 4 yeni oturum başlayamadı → PR merge olunca oturumu hemen arşivle (worktree'yi siler); arşivlenemeyen/korunan worktree'nin `src-tauri/target`'ını sil; yeni oturum açmadan önce boş alana bak (oturum başı ≥5.4 GB, hedef ≥30 GB). `target-review/` ve ana `src-tauri/target` hariç
