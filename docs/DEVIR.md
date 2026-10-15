<!-- devir: 2026-10-14 23:10 | yazan: claude | commit: c9a341e -->
# Devir notu
<!-- /devir üzerine yazar; elle düzeltirsen "yazan: kullanıcı" yap. ≤40 satır. Geçmiş git log'da, kararlar KARARLAR.md'de. -->

## Yön
Windows Swiss-army-knife (Tauri v2 + Svelte 5); analiz önce, onaysız silme yok, her şey geri alınabilir.
Hedef: Codex for OSS başvurusu (yıldız eşiği yok); öncelik yayın ve dağıtım.

## Şu an
- v0.2.0 yayında. winget: microsoft/winget-pkgs#452789 açık, CLA imzalı, doğrulama 01–10 geçti (kurulum testi dahil); Microsoft incelemesi bekleniyor.
- Temiz makine kabulü geçti (GitHub windows-latest, `install-smoke.yml`): indir + SHA + sessiz kurulum + açılış ekranı + kaldırma.
- VirusTotal: 3/71 jenerik ML tespiti (Arctic Wolf, DeepInstinct, SecureAge); Microsoft temiz. URL raporu 0/93.
- Eski uzak dallar silindi.

## Yarım kalan
- Demo GIF (senaryo `docs/DEMO-SCRIPT.md`): kullanıcının gerçek makinesinde çekilmez, gerçek klasör adları görünür.
- FindingList açılır reason ve v0.2.0 sayfaları canlıda görsel doğrulanmadı.

## Açık sorular
- TR arayüz: 117/352 anahtar, dil seçici yok (DECISIONS-NEEDED).
- Rust crate'leri 2.12'ye çekmek windows 0.62 + wry 0.57 getiriyor; npm + crate birlikte yükseltilmeli.

## Sıradaki adım
1. Demo GIF, sonra Codex for OSS başvurusu.
2. winget PR'ı Microsoft onaylayınca birleşir; sonraki sürümler için `packaging/winget/README.md`.
3. Kod imzalama (SmartScreen uyarısı + VT ML tespitleri): SignPath Foundation 2026-10-15 reddetti (görünürlük yetersiz, K-014). Şimdilik imzasız; görünürlük artınca yeniden başvur. Ücretli/ucuz yol: Certum Open Source sertifikası.

## Bilinen tuzaklar
- 5 symlink testi lokal Developer Mode olmadan düşer (CI'da geçer). EFS testleri `WINKNIFE_EFS_TESTS=1` ile.
- CI yalnız main'e push/PR'da tetiklenir; başka base'li PR için `gh workflow run ci.yml --ref <dal>`.
- `src-tauri/target-review/` (eski derleme önbelleği) ve `chatgpt Pocketknife addon.txt` kullanıcının; dokunma.
- Repo PUBLIC: Armada'nın otomatik "Son oturum" bloğu (`oto:başla` işaretli) kullanıcı isteklerini DEVIR'e yazar; commit'e girmeden sil.
- winget kurulumu CI'da SmartScreen'de takılır (imzasız installer); CI yalnız `winget validate` yapar.
