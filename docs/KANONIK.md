<!-- durum: aktif -->
# Kanonik yollar

Her varlığın TEK geçerli yeri. Burada olmayan logo, renk, şema, ana modül vb. için Claude tahmin etmez, sorar.
Bir şey değişince: eski dosya `_arsiv/`e, bu tablo güncellenir, KARARLAR'a satır düşülür (hepsi tek commit).

| Varlık | Geçerli yol | Not |
|---|---|---|
| Hafıza | docs/ | DEVIR, KARARLAR, KANONIK, DERSLER, LISANSLAR |
| Mimari + trait sözleşmesi | docs/ARCHITECTURE.md | Karantina §6 |
| Kod kuralları | docs/CONVENTIONS.md | |
| Faz brief'leri | docs/phases/PHASE-N-*.md | Son: PHASE-5-companion |
| Modül spec'leri | docs/modules/NN-*.md | Bekleyenler BACKLOG.md |
| Açık/çözülmüş mimar kararları | docs/DECISIONS-NEEDED.md | |
| Dev changelog / kullanıcı changelog | docs/CHANGELOG-DEV.md / CHANGELOG.md | |
| Logo kaynağı | assets/logo.png | GitHub varyantı assets/logo-gh.png |
| Uygulama ikonları | src-tauri/icons/ , static/favicon.png | `tauri icon assets/logo.png` ile üretilir, elle düzenlenmez |
| Temizlik kuralları | rules/*.json | Biçim docs/RULES-FORMAT.md |
| Sürüm yayın akışı | docs/RELEASING.md + .github/workflows/release.yml | |
| winget manifesti | packaging/winget/manifests/ (yeni sürüm adımları packaging/winget/README.md) | |
| Codex sınırları | AGENTS.md | |

## İstisna
<!-- Adı arşiv/kopya gibi görünen ama bilerek AKTİF yollar: "- `yol/` | neden". kontrol ve kapı bunları arşiv/geçici saymaz. -->
- `rules/windows-temp.json` | Windows TEMP klasörü temizlik kuralı; "temp" adı içeriği anlatır, geçici dosya değil

## Süreli
<!-- Biten/yenilenen şeyler: "- YYYY-MM-DD | ne | not". 30 gün kala oturum başında uyarı çıkar. -->
- 2027-01-14 | Geri yükleme tatbikatı | /temizlik tatbikat: _arsiv/ ve pivot tag'inden bir varlığı geri getir, sonra geri al
