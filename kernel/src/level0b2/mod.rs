//! Level-0b2: Merkezi Denetleyici ve Gozetim Birimi (doc S.2.2.A).
//! "Trafik Polisi" -- tum interrupt/exception/syscall mantiksal olarak
//! once buraya dusermis gibi ele alinir: Dispatcher, State Monitor,
//! Load Balancer, Fallback Interface.

pub mod dispatcher;
pub mod fallback;
pub mod ipc;
pub mod load_balancer;
pub mod state_monitor;
