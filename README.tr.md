# WinKnife

[English](README.md)

Disk temizliği, aynı dosyaları ve boş klasörleri bulma, disk haritası, başlangıç
uygulamaları, dosya kilidi açıcı, uyanık tutma, geri yükleme noktası ve kapatma
zamanlayıcısını tek pencerede sunan açık kaynak Windows aracı.

**Durum:** Dokuz araç uygulanmış durumda; ilk sürüm v0.2.0 hazırlanıyor.
Temiz bir Windows sanal makinesinde kurulum, temizleme, geri yükleme ve kaldırma
kabul testi henüz tamamlanmadı.

| Araç | İşlev |
| --- | --- |
| Disk temizleyici | Önce tarar; nedenleri ve riskleri gösterir. Seçilen dosyaları karantinaya taşır, önceki taramanın özetini gösterir. |
| Aynı dosyaları bulma | İçeriği aynı olan kopyaları bulur; seçilen kopyalar onayla karantinaya alınır. |
| Disk haritası | Klasörlerin kullandığı alanı gösterir; alt klasörleri incelemeyi sağlar. |
| Başlangıç uygulamaları | Desteklenen başlangıç girdilerini inceleme, devre dışı bırakma ve geri açma. Etki puanı bir tahmindir. |
| Kapatma zamanlayıcısı | Kapatma, yeniden başlatma veya uyku; geri sayım, uyarılar ve iptal. |
| Boş klasörler | Hiçbir derinlikte dosya içermeyen klasörleri bulur; seçilenler karantinaya taşınır. |
| Dosya kilidi açıcı | Bir dosyayı hangi programın açık tuttuğunu gösterir; kapanmasını isteyebilir. |
| Uyanık tut | Bilgisayarı (isteğe bağlı ekranı da) 24 saate kadar uyanık tutar. Hiçbir şey kaydedilmez. |
| Geri yükleme noktası | Sistem Koruması durumunu gösterir, geri yükleme noktası oluşturur (UAC istemi). |

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