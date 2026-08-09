//! Yuk Dengeleyici: sistem darbogaza girdiginde cagrilari kuyruga alir
//! veya dagitir (doc S.2.2.A). Faz 1'de gercek kuyruklama yok -- sadece
//! cagri hacmini izleyen bir sayac; Faz 4+'ta mesaj kuyrugu (ring buffer)
//! ile gercek dagitima donusecek.

use core::sync::atomic::{AtomicU32, Ordering};

static CALL_COUNT: AtomicU32 = AtomicU32::new(0);

pub fn note_call(_vector: u8) {
    CALL_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[allow(dead_code)]
pub fn call_count() -> u32 {
    CALL_COUNT.load(Ordering::Relaxed)
}
