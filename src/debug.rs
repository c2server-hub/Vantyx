// debug — le cœur du contrat: chaque lecture mémoire s'imprime avec son
// adresse ABSOLUE, sa valeur et un verdict. F1 (ou panneau GUI) bascule le
// mode verbeux en live.
//
// JOURNAL PARTAGÉ: emit() imprime dans la console ET bufferise dans un anneau
// (600 lignes) que le panneau GUI draine à chaque boucle → le panneau est un
// miroir exact de la console, y compris les lignes émises avant sa création.
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(true);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static FRAME: AtomicU64 = AtomicU64::new(0);
static RPM_FAIL: AtomicU64 = AtomicU64::new(0);

pub fn init(verbose: bool, quiet: bool) {
    ENABLED.store(!quiet, Ordering::Relaxed);
    VERBOSE.store(verbose, Ordering::Relaxed);
}
pub fn enabled() -> bool { ENABLED.load(Ordering::Relaxed) }
pub fn verbose() -> bool { VERBOSE.load(Ordering::Relaxed) }
/// le menu est la source de vérité (opts.verbose) — la boucle overlay
/// resynchronise l'atomique à chaque itération via ça.
pub fn set_verbose(v: bool) { VERBOSE.store(v, Ordering::Relaxed); }
pub fn next_frame() -> u64 { FRAME.fetch_add(1, Ordering::Relaxed) + 1 }

// vrai ~1×/seconde — pour les warns qui sinon tireraient à CHAQUE frame de la
// boucle (interval 1 ms → ~1000 lignes/s dans la console): offset stale,
// BindDC, EndDraw, etc. Le RPM-fail a toujours eu son throttle; eux non.
static T0: OnceLock<Instant> = OnceLock::new();
static LAST_SEC: AtomicU64 = AtomicU64::new(0);
static THROTTLE_SEEDED: AtomicBool = AtomicBool::new(false);

pub fn once_per_sec() -> bool {
    // LA PREMIÈRE occurrence tire TOUJOURS — sinon le seed à sec==0
    // faisait des runs courts (< 1 s) muets sur CE QUI FAIT JUSTEMENT
    // ÉCHOUER le snapshot (bug constaté en dump: warns partis en silence).
    let sec = T0.get_or_init(Instant::now).elapsed().as_secs();
    if !THROTTLE_SEEDED.swap(true, Ordering::Relaxed) {
        return true;
    }
    LAST_SEC.fetch_max(sec, Ordering::Relaxed) != sec
}

// throttle: pas 500 lignes par seconde quand le process meurt
pub fn rpm_fail_tick() -> bool {
    let n = RPM_FAIL.fetch_add(1, Ordering::Relaxed) + 1;
    n == 1 || n % 500 == 0
}

// ── journal partagé console + panneau GUI ──────────────────────────────────
static LOG: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
const LOG_CAP: usize = 600;

fn logq() -> &'static Mutex<VecDeque<String>> {
    LOG.get_or_init(|| Mutex::new(VecDeque::new()))
}

/// LA fonction d'émission: console + tampon pour le panneau.
/// (paramètre String concret — un générique Into<String> rendrait les
///  `.into()` des appels ambigus: E0283)
pub fn emit(s: String) {
    println!("{s}");
    if let Ok(mut q) = logq().lock() {
        q.push_back(s);
        while q.len() > LOG_CAP {
            q.pop_front();
        }
    }
}

/// le panneau appelle ça chaque boucle; renvoie les lignes apparues depuis.
pub fn drain_log() -> Vec<String> {
    logq().lock().map(|mut q| q.drain(..).collect()).unwrap_or_default()
}

#[macro_export]
macro_rules! dlog {
    ($($arg:tt)*) => {
        if crate::debug::enabled() { crate::debug::emit(format!("[ dbg] {}", format!($($arg)*))); }
    };
}

#[macro_export]
macro_rules! dwarn {
    ($($arg:tt)*) => {
        if crate::debug::enabled() { crate::debug::emit(format!("[warn ] {}", format!($($arg)*))); }
    };
}

// lecture mémoire tracée: nom du champ, adresse ABSOLUE, valeur.
// c'est exactement là que tu poses ton write si tu veux modifier le champ.
// ATTENTION: verbose-only — ces traces tirent par frame (~1000/s à 1 ms).
// Le dump ponctuel (F4 / bouton) donne le détail complet sans le déluge.
#[macro_export]
macro_rules! dmem {
    ($addr:expr, $name:expr, $val:expr) => {
        if crate::debug::verbose() {
            crate::debug::emit(format!("[ mem ] {} @ {:#018x} = {}", $name, $addr as u64, $val));
        }
    };
}
