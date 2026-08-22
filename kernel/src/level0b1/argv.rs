//! Arguman vektoru: ayni bilgi, iki temsil.
//!
//! Bir programa arguman vermenin iki gelenegi var ve **ikisi de
//! cekirdegin kapisina kadar geliyor**:
//!
//! ```text
//!   POSIX    execve(yol, argv[], envp[])   -> ayrilmis bir DIZI
//!   Win32    CreateProcessA(.., lpCommandLine, ..) -> tek bir DIZE
//! ```
//!
//! Fark kozmetik degil. Bir dizide `"iki kelime"` **tek** bir elemandir
//! ve icindeki bosluk hicbir sey ifade etmez. Tek dizede ise bosluk
//! ayiricidir, o yuzden Windows'un alintilama kurallari vardir. Iki
//! yonu de kaybetmeden tasimak gerekiyor:
//!
//!   * bir PE, `CreateProcessA` ile bir **ELF** baslatabiliyor -- dize
//!     diziye cevrilmeli;
//!   * bir ELF, `execve` ile bir **PE** baslatabiliyor -- dizi dizeye.
//!
//! ## Cekirdegin ic bicimi
//!
//! Ikisinin ortasinda tek bir tasiyici var: elemanlarin **NUL ile
//! ayrildigi** duz bir blok.
//!
//! ```text
//!   "browse\0/boot/msg.txt\0iki kelime\0"
//! ```
//!
//! Neden bu bicim: bosluk ayirici olmadigi icin alintilama sorunu
//! **bir kez**, giriste cozuluyor. Onceki tasiyici bosluklu bir dizeydi
//! ve `build_posix_stack` onu `split_whitespace` ile boluyordu -- yani
//! `"iki kelime"` yazan bir cagiran iki arguman aliyordu. Blok bicimi bu
//! kaybi ortadan kaldiriyor.
//!
//! Blok **`argv[0]` dahil** tutulur. Gercek `execve`de `argv[0]` yolun
//! kendisi olmak zorunda degildir (busybox'in tek ikilide onlarca komut
//! sunmasi tam da bunu kullanir), o yuzden cagiranin verdigi deger
//! korunuyor.
//!
//! ## Alintilama kurallari
//!
//! `split` ve `join`, Windows'un `CommandLineToArgvW` kurallarini
//! uygular -- uydurma degil:
//!
//!   * Cift tirnak "tirnak icinde" durumunu **degistirir**; o durumdayken
//!     bosluk siradan bir karakterdir.
//!   * Bir tirnaktan onceki `2n` ters bolu -> `n` ters bolu + durum
//!     degisir.
//!   * `2n+1` ters bolu -> `n` ters bolu + **gercek** bir tirnak
//!     karakteri.
//!
//! Ters bolu yalnizca tirnaktan onceyken ozeldir; `C:\dizin\dosya` gibi
//! bir yol hicbir kacisa ugramaz. Bu, Windows'un yol ayiricisiyla kacis
//! karakterini ayni yapmasinin bilinen sonucudur.

/// Blok icindeki ayirici.
pub const SEP: u8 = 0;

/// Blogun elemanlari uzerinde gezinir.
///
/// Bos elemanlar atlanmaz: gercek bir `execve` bos bir arguman
/// gecirebilir ve onu yutmak bilgi kaybi olurdu. Yalnizca blogun
/// sonundaki kapanis NUL'u eleman sayilmaz.
pub fn iter(block: &str) -> impl Iterator<Item = &str> {
    let trimmed = block.strip_suffix('\0').unwrap_or(block);
    let empty = trimmed.is_empty();
    trimmed.split('\0').filter(move |_| !empty)
}

/// Blokta kac eleman var?
pub fn count(block: &str) -> usize {
    iter(block).count()
}

/// Bir Windows komut satirini bloga cevirir. Doner: yazilan bayt sayisi.
///
/// Cikti `out`a sigmazsa **kesilir**: yarim bir arguman uretmektense
/// eksik uretmek yeglenir, cunku yarim bir yol dosya sistemine
/// gonderilirdi.
pub fn split(line: &str, out: &mut [u8]) -> usize {
    let bytes = line.as_bytes();
    let mut i = 0usize;
    let mut written = 0usize;
    let mut in_quotes = false;
    let mut started = false;

    let mut push = |byte: u8, written: &mut usize| -> bool {
        if *written >= out.len() {
            return false;
        }
        out[*written] = byte;
        *written += 1;
        true
    };

    while i < bytes.len() {
        let c = bytes[i];

        if !in_quotes && (c == b' ' || c == b'\t') {
            if started {
                if !push(SEP, &mut written) {
                    break;
                }
                started = false;
            }
            i += 1;
            continue;
        }

        if c == b'\\' {
            // Ters bolu dizisi **yalnizca** bir tirnaga bakiyorsa
            // ozeldir; degilse oldugu gibi gecer.
            let mut slashes = 0usize;
            while i + slashes < bytes.len() && bytes[i + slashes] == b'\\' {
                slashes += 1;
            }
            let quote_follows = i + slashes < bytes.len() && bytes[i + slashes] == b'"';
            let emit = if quote_follows { slashes / 2 } else { slashes };
            for _ in 0..emit {
                if !push(b'\\', &mut written) {
                    return written;
                }
                started = true;
            }
            i += slashes;
            if quote_follows {
                if slashes % 2 == 1 {
                    // Tek sayida ters bolu: tirnak **karakterdir**.
                    if !push(b'"', &mut written) {
                        return written;
                    }
                    started = true;
                } else {
                    in_quotes = !in_quotes;
                    started = true;
                }
                i += 1;
            }
            continue;
        }

        if c == b'"' {
            in_quotes = !in_quotes;
            // Bos bir alintilamanin (`""`) bos bir arguman uretmesi
            // gerekiyor, o yuzden burada da "baslandi" isaretlenir.
            started = true;
            i += 1;
            continue;
        }

        if !push(c, &mut written) {
            return written;
        }
        started = true;
        i += 1;
    }

    if started {
        let _ = push(SEP, &mut written);
    }
    written
}

