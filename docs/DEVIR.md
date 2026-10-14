<!-- devir: 2026-10-14 22:00 | yazan: claude | commit: a916dea -->
# Devir notu
<!-- /devir üzerine yazar; elle düzeltirsen "yazan: kullanıcı" yap. ≤40 satır. Geçmiş git log'da, kararlar KARARLAR.md'de. -->

## Yön
Windows Swiss-army-knife (Tauri v2 + Svelte 5); analiz önce, onaysız silme yok, her şey geri alınabilir.
Hedef: Codex for OSS başvurusu (yıldız eşiği yok); öncelik yayın ve dağıtım.

## Şu an
- v0.2.0 yayınlandı (2026-10-13, latest). main = a916dea (#25 sürüm bump, #26 `%windir%` genişletme + sayı gruplama).
- Armada hafıza katmanı kuruldu (`chore/armada-hafiza`).

## Yarım kalan
- Manuel kabul: temiz VM kurulumu, VirusTotal taraması, demo GIF (senaryo `docs/DEMO-SCRIPT.md`).
- FindingList açılır reason ve v0.2.0 sayfaları canlıda görsel doğrulanmadı.

## Açık sorular
- TR arayüz: 117/352 anahtar, dil seçici yok (DECISIONS-NEEDED).
- Rust crate'leri 2.12'ye çekmek windows 0.62 + wry 0.57 getiriyor; npm + crate birlikte yükseltilmeli.

## Sıradaki adım
1. Temiz VM kabulü + VirusTotal + demo GIF, sonra Codex for OSS başvurusu.
2. Dağıtım: winget manifesti.
3. Eski uzak dallar (`feat/v0.2.0`, `feat/ui-declutter`) temizlenebilir.

## Bilinen tuzaklar
- 5 symlink testi lokal Developer Mode olmadan düşer (CI'da geçer). EFS testleri `WINKNIFE_EFS_TESTS=1` ile.
- CI yalnız main'e push/PR'da tetiklenir; başka base'li PR için `gh workflow run ci.yml --ref <dal>`.
- `src-tauri/target-review/` (eski derleme önbelleği) ve `chatgpt Pocketknife addon.txt` kullanıcının; dokunma.
