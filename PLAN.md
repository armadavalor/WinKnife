# Proje Planı — Windows İsviçre Çakısı (kod adı: WinKnife)

> Açık kaynak, kâr amacı gütmeyen (Buy Me a Coffee), portfolyo değeri yüksek bir Windows yardımcı aracı.
> Tek cümlelik vizyon: **"Windows'un günlük sıkıcı zorluklarına tek pencereden, güvenli ve akıllı çözümler."**

---

## 1. Ürün Vizyonu

Sıradan bir cleaner değil, **modüler bir araç kutusu**. Temizlik ilk ve en güçlü "bıçak", ama mimari baştan çoklu araca göre kurulur. Her araç bağımsız bir modül; yeni modül eklemek çekirdeğe dokunmayı gerektirmez. GitHub yıldızı getiren şey de tam bu olur: "bir işi iyi yapan + genişleyebilen" araçlar.

**İlkeler:**

1. **Analiz önce, silme sonra.** Hiçbir şey kullanıcı onayı olmadan silinmez. Silinenler önce geri dönüşüme/karantinaya gider (geri alınabilirlik).
2. **Offline-first.** Çekirdek zekâ kural motorudur; internet ve API key opsiyoneldir.
3. **Şeffaflık.** Her öneri "neden?" açıklaması taşır. Açık kaynak olmanın gereği.
4. **Hafiflik.** Cleaner'ın kendisi şişko olamaz. Hedef: <15 MB kurulum, <50 MB RAM boşta.

---

## 2. Teknoloji Stack'i

| Katman | Seçim | Neden |
|---|---|---|
| Çekirdek/backend | **Rust (Tauri v2)** | Hızlı disk taraması, güvenli dosya işlemleri, küçük binary |
| UI | **Web (React/Svelte + TypeScript)** | Hızlı geliştirme, treemap gibi görselleştirmeler kolay |
| IPC | Tauri commands + events | Tarama ilerlemesini UI'a canlı akıtma |
| Depolama | SQLite (tarama geçmişi, karantina kaydı) + JSON kural dosyaları | Basit, taşınabilir |
| AI | Kural motoru (Rust) + opsiyonel LLM (kullanıcı API key: Claude/OpenAI/Ollama) | Hibrit |
| CI/CD | GitHub Actions → imzasız portable exe + installer (NSIS) | Otomatik release |

UI framework önerisi: **Svelte** (küçük bundle, Tauri ile popüler kombo) — React bilgin daha iyiyse React de olur, mimariyi etkilemez.

---

## 3. Mimari — Modüler "Çakı" Tasarımı

```
┌────────────────────────────────────────────┐
│                  UI (WebView)              │
│   Shell: sidebar'da araçlar, her araç      │
│   kendi sayfası (lazy-loaded)              │
├────────────────────────────────────────────┤
│              Tauri IPC Katmanı             │
├────────────────────────────────────────────┤
│                Core (Rust)                 │
│  ┌──────────┐ ┌──────────┐ ┌────────────┐  │
│  │ Scanner  │ │  Brain   │ │  Executor  │  │
│  │ (tarama) │ │ (analiz) │ │ (uygulama) │  │
│  └──────────┘ └──────────┘ └────────────┘  │
│  ┌──────────────────────────────────────┐  │
│  │        Module Registry (trait)       │  │
│  │  cleaner │ treemap │ dupes │ startup │  │
│  │  shutdown-timer │ ...gelecek araçlar │  │
│  └──────────────────────────────────────┘  │
└────────────────────────────────────────────┘
```

**Her modül aynı trait'i uygular:** `scan() → Vec<Finding>`, `analyze(findings) → Vec<Recommendation>`, `execute(approved) → Report`. Bu sayede:
- Yeni araç = yeni bir Rust dosyası + bir UI sayfası
- Topluluk katkısı kolay (open source için kritik)
- "Brain" tüm modüllerin bulgularını aynı risk diliyle skorlar

### 3.1 Brain (küçük beyin) — Hibrit Zekâ

**Katman 1 — Kural motoru (her zaman, offline):**
- Sinyaller: dosya yaşı, son erişim, boyut, uzantı, konum (bilinen temp/cache yolları), imza (hangi programa ait), çalışan süreç kilidi
- Bilinen-güvenli yol veritabanı: JSON kural dosyaları (`rules/*.json`) — topluluk PR'la genişletebilir
- Çıktı: her bulgu için 0–100 güven skoru → üç kova:
  - 🟢 **Güvenli** — temp, cache, çöp kutusu, eski loglar → "tek tıkla temizle"
  - 🟡 **Kararın gerekli** — eski indirmeler, büyük medya, kullanılmayan programlar → tek tek onay
  - 🔴 **SİLME (önerilmez)** — sistem dosyaları, belirsiz ama riskli görünenler → sadece bilgi, buton yok
- Sistem yolu koruması hard-coded: `C:\Windows`, `Program Files`, kullanıcı profil kökleri asla 🟢'e düşemez

**Katman 2 — LLM (opsiyonel):**
- Kullanıcı kendi API key'ini girer (Claude/OpenAI) veya lokal Ollama'ya bağlanır
- Görevleri: 🟡 kovadaki belirsiz dosyaları açıklama ("Bu klasör 2023'te kaldırdığın X oyununun artığı"), doğal dilde soru-cevap ("50GB nereye gitti?"), kural motorunun bilmediği klasörleri sınıflandırma
- **Asla dosya içeriği göndermez** — sadece yol/boyut/tarih metadata'sı, o da kullanıcı onayıyla. README'de net yazılır (gizlilik = güven = yıldız)