/// Bir arguman, komut satirinda alintilanmak zorunda mi?
fn needs_quotes(arg: &str) -> bool {
    arg.is_empty() || arg.bytes().any(|b| b == b' ' || b == b'\t' || b == b'"')
}

/// Blogu tek bir Windows komut satirina cevirir (`split`in tersi).
///
/// Alintilama, `split`in geri okuyabilecegi bicimde yapilir: bosluk ya
/// da tirnak iceren -- ve **bos** olan -- elemanlar tirnaga alinir,
/// tirnaktan onceki ters bolular ikilenir.
///
/// Doner: yazilan bayt sayisi (kapanis NUL'u haric).
pub fn join(block: &str, out: &mut [u8]) -> usize {
    let mut written = 0usize;
    let mut push = |byte: u8, written: &mut usize| -> bool {
        if *written >= out.len() {
            return false;
        }
        out[*written] = byte;
        *written += 1;
        true
    };

    for (index, arg) in iter(block).enumerate() {
        if index > 0 && !push(b' ', &mut written) {
            return written;
        }
        if !needs_quotes(arg) {
            for byte in arg.bytes() {
                if !push(byte, &mut written) {
                    return written;
                }
            }
            continue;
        }
        if !push(b'"', &mut written) {
            return written;
        }
        let bytes = arg.as_bytes();
        let mut i = 0usize;
        while i < bytes.len() {
            let mut slashes = 0usize;
            while i + slashes < bytes.len() && bytes[i + slashes] == b'\\' {
                slashes += 1;
            }
            i += slashes;
            if i == bytes.len() {
                // Sondaki ters bolular kapanis tirnagina bakiyor:
                // ikilenmezlerse tirnagi kacirmis olurlardi.
                slashes *= 2;
            } else if bytes[i] == b'"' {
                slashes = slashes * 2 + 1;
            }
            for _ in 0..slashes {
                if !push(b'\\', &mut written) {
                    return written;
                }
            }
            if i < bytes.len() {
                if !push(bytes[i], &mut written) {
                    return written;
                }
                i += 1;
            }
        }
        if !push(b'"', &mut written) {
            return written;
        }
    }
    written
}

/// Bir POSIX `argv[]` dizisini bloga cevirir.
///
/// Dizi kullanici alanindadir ve NULL ile biter; her isaretci ayri ayri
/// dogrulanir (bkz. `mmu::is_user_accessible`). Bozuk bir dizi `None`
/// dondurur -- yarim bir blok uretmek, cagiranin beklemedigi bir
/// programi baslatmak olurdu.
///
/// # Safety
/// Cagiran gorevin adres uzayi etkin olmalidir.
pub unsafe fn from_user_vector(list: usize, out: &mut [u8]) -> Option<usize> {
    use crate::level0a::core::mmu;

    let width = core::mem::size_of::<usize>();
    let mut written = 0usize;
    let mut index = 0usize;

    loop {
        let slot = list + index * width;
        if !mmu::is_user_accessible(slot) || !mmu::is_user_accessible(slot + width - 1) {
            return None;
        }
        let pointer = (slot as *const usize).read_unaligned();
        if pointer == 0 {
            return Some(written);
        }
        // Elemanin kendisi de kullaniciya ait: bayt bayt okunur ve her
        // sayfa siniri ayrica dogrulanir.
        let mut at = pointer;
        loop {
            if !mmu::is_user_accessible(at) {
                return None;
            }
            let byte = (at as *const u8).read();
            if written >= out.len() {
                // Blok doldu: buraya kadar uretilen kisim tutarli
                // (eleman sinirinda kesilmis degil), o yuzden
                // sonlandirip donuyoruz.
                return Some(written);
            }
            out[written] = byte;
            written += 1;
            if byte == 0 {
                break;
            }
            at += 1;
        }
        index += 1;
        // Cok uzun bir dizi cekirdegi oyalayabilir; blok zaten dolacagi
        // icin bu yalnizca ek bir emniyet.
        if index > 64 {
            return Some(written);
        }
    }
}
