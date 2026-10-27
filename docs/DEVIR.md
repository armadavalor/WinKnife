<!-- devir: 2026-10-27 | yazan: claude | commit: (#115 sonrası, 65dd300) -->
# Devir notu
<!-- /devir üzerine yazar; elle düzeltirsen "yazan: kullanıcı" yap. ≤40 satır. Geçmiş git log'da, kararlar KARARLAR.md'de. -->

## Yön
Windows Swiss-army-knife (Tauri v2 + Svelte 5); analiz önce, onaysız silme yok, her şey geri alınabilir.
Hedef: Codex for OSS başvurusu (yıldız eşiği yok); öncelik yayın ve dağıtım.

## Şu an
- **v0.5.0 tag'lendi (2026-10-27, 63c4c65)**: release.yml draft sürüm üretir; yayınlama, VirusTotal ve install-smoke elle (RELEASING). Masaüstüne kuruldu. v0.4.0 (2026-10-25) ilk EN+TR ve attestation'lı sürüm. İmzasız (K-014).
- 0.5.0'a girenler (#111–#113): kenar çubuğu akordeonu, Remotion demo altyapısı (`demo/`), XP-Bliss halftone/ASCII bulut arka planı (varsayılan halftone + graphite + animasyon açık; eski "on" tercihi "system"e göçer), panoda sabitlenen araçlar (`winknife.pinnedModules`, + ile ekle/çıkar), kenar çubuğu ve panel saydamlık kaydırıcıları (%30–100), Sky GPU/CPU yükü: bulut dışı pikselde erken çıkış + pencere odak dışındayken duraklama (gerçek GPU % ölçülmedi). e2e artık animasyon kapalı koşar; `goTo` yalnız `nav` linklerine tıklar.
- winget: 0.2.0 PR'ı microsoft/winget-pkgs#452789 Microsoft incelemesinde; 0.3.0 ve 0.4.0 manifestleri repoda.
- Toplu temizlik/geri yükleme yavaş (CI'da 5000 dosya: temizlik 58 s, geri yükleme 47 s; lokal ~1 ms/öğe). Kök neden: öğe başı fsync'li ledger commit'i + taşıma. Batch ertelendi (çökme penceresi büyür); `synchronous` değişikliği kullanıcıya sorulmadan yapılmaz. Dal `perf/bulk-clean-restore` (PR #56 kapalı).
- main branch protection açık; merge yetkisi ajanda (K-015).
- 0.4.0 içeriği: bıçak logosu + arka plan ışığı (#65, #66), QA/QC ilk tur (#67), yarıda ölen geri yüklemenin açılışta onarımı (A9, K-017), A14/A15/A19 UI e2e testleri + uzun yolda dosya adı artık kırpılmıyor (#70). Ayarlar'dan araç gizleme: kenar çubuğu gizlenen aracı göstermez, pano/rota/`--module` açmaya devam eder (#72, issue #8; tercih localStorage `winknife.hiddenModules`). Başlangıç sayfası her kapatılabilir girişte Rust puanını renkli etiketle ve "Bu değerlendirme neden?" gerekçesiyle gösterir; korumalı yol gerekçesi temizleyici diliyle değil yerin adıyla (#75, issue #4). C9 kaynak doğrulama: README "Verify this download", release.yml build attestation (`attest-build-provenance`, yalnız bir sonraki tag'li sürümden itibaren; henüz denenmedi), `docs/ANTIVIRUS-FALSE-POSITIVE.md`, Ayarlar'da "About WinKnife / What WinKnife never does" kartı. C1 geri alınabilir ayar motoru (#79, K-018, `docs/phases/PHASE-6-tweaks.md`): yalnız HKCU, yazmadan önce eski değer (ya da "yoktu") SQLite'a, yazınca geri okuma, Undo eski değeri koyar; yeni "Tweaks" sayfası yol/eski/yeni/kaynak gösterir. C3 yinelenen klasörler (#78): aynı dosya kümesine sahip klasörler grup olur; grup başına tutma kuralı (en eski/en yeni/tercih), tutulmayan kopyalar karantinaya önseçili, korumalı üye asla seçilmez; "Bu grubu yok say" settings tablosunda (`duplicates.ignoredFolderGroups`, migration yok). Her kopyanın gitmesi kodda imkânsız (`keep_at_least_one` + executor kapı 4, testli). C4 hosts düzenleyici (#80, 448d736). C7 sapma kontrolü (#83, ea912c4): uygulanan ayar sonradan değişirse gerekçeli uyarı + Yeniden uygula (C1 yolu); Windows Update engel durumu salt okunur. C10 ayar profili dışa/içe aktarma (#83): sürümlü JSON sınırda doğrulanır, içe aktarma önce fark gösterir, tweak'ler C1 yolundan (fence korunur). C2 bağlam menüsü temizleyici (#84, 87da120): `shell` fiilleri + `shellex\ContextMenuHandlers` (6 kapsam, HKCU+HKLM), gerekçe sahibi CLSID → InprocServer32 → dosya → imzacı zinciriyle adlandırır, imzacısız = "needs review"; Disable C1 hattından (`context-menu:` önekli tweak id, `TweakRouter`), yalnız HKCU (`LegacyDisable` / `Shell Extensions\Blocked`); HKLM yalnız listelenir, hiç yazılmaz. Explorer/Win11 yeni menünün HKCU bloğuna uyması elle denenmedi (DECISIONS-NEEDED). C1, C2, C3, C4, C7, C10 main'de ve masaüstüne kurulu.
- 2026-10-25 (3e2f659, masaüstüne kurulu): uzun listelerde ara/süz/tümünü seç/hiçbirini/tersine, katlanır gruplar, süzgecin gizlediği seçili satır uyarısı (#87, `ListToolbar`, `listTools.ts`). Karantinada elle kalıcı kaldırma: tek/seçim/grup, onay penceresi, kilitli öğe gerekçesiyle kalır; geri yüklenenler listeden düşer (#88, K-020). 27 HKCU tweak, gruplu + aranır (#89; admin tweak'leri DECISIONS-NEEDED). Yönetici olarak yeniden başlat: yalnız tıklamayla `runas`, erişim reddi `needs_admin` → düğme (#90, K-021).
- 2026-10-26 (c7885a4): C8 erişilebilirlik + ölçekleme (#95), C14 ağ modülü: portlar/bağlantılar/DNS flush (#96), C15 ortam değişkeni düzenleyici, önizleme + geri al (#97), C19 kurulu uygulamalar (#98), C6 onarım kiti: tıklamayla SFC/DISM (#99), C7 sapma testi (#100), uzun işlemlerde iptal + zaman aşımı, Restart Manager `TimedRestartManager` (#101, issue #41 kapandı).
- 2026-10-26 (2. tur, masaüstüne kurulu): C12 klasörü koruyup içini temizle (#103, K-027), C16 büyük dosya arşivleyici (#104), C13 treemap'ten etiketle-karantina, kök sürücü reddi (#105, K-028/K-029), C5 kaldırma kalıntıları, kapı 3 istisnası fail-closed yeniden kontrolle (#106, K-030), kenar çubuğu kategori sekmeleri (#107), C11 zamanlı tarama: otomatik karantina varsayılan kapalı, yalnız Safe, onay penceresi (#108, K-031), C18 benzer görseller: bulut-yalnız dosyalar açılmadan atlanır (#109, K-032). #104 artıkları (spawn_blocking, iptal, boş alan kontrolü, `cancelled` bayrağı, testler) küçük takip işi.

## Yarım kalan
- Yok. README b-roll'u (#115) birleşti: `assets/demo/demo.gif` + `demo.mp4`, 14.5 s Remotion; kareler CI `demo.yml` ile temiz runner'da demo profilinden çekilir, gerçek makineden asla. Remotion skill'leri repoda değil, `demo/README.md`'deki komutla yerel kurulur.

## Açık sorular
- TR arayüz (K-022): dil seçici + tam TR çevirisi PR olarak hazır; Rust kaynaklı gerekçe/hata metinleri hâlâ İngilizce (DECISIONS-NEEDED).
- Rust crate'leri 2.12'ye çekmek windows 0.62 + wry 0.57 getiriyor; npm + crate birlikte yükseltilmeli.

## Sıradaki adım
0. **QA/QC turu (kullanıcı, 2026-10-18):** `docs/QA-CHECKLIST.md`. 2026-10-20: A5/A8/A13/A17 testlendi, A9 düzeltildi (K-017). 2026-10-23: A14/A15/A19 UI e2e ile kapandı (#70); ürün hatası çıkmadı. Açık yalnız A18 (gerçek ikinci birim gerek).
1. Polish queue (`docs/modules/BACKLOG.md`, kullanıcı 2026-10-18) tamamen kapandı; direkt silme yok, yalnız karantina (K-016). Temizlik süresi kullanıcının PC'sinde ölçülmedi. Araştırma listesinde (`docs/RESEARCH-FEATURES-2026-10.md`) kalan yalnız C17 TR/dil seçici (K-022 ile kısmen). C2'nin Explorer'da gerçekten gizlediği elle doğrulanmalı. Açık issue: #21. Worktree başına ~15 GB `target/` birikir; disk dolunca eski worktree'lerin `src-tauri/target`'ı silinir (yeni oturum worktree'si ≥5.4 GB boş ister).
2. Codex for OSS başvurusu (demo GIF hazır, #115).
3. winget 0.2.0 birleşince 0.4.0 manifestini (repoda hazır) microsoft/winget-pkgs'e gönder (`packaging/winget/README.md`).
4. Kod imzalama (SmartScreen uyarısı + VT ML tespitleri): SignPath Foundation 2026-10-15 reddetti (görünürlük yetersiz, K-014). Şimdilik imzasız; görünürlük artınca yeniden başvur. Ücretli/ucuz yol: Certum Open Source sertifikası.

## Bilinen tuzaklar
- 5 symlink testi lokal Developer Mode olmadan düşer (CI'da geçer). EFS testleri `WINKNIFE_EFS_TESTS=1` ile.
- CI yalnız main'e push/PR'da tetiklenir; başka base'li PR için `gh workflow run ci.yml --ref <dal>`.
- `src-tauri/target-review/` (eski derleme önbelleği) ve `chatgpt Pocketknife addon.txt` kullanıcının; dokunma.
- Repo PUBLIC: Armada'nın otomatik "Son oturum" bloğu (`oto:başla` işaretli) kullanıcı isteklerini DEVIR'e yazar; commit'e girmeden sil.
- e2e `duplicates: Stop cancels the scan` flake'inin kökü bulundu ve düzeltildi (2026-10-20, QA/QC): kökte uygun aday yokken backend progress olayı yollamıyor, UI ertelenen Stop'u gönderemiyordu (docs/QA-CHECKLIST.md B3, B8).
- `quarantine_batch_items` artık geri yüklenen satırları dönmez (#88). e2e işi ~26 dk, sınır 60 dk.
- winget kurulumu CI'da SmartScreen'de takılır (imzasız installer); CI yalnız `winget validate` yapar.
