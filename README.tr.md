<div align="center">

<img width="260" height="260" alt="WinKnife logosu" src="https://github.com/user-attachments/assets/02b97ebe-a96e-4508-9b20-208a2db247a1" />

# WinKnife

### Sürekli güncellenen 20+ Windows aracı. Hızlı, güvenilir, kullanımı kolay.

Diskleri temizleyin, yinelenen dosyaları bulun, başlangıç öğelerini düzenleyin, dosyaların kilidini açın, Windows'u özelleştirin, hata olursa tek tıkta geri alın.
Her bulgunun bir nedeni vardır. Kaldırılan her dosya önce karantinaya alınır.

[![CI](https://github.com/armadavalor/WinKnife/actions/workflows/ci.yml/badge.svg)](https://github.com/armadavalor/WinKnife/actions/workflows/ci.yml)
[![Son sürüm](https://img.shields.io/github/v/release/armadavalor/WinKnife)](https://github.com/armadavalor/WinKnife/releases/latest)
[![Lisans: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

[**İndir**](https://github.com/armadavalor/WinKnife/releases/latest) · [English](README.md)

[![15 saniyede WinKnife: gerekçeli tarama, karantina, geri yükleme, tüm araçlar](assets/demo/demo.gif)](assets/demo/demo.mp4)

</div>

**Durum:** Yirmi araç uygulanmış durumda; güncel sürüm v0.5.0. Arayüz İngilizce ve Türkçe.

| Araç | İşlev |
| --- | --- |
| Disk temizleyici | Önce tarar; nedenleri ve riskleri gösterir. Seçilen dosyaları karantinaya taşır, önceki taramanın özetini gösterir. |
| Aynı dosyaları bulma | İçeriği aynı dosyaları, aynı içerikli klasörleri ve benzer görselleri bulur; seçilen kopyalar onayla karantinaya alınır. |
| Disk haritası | Klasörlerin kullandığı alanı gösterir; alt klasörleri incelemeyi sağlar. |
| Başlangıç uygulamaları | Desteklenen başlangıç girdilerini inceleme, devre dışı bırakma ve geri açma. Etki puanı bir tahmindir. |
| Kapatma zamanlayıcısı | Kapatma, yeniden başlatma veya uyku; geri sayım, uyarılar ve iptal. |
| Boş klasörler | Hiçbir derinlikte dosya içermeyen klasörleri bulur; seçilenler karantinaya taşınır. |
| Dosya kilidi açıcı | Bir dosyayı hangi programın açık tuttuğunu gösterir; kapanmasını isteyebilir. |
| Uyanık tut | Bilgisayarı (isteğe bağlı ekranı da) 24 saate kadar uyanık tutar. Hiçbir şey kaydedilmez. |
| Geri yükleme noktası | Sistem Koruması durumunu gösterir, geri yükleme noktası oluşturur (UAC istemi). |
| Eski paneller | 22 klasik Windows aracını tek sayfadan açar; bulunmayanlar devre dışıdır. |
| Ayarlar (Tweaks) | Geçerli kullanıcı için 27 geri alınabilir Windows ayarı; eski değer saklanır, Geri al onu koyar. |
| Bağlam menüsü | Sağ tık menüsü girdilerini sahibi ve yayıncısıyla listeler; bir girdiyi geçerli kullanıcı için geri alınabilir şekilde kapatır. |
| Hosts dosyası | Hosts dosyasını görüntüler ve düzenler; her kayıttan önce yedek ve fark gösterir (UAC istemi). |
| Kurulu uygulamalar | Win32 ve Store uygulamalarını yayıncı ve imzacıyla listeler (salt okunur); Windows'un Uygulamalar ayarını açar. |
| Kaldırma kalıntıları | Hiçbir kurulu programın sahiplenmediği uygulama verisi klasörleri; seçilenler karantinaya taşınır. |
| Büyük dosya arşivleyici | Uzun süredir dokunulmamış büyük dosyaları seçtiğin klasöre ya da sürücüye taşır. |
| Zamanlı tarama | Uygulama açıkken salt okunur temizleyici taramasını zamanında çalıştırır; otomatik karantina varsayılan olarak kapalı. |
| Ortam değişkenleri | Kullanıcı ve sistem değişkenlerini listeler; kullanıcı değişkenlerini fark önizlemesi ve Geri al ile düzenler. |
| Ağ | Hangi işlemin hangi portu dinlediği ya da kullandığı; DNS önbelleğini temizleme. Gerisi salt okunur. |
| Onarım kiti | Windows'un kendi SFC ve DISM denetimleri, her biri tek tıkla; tam komut önce gösterilir. |

## Güvenlik ve gizlilik

Tarama salt okunurdur; temizlik öncesinde bulguları ve açıklamaları incelersin.
Normal dosya temizliği karantinadan geçer. Geri yükleme yalnızca dosya karantinada
kaldığı sürece mümkündür: süre ve boyut sınırları, uygulama açılışı dahil,
kalıcı temizliğe yol açabilir.

**Windows Geri Dönüşüm Kutusu'nu boşaltmak kalıcıdır** ve ayrıca onay ister.
Karantinadan geri alınamaz. Korunan sistem konumları için temizleme önerilmez.

Mevcut uygulama yerel kurallarla çalışır. İsteğe bağlı LLM desteği planlanmıştır,
henüz uygulanmamıştır. WinKnife bir kayıt defteri temizleyicisi, sürücü güncelleyici,
antivirüs ya da FPS artışı vaadi değildir.

## Geliştirme ve katkı

Windows, Rust stable/MSVC, Visual Studio C++ Build Tools, Node.js 24 ve WebView2
gerekir. Depo kökünde:

~~~powershell
npm ci
npm run tauri dev
npm run check
npm run test:ui
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri build -- --bundles nsis
~~~

Sembolik bağlantı güvenlik testleri Windows Geliştirici Modu veya yükseltilmiş
bir test oturumu ister. Ayrıntılar [yayın yönergesinde](docs/RELEASING.md).

`--dry-run` yalnızca gerçek kapatma/uyku işlemlerini engeller; dosya temizliği,
başlangıç değişiklikleri ve karantina temizliğini engellemez.

Taşınabilir ZIP, uygulama ile gerekli `rules/` klasörünü birlikte içerir.
Veriler Windows'un roaming uygulama verisindeki `com.winknife.app` klasöründe
kalır; ZIP kullanmak verileri USB belleğe taşımaz.

Küçük hata düzeltmeleri, testler, belgeler ve dikkatle incelenmiş kural paketi
katkıları değerlidir. [Katkı kurallarını](docs/CONVENTIONS.md),
[kural paketi biçimini](docs/RULES-FORMAT.md) ve ilgili
[modül belgesini](docs/modules/) okuyarak başla.

Derleme ayrıntıları [İngilizce README'de](README.md),
yayın kabul adımları [release belgesinde](docs/RELEASING.md).

MIT — [LICENSE](LICENSE).
