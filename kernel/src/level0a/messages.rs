//! Level-0a'nin Level-0b2 mesaj kuyrugu tuketicisi (doc S.10, Faz 4+).
//!
//! Level-0b2 olaylari **uretildikleri yerde** kuyruga birakir; bunlarin
//! bir kismi kesme baglamindadir (istisna handler'i, yuk penceresi). Orada
//! uzun uzun konsola yazmak, hem kesme suresini uzatir hem de kilit
//! gerektiren bir surucuye kesme icinden girmek demektir.
//!
//! Tuketim bu yuzden **masaustu dongusunde**, kesmeler acikken yapilir:
//! olaylar burada gunluge yazilir ve kabugun `ipc` komutu icin gecmise
//! alinir.

use crate::level0b2::ipc::{self, Kind};

/// Tek bir turda islenecek en fazla mesaj. Sinir olmasa, kuyruk surekli
/// dolduran bir kaynak masaustu dongusunu (ve dolayisiyla kompozisyonu)
/// suresiz geciktirebilirdi.
const MAX_PER_ROUND: usize = 8;

/// Bekleyen mesajlari isler; islenen mesaj sayisini dondurur.
pub fn drain() -> usize {
    let mut handled = 0;

    while handled < MAX_PER_ROUND {
        let message = match ipc::poll() {
            Some(m) => m,
            None => break,
        };
        handled += 1;

        match message.kind {
            Kind::BootComplete => {
                crate::println!("[IPC] Level-0a acilisi tamam ({} servis).", message.a);
            }
            Kind::AppStart => {
                crate::println!("[IPC] uygulama basladi: gorev #{}", message.task);
            }
            Kind::AppExit => {
                crate::println!(
                    "[IPC] uygulama bitti: gorev #{} cikis={}",
                    message.task,
                    message.a
                );
            }
            Kind::Fault => {
                crate::println!(
                    "[IPC] istisna #{} ({}) gorev #{} adres=0x{:08x}",
                    message.a,
                    message.text,
                    message.task,
                    message.b
                );
            }
            Kind::Throttled => {
                // Yuk dengeleyicinin gorunur ciktisi: hangi gorev, ne kadar.
                crate::println!(
                    "[IPC] YUK: gorev #{} ({}) kotayi asti -- {} cagri/sn, toplam {} kisitlama",
                    message.task,
                    crate::level0a::core::scheduler::name_of(message.task),
                    message.a,
                    message.b
                );
            }
            Kind::HealthChange => {
                crate::println!("[IPC] saglik: {}", message.text);
            }
            Kind::Note => {
                crate::println!("[IPC] {}", message.text);
            }
        }
    }

    handled
}
