<!-- devir: 2026-10-20 | yazan: claude | commit: 3ddbadc -->
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
- 2026-10-20 main'de (sürümsüz, masaüstüne kurulu): Polish 6–7 bıçak logosu + 3 varyantlı arka plan ışığı (#65), ışık tüm yüksekliğe yayıldı (#66), Polish 9 QA/QC ilk tur (#67): `docs/QA-CHECKLIST.md` 35 vaka, duplicates/treemap Stop hataları testle düzeltildi.

## Yarım kalan
- Demo GIF (senaryo `docs/DEMO-SCRIPT.md`): kullanıcının gerçek makinesinde çekilmez, gerçek klasör adları görünür.
- v0.3.0 sürüm sayfası canlıda görsel doğrulanmadı.

## Açık sorular
- TR arayüz: 117/352 anahtar, dil seçici yok (DECISIONS-NEEDED).
- Rust crate'leri 2.12'ye çekmek windows 0.62 + wry 0.57 getiriyor; npm + crate birlikte yükseltilmeli.

## Sıradaki adım
0. **Kullanıcı kararı bekliyor (yıkıcı yol):** geri yükleme öğeyi sahiplenip taşımadan önce süreç ölürse öğe yetim kalır (gizli, hiç purge edilmez). DECISIONS-NEEDED'de önerilen açılışta onarım; onaysız uygulanmaz.
1. QA/QC devamı: `docs/QA-CHECKLIST.md` içindeki UNTESTED vakalar (kilitli/uzun yol, büyük ağaç, iptal uçtan uca). Kullanıcı arka plan ışığını gerçek ekranda görsün; sonraki sürüm (0.4.0) tag'i kullanıcı onayıyla.
2. Demo GIF, sonra Codex for OSS başvurusu.
3. winget 0.2.0 birleşince 0.3.0 manifestini microsoft/winget-pkgs'e gönder (`packaging/winget/README.md`).
4. Kod imzalama (SmartScreen uyarısı + VT ML tespitleri): SignPath Foundation 2026-10-15 reddetti (görünürlük yetersiz, K-014). Şimdilik imzasız; görünürlük artınca yeniden başvur. Ücretli/ucuz yol: Certum Open Source sertifikası.

## Bilinen tuzaklar
- 5 symlink testi lokal Developer Mode olmadan düşer (CI'da geçer). EFS testleri `WINKNIFE_EFS_TESTS=1` ile.
- CI yalnız main'e push/PR'da tetiklenir; başka base'li PR için `gh workflow run ci.yml --ref <dal>`.
- `src-tauri/target-review/` (eski derleme önbelleği) ve `chatgpt Pocketknife addon.txt` kullanıcının; dokunma.
- Repo PUBLIC: Armada'nın otomatik "Son oturum" bloğu (`oto:başla` işaretli) kullanıcı isteklerini DEVIR'e yazar; commit'e girmeden sil.
- e2e `duplicates: Stop cancels the scan` flake'inin kökü bulundu ve düzeltildi (2026-10-20, QA/QC): kökte uygun aday yokken backend progress olayı yollamıyor, UI ertelenen Stop'u gönderemiyordu (docs/QA-CHECKLIST.md B3, B8).
- winget kurulumu CI'da SmartScreen'de takılır (imzasız installer); CI yalnız `winget validate` yapar.
