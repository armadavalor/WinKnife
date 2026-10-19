<!-- devir: 2026-10-19 | yazan: claude | commit: b051fa9 -->
# Devir notu
<!-- /devir üzerine yazar; elle düzeltirsen "yazan: kullanıcı" yap. ≤40 satır. Geçmiş git log'da, kararlar KARARLAR.md'de. -->

## Yön
Windows Swiss-army-knife (Tauri v2 + Svelte 5); analiz önce, onaysız silme yok, her şey geri alınabilir.
Hedef: Codex for OSS başvurusu (yıldız eşiği yok); öncelik yayın ve dağıtım.

## Şu an
- **v0.3.0 yayında (2026-10-19)**, imzasız (K-014). install-smoke (indir + SHA + sessiz kurulum + açılış + kaldırma + winget validate) yeşil. Masaüstüne kuruldu.
- winget: 0.2.0 PR'ı microsoft/winget-pkgs#452789 Microsoft incelemesinde; 0.3.0 manifesti repoda (`packaging/winget/.../0.3.0`), 0.2.0 birleşince gönderilecek.
- Toplu temizlik/geri yükleme yavaş (CI'da 5000 dosya: temizlik 58 s, geri yükleme 47 s; lokal ~1 ms/öğe). Kök neden: öğe başı fsync'li ledger commit'i + taşıma. Batch ertelendi (çökme penceresi büyür); `synchronous` değişikliği kullanıcıya sorulmadan yapılmaz. Dal `perf/bulk-clean-restore` (PR #56 kapalı).
- main branch protection açık; merge yetkisi ajanda (K-015).

## Yarım kalan
- Demo GIF (senaryo `docs/DEMO-SCRIPT.md`): kullanıcının gerçek makinesinde çekilmez, gerçek klasör adları görünür.
- v0.3.0 sürüm sayfası canlıda görsel doğrulanmadı.

## Açık sorular
- TR arayüz: 117/352 anahtar, dil seçici yok (DECISIONS-NEEDED).
- Rust crate'leri 2.12'ye çekmek windows 0.62 + wry 0.57 getiriyor; npm + crate birlikte yükseltilmeli.

## Sıradaki adım
0. **QA/QC turu (kullanıcı, 2026-10-18):** her şey, özellikle silme/temizlik/karantina ve arama/tarama akışları; BACKLOG Polish queue madde 9.
1. Kullanıcı notları (2026-10-18) `docs/modules/BACKLOG.md` "Polish queue": ilerleme detayı + bar animasyonu, logo animasyonu, 3 varyantlı mouse'a tepkili arka plan ışığı, animasyon kapatma. Direkt silme yok, yalnız karantina (K-016); hızlı temizlik denenince yeniden bakılır. Temizlik süresi kullanıcının PC'sinde ölçülmedi.
2. Demo GIF, sonra Codex for OSS başvurusu.
3. winget 0.2.0 birleşince 0.3.0 manifestini microsoft/winget-pkgs'e gönder (`packaging/winget/README.md`).
4. Kod imzalama (SmartScreen uyarısı + VT ML tespitleri): SignPath Foundation 2026-10-15 reddetti (görünürlük yetersiz, K-014). Şimdilik imzasız; görünürlük artınca yeniden başvur. Ücretli/ucuz yol: Certum Open Source sertifikası.

## Bilinen tuzaklar
- 5 symlink testi lokal Developer Mode olmadan düşer (CI'da geçer). EFS testleri `WINKNIFE_EFS_TESTS=1` ile.
- CI yalnız main'e push/PR'da tetiklenir; başka base'li PR için `gh workflow run ci.yml --ref <dal>`.
- `src-tauri/target-review/` (eski derleme önbelleği) ve `chatgpt Pocketknife addon.txt` kullanıcının; dokunma.
- Repo PUBLIC: Armada'nın otomatik "Son oturum" bloğu (`oto:başla` işaretli) kullanıcı isteklerini DEVIR'e yazar; commit'e girmeden sil.
- e2e `duplicates: Stop returns to idle` #50'de bir kez 61 s zaman aşımıyla düştü, yeniden çalışınca geçti: flaky, QA turunda bak.
- winget kurulumu CI'da SmartScreen'de takılır (imzasız installer); CI yalnız `winget validate` yapar.
