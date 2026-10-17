<!-- durum: aktif -->
# Kararlar

Append-only. Satır silinmez, düzeltilmez: fikir değişirse YENİ satır aç, `Yerine` sütununa eskisinin No'sunu yaz, eskisinin Durum'unu `GEÇERSİZ` yap.
Eşik: "Bir ay sonra bunu bilmezsem zarar görür müyüm?" Evetse yaz. Oturum başında son 15 AKTİF satır Claude'a otomatik verilir.
Ayrıntılı mimar gerekçeleri: `docs/DECISIONS-NEEDED.md` ("Architect resolutions").

| No | Tarih | Durum | Karar | Yerine | Kaynak |
|---|---|---|---|---|---|
| K-001 | 2026-10-14 | AKTİF | Proje hafızası docs/ altında: DEVIR, KARARLAR, KANONIK, DERSLER. Eski/terk edilen her şey `_arsiv/` altına. | - | kurulum |
| K-002 | 2026-08-06 | AKTİF | Ad WinKnife; kod adı `cleanify` koddan tamamen çıktı. Repo armadavalor/WinKnife, MIT. | - | kullanıcı |
| K-003 | 2026-08-06 | AKTİF | Her yol-yol karşılaştırması `core::path_compare` üzerinden (8.3, UNC, `\?\` aynı yolun farklı yazımı). | - | mimar, Faz 1 |
| K-004 | 2026-08-06 | AKTİF | Korumalı yol kapısı allow-list: yalnız kanonik `X:\...` yargılanır, kalanı "yargılanamaz". Enumerate-bad yaklaşımı kullanılmaz. | - | mimar, Faz 1 |
| K-005 | 2026-08-06 | AKTİF | `%WINDIR%` için istisna yok; `windows-update-leftovers` yalnız bilgilendirici, Windows Disk Temizleme'ye yönlendirir. | - | mimar |
| K-006 | 2026-08-06 | AKTİF | Executor üç kapı: nerede + kilitli mi + gerekçe hâlâ doğru mu. | - | mimar, Faz 2 |
| K-007 | 2026-10-11 | AKTİF | Geri dönüşüm kutusu boşaltma tek kalıcı silme istisnası: satır tiksiz başlar, backend ayrı onay bayrağı olmadan reddeder. | - | kullanıcı (a) |
| K-008 | 2026-10-12 | AKTİF | Startup "both kept": dürüst etiket; byte-eşit saklanan kopya karantinaya. EFS sınırı = uyarı yeterli. Downgrade azaltma won't-fix. | - | kullanıcı #21 |
| K-009 | 2026-10-12 | AKTİF | EFS testleri opt-in (`WINKNIFE_EFS_TESTS=1`, ci.yml + release.yml set eder). | - | kullanıcı |
| K-010 | 2026-10-13 | AKTİF | npm Tauri paketleri Rust crate minor'una kilitli (`~2.11.1`, plugin-dialog `~2.7.3`); yükseltme crate + npm birlikte. | - | CI kırığı 18020bd |
| K-011 | 2026-10-13 | AKTİF | İlk public sürüm 0.2.0 (0.1.0 hiç tag'lenmedi). Türkçe arayüz iddia edilmez (seçici yok). | - | kullanıcı |
| K-012 | 2026-10-13 | AKTİF | CI GitHub windows-latest'te kalır (public repo = ücretsiz); ev sunucusu runner kullanılmaz. | - | kullanıcı |
| K-013 | 2026-10-13 | GEÇERSİZ | Merge dışı her adım ajan için serbest; merge yalnız kullanıcı onayıyla. | - | kullanıcı |
| K-014 | 2026-10-15 | AKTİF | SignPath Foundation başvurusu reddedildi (yıldız/fork/dış referans yetersiz). Ücretli SignPath alınmaz; sürümler şimdilik imzasız, SmartScreen/VT için README'de dürüst not + SHA256. Görünürlük artınca yeniden başvurulur. | - | kullanıcı, SignPath e-postası |
| K-015 | 2026-10-18 | AKTİF | Merge de ajana bırakıldı: CI yeşil ve diff okunup doğrulandıktan sonra ajan kendisi merge eder. Sürüm tag'i/yayın, force-push, yıkıcı/geri alınamaz eylemler ve silme/karantina belirsizliği yine kullanıcıya. | K-013 | kullanıcı |
| K-016 | 2026-10-18 | AKTİF | Karantinasız direkt silme eklenmez; temizlik yalnız karantinaya taşır. Hızlandırılmış temizlik (kilit probu düzeltmesi, #50) denendikten sonra yeniden bakılır. | - | kullanıcı |
