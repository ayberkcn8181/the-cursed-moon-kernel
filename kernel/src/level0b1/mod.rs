//! Level-0b1: Uyumluluk ve Ceviri Katmani (doc S.2.2.B).
//!
//! Faz 1 kapsami dışında -- burada henuz POSIX/NT subsystem veya
//! ELF/PE Binary Loader yok. Level-0b2'nin dispatcher'i su an int 0x80'i
//! dogrudan karsiliyor; bu katman Faz 3'te (ELF/POSIX) devreye girecek.