### 3.2 Güvenlik omurgası (yıldız getiren asıl şey)
- **Karantina:** Silinen her şey önce `%APPDATA%\<app>\quarantine`'e taşınır, 7 gün sonra gerçekten silinir. "Geri Al" butonu.
- **Dry-run raporu:** temizlik öncesi "şunları sileceğim, ~X GB kazanacaksın" özeti
- **İşlem günlüğü:** her execute SQLite'a loglanır

---

## 4. V1 Modülleri

| # | Modül | Özet | Zorluk |
|---|---|---|---|
| 1 | **Temizlik Çekirdeği** | Temp/cache/tarayıcı artıkları/çöp/log-dump taraması + 3 kovalı liste + karantinalı silme | Orta |
| 2 | **Disk Treemap** | "Yer nereye gitti?" görsel haritası; tıkla-in, oradan temizliğe gönder | Orta |
| 3 | **Duplicate Bulucu** | Boyut → kısmi hash → tam hash (blake3) aşamalı eşleştirme; hangisini tutacağını öner | Orta |
| 4 | **Startup Yöneticisi** | Registry + Startup klasörü + görev zamanlayıcı girişleri; etki tahmini + devre dışı bırak | Orta-Kolay |
| 5 | **Kapatma Zamanlayıcı** | "2 saat sonra / indirme bitince / şu saat" kapat-uyut-yeniden başlat | Kolay |

Modül 5 kolay ve sevimli — ilk PR/ilk demo videosu için ideal başlangıç.

### Gelecek modül fikirleri (backlog — ihtiyaçtan doğanlar)
Sağ-tık menüsü temizleyici, "bu dosyayı hangi program kilitledi?" çözücü, toplu dosya adlandırıcı, Windows bildirim/reklam kapatıcı (debloat-lite), pano geçmişi, ekran görüntüsü klasör düzenleyici, boş klasör avcısı, büyük dosya "arşivle" önerici (taşınabilir diske). Kural: her modül gerçek bir Windows can sıkıntısını çözmeli, ayar yığını olmamalı.

---

## 5. Yol Haritası

**Faz 0 — İskelet (1. hafta):** Tauri projesi, modül trait'i, UI shell (sidebar + boş sayfalar), CI. → İlk commit'ten itibaren derlenen exe.

**Faz 1 — İlk kan (2–3. hafta):** Kapatma zamanlayıcı (kolay zafer) + Temizlik çekirdeği kural motoru v0 (sadece 🟢 kova: temp/çöp/cache). Karantina sistemi.

**Faz 2 — Beyin (4–6. hafta):** Skorlama, 🟡/🔴 kovalar, kural JSON formatı, açıklamalı öneriler. Duplicate bulucu.

**Faz 3 — Görsellik (7–8. hafta):** Treemap, startup yöneticisi, UI cilası, koyu tema.

**Faz 4 — Yayın (9–10. hafta):** LLM opsiyonu, README + ekran görüntüleri + GIF demo, ilk GitHub Release, Reddit (r/software, r/windows) + Product Hunt tanıtımı.

---

## 6. Açık Kaynak / Portfolyo Stratejisi

- **Lisans:** MIT (katkı dostu) — kurallar veritabanı da repo içinde, PR'la büyür
- **README:** İngilizce ana + `README.tr.md`; hero GIF ilk 5 saniyede ne yaptığını göstermeli
- **Yıldız mıknatısları:** karantina/geri-al güvenliği, "AI ama offline-first" hikâyesi, tek exe portable sürüm, `good first issue` etiketleri
- **Güven:** VirusTotal linki release notlarında (cleaner'lara şüpheyle bakılır), imzalama sonra düşünülür
- Buy Me a Coffee linki README + uygulama "Hakkında" sayfasında, asla popup değil

---

## 7. İsim Adayları (İsviçre çakısı ilhamı)

| İsim | Mantık |
|---|---|
| **WinBlade** | Windows + çakı bıçağı; kısa, akılda kalıcı |
| **Multitool** / **WinMultitool** | Doğrudan konsept; jenerik olma riski var |
| **Sackmesser** | İsviçre Almancası "çakı"; egzotik ama telaffuz zor |
| **Pocketknife** | Sade, sıcak; "PC'nin cep çakısı" |
| **Winnife** (Win + knife) | Uydurma ama benzersiz, domain/repo bulunması kolay |
| **Çakı** / **Chaki** | Türkçe kimlik; uluslararası okunuşu kolay |
| **SwissWin** | Konsepti direkt söylüyor |
| **Blade&Board** | Çakı + pano; iki kelimeli, marka gibi |

Önerim kısa liste: **WinBlade**, **Chaki**, **Winnife**. (GitHub'da benzersizlik kontrolünü isim seçilince yaparız.)

**Karar (2026-08-06):** İsim **WinKnife** olarak kesinleşti. GitHub deposu:
`armadavalor/WinKnife` (public, MIT).

---

## 8. İlk Adım (onayınla)

1. `T:\Claude\Cleanify` içinde Tauri v2 iskeletini kur (Rust + Svelte/React)
2. Modül trait'i + registry + UI shell
3. Kapatma zamanlayıcıyı uçtan uca çalışır hale getir → ilk çalışan demo
