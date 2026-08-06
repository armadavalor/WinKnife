# Orkestra Rehberi (Babuş için — repoya girmez, Türkçe)

Opus 4.8'i nasıl süreceğin. Ben (Fable) şef/mimar; Opus işçi; sen yapımcı.

## Dosya haritası
- `CLAUDE.md` → Opus'un anayasası; oturum başında otomatik okur. Sert kurallar + "bitti" tanımı burada.
- `docs/ARCHITECTURE.md` + `CONVENTIONS.md` → tasarım ve üslup. Opus bunları **değiştirmez**, uygular.
- `docs/modules/*.md` → araç başına spec. `docs/phases/PHASE-N-*.md` → sipariş fişleri.
- `docs/RULES-FORMAT.md` → kural JSON formatı.

## Çalıştırma döngüsü (her faz için aynı)
1. Opus'a tek satır ver: **"PHASE-N brief'ini uygula: docs/phases/PHASE-N-....md"** — gerisi dokümanlarda. Uzun prompt yazma; spec'ler zaten prompt.
2. Faz bitince Opus'tan iste: "Acceptance listesini tek tek kanıtla (komut çıktısı/test sonucu ile)."
3. Bana dön: fazın diff özetini/CHANGELOG-DEV girişini bana göster, mimariden sapma var mı denetlerim. Özellikle: karantina dışı silme, `commands.rs`'e sızan iş mantığı, testsiz scoring değişikliği.
4. `docs/DECISIONS-NEEDED.md` birikmişse fazlar arasında birlikte cevaplarız — Opus'a cevapları tek mesajla geri ver.

## Faz sırası ve bekleme noktaları
PHASE-0 → 1 → 2 → 3 → 4. Faz atlatma yok; her fazın "Out of scope" listesi Opus'un ileri koşmasını engellemek için var, ısrar ederse hatırlat.

## Kırmızı çizgiler (denetlerken buna bak)
- Kullanıcı dosyasına doğrudan `remove` çağrısı → ret.
- Korumalı yol testleri silinmiş/gevşetilmiş → ret.
- LLM katmanına dosya içeriği giden payload → ret.
- "Reason" alanı boş öneri → ret.

## Fazlar bittikçe büyüme
Yeni araç fikri geldiğinde: önce `docs/modules/BACKLOG.md`'ye seed spec olarak ekletirsin (bana yazdır), tam spec'e terfi ettirmeden Opus'a implementasyon verme. Kabul kriteri: gerçek bir Windows can sıkıntısı + tek ekran + scan→review→execute kalıbına uyum.

## İsim
Karar verilince: `Cargo.toml`, `tauri.conf.json`, README, repo adı tek commit'te değişir — Phase 4'ten önce kararlaştır (GIF ve installer isimle üretilecek).
