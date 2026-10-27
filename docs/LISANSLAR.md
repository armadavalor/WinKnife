# Lisanslar ve varlık kaynağı
<!-- Dışarıdan gelen her varlık/paket için bir satır: font, ikon, ses, model, görsel, kod parçası. Yayından önce "Ticari?" sütunu boş kalmaz. -->
<!-- Rust crate'lerinin tam listesi Cargo.lock'ta, npm paketlerininki package-lock.json'da. Burada yalnız ana çatılar ve elle eklenen varlıklar. -->

| Varlık/paket | Kaynak | Lisans | Ticari? | Not |
|---|---|---|---|---|
| Logo + ikonlar | assets/logo.png (proje sahibi) | MIT (repo) | Evet | Çakı + Windows bayrağı, 2026-08-07 |
| Demo GIF | assets/demo.gif (kendi kaydımız, `demo.yml` + ffmpeg) | MIT (repo) | Evet | v0.2.0, CI runner'da sahte `C:\Users\Demo` profili, 2026-10-14 |
| Tauri v2 (crate + @tauri-apps/*) | tauri.app | MIT / Apache-2.0 | Evet | |
| Svelte 5 / SvelteKit | svelte.dev | MIT | Evet | |
| Tailwind CSS 4 | tailwindcss.com | MIT | Evet | |
| Lucide ikonları (vendored, `src/lib/icons.ts`) | lucide.dev, lucide-static 1.57.0 | ISC (power/chevron-right/info: Feather MIT) | Evet | 2026-10-15 |
| actions/attest-build-provenance@v3 | github.com/actions/attest-build-provenance | MIT | Evet | release.yml, yalnız CI adımı; 2026-10-24 |
| image 0.25 (Rust crate) | github.com/image-rs/image | MIT OR Apache-2.0 | Evet | C18 benzer resimler; yalnız jpeg/png/webp/gif özellikleri, 2026-10-26 |
| Remotion 4.0.539 (`remotion`, `@remotion/cli`) | remotion.dev | Remotion License: ücretsiz bireyler, ≤3 kişilik ekipler ve kâr amacı gütmeyenler için | Hayır (ücretsiz kapsam; ticari kullanımda ücretli) | `demo/` içinde, yalnız dev; uygulama ve Tauri paketine girmez; `free-license` anahtarı; 2026-10-27 |
| React 19.1.0 / react-dom (demo/) | react.dev | MIT | Evet | yalnız `demo/`, 2026-10-27 |
| demo/public/*.png ekran görüntüleri | WinKnife 0.4.0 CI kaydı (sahte `C:\Users\Demo` profili) | Proje lisansı | Evet | gerçek kullanıcı verisi yok; 2026-10-27 |
