// Vantyx.cc v0.8.2 — ESP + RAGE externe Counter-Strike 2 · Rust · windows-rs 0.58
//
// architecture: ReadProcessMemory (usermode, LECTURE SEULE) → walk de la
// liste d'entités → world→screen via dwViewMatrix → overlay layeré à ALPHA
// PAR PIXEL (UpdateLayeredWindow + Direct2D prémultiplié + DirectWrite),
// boîtes ESP et menu GLASS PREMIUM in-game (INSERT): catégories COMBAT /
// VISUEL / MISC / THÈME, fond animé (orbes + balayage), studio de couleurs
// RGB live (thème CUSTOM persisté), debug complet en console.
//
// v0.6 — CORRECTIFS (les 4 bugs):
//   · aimbot     : lecture d'angles convention a2x DIRECTE (le pin dwCSGOInput
//                  faux décalait la lecture → vue périmée → visée morte)
//   · bomb timer : dwGameRules/dwPlantedC4 re-pinés (le timer ne s'armait
//                  JAMAIS) + décompte horloge SERVEUR (curtime sondé GlobalVars)
//   · weapon ESP : filtre m_hOwnerEntity — les armes possédées (tenues /
//                  inventaire) ne s'affichent plus → fini les endroits fantômes
//   · sens auto  : dwSensitivity re-piné → conversion degrés→counts exacte
//
// v0.6 — AJOUTS:
//   · anti screen-share TOGGABLE: case au menu (MISC) + F10 en direct
//   · launcher animé (crate vantyx-launcher): orbes, vérifications LED,
//     bouton LANCER avec progression → spawn vantyx.exe
//   · menu premium: 4 catégories, orbes animés, flashs de clic, pastille
//     état, studio de couleurs 6 rôles RGB live persisté dans vantyx.ini
//
// build (Windows x64, workspace):
//   cargo build --release              → vantyx.exe + launcher (même dossier)
//   cargo build --release -p vantyx-launcher → le launcher seul
//
// lancement:
//   launcher.exe               (recommandé — carte animée + vérifications)
//   vantyx.exe                 ESP + menu glass (INSERT) + console debug
//   vantyx.exe --dump          dump des adresses + entités en console, sans overlay
//   vantyx.exe --teammates     affiche aussi les coéquipiers
//   vantyx.exe --panel         panneau diagnostic Win32 dès le boot
//   vantyx.exe --verbose       debug verbeux dès le départ (sinon F1 en live)
//   vantyx.exe --quiet         console quasi muette
//   vantyx.exe --interval-ms 2 cadence de boucle (défaut 1 ms, aussi réglable au menu)
//
// touches: INSERT menu · END quitter · F10 anti screen-share · F1 debug
//          verbeux · F2 coéquipiers · F4 dump ponctuel
//
// OFFSETS: CS2 patch toutes les ~2 semaines et les offsets bougent — la
// validation debug nomme le champ fautif; re-pointe depuis un dumper à jour
// (offsets.rs documente la re-dérivation manuelle au x64dbg si tu préfères).

mod aimbot;
mod aimcore;
mod debug;
mod entity;
mod gui;
mod math;
mod mem;
mod menu;
mod modules;
mod offsets;
mod overlay;
mod theme;
mod thirdperson;
mod world;

use std::time::Duration;

pub const GAME_PROCESS: &str = "cs2.exe";
pub const GAME_WINDOW: &str = "Counter-Strike 2";

struct Args {
    dump: bool,
    verbose: bool,
    quiet: bool,
    teammates: bool,
    panel: bool,
    interval_ms: Option<u64>,
}

fn parse_args() -> Args {
    let mut a = Args { dump: false, verbose: false, quiet: false, teammates: false, panel: false, interval_ms: None };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dump" => a.dump = true,
            "--verbose" | "-v" => a.verbose = true,
            "--quiet" | "-q" => a.quiet = true,
            "--teammates" => a.teammates = true,
            "--panel" => a.panel = true,
            "--interval-ms" => {
                if let Some(v) = it.next() {
                    a.interval_ms = v.parse().ok();
                }
            }
            "--help" | "-h" => {
                println!("vantyx [--dump] [--verbose] [--quiet] [--teammates] [--panel] [--interval-ms N]");
                std::process::exit(0);
            }
            _ => {}
        }
    }
    a
}

fn main() {
    let args = parse_args();
    debug::init(args.verbose, args.quiet);
    install_panic_hook();

    debug::emit("Vantyx.cc v0.9.0 — LE TRIO PRÉCISION/CAMÉRA: (1) AIMBOT v0.9: VERROU à hystérésis 30 % + FOV de grâce 1.5× + 150 ms de survie — le flip-flop entre deux têtes d'un groupe est MORT (le fix \"il stresse en groupe\") · PRÉDICTION v·lead (lerp + ½ tick + âge snapshot + réglage) — les cibles qui strafent se prennent la balle DANS la tête, plus derrière l'oreille · GAIN ADAPTATIF 1→4 + plafond de counts 40→240 selon l'erreur — acquisition ~4× plus rapide, précision finale inchangée (deadzone 0.012° + carry). (2) SPINBOT GHOST (mode 4): write-restore synchronisé frame — le SERVEUR voit l'angle spunné, le RENDU retrouve ta vraie vue: ta caméra reste jouable pendant l'anti-aim, et l'angle est TOUJOURS restauré avant un cède. (3) CAMÉRA 3E PERSONNE (onglet RAGE): flag m_bCameraInThirdPerson ré-écrit à ~1 kHz + patch moteur opt-in (scan signature) — tu vois ton perso bouger, combo parfait avec GHOST.".into());
    debug::emit("INSERT ouvre le menu (verre translucide + orbes animés) · END quitter · F10 anti screen-share · F1/F2/F4 aussi au menu".into());
    debug::emit(String::new());

    loop {
        debug::emit(format!("[*] recherche de {GAME_PROCESS}…"));
        let pid = wait_for_process(GAME_PROCESS);

        let mut mem = match mem::Mem::attach(pid) {
            Ok(m) => m,
            Err(e) => {
                debug::emit(format!("[!] attach impossible: {e}"));
                std::process::exit(1);
            }
        };
        let (offs, report) = offsets::Offsets::load();
        dlog!("[offs] source: {}", report.source);
        offs.print_resolution(&mem);
        offs.check_build(&mem);

        if args.dump {
            entity::dump_and_exit(&mem, &offs);
            return;
        }

        match overlay::run(
            &mut mem,
            &offs,
            overlay::Config {
                panel: args.panel,
                teammates: args.teammates,
                verbose: args.verbose,
                interval_ms: args.interval_ms,
            },
        ) {
            overlay::ExitCode::GameClosed => {
                debug::emit(format!("[!] {GAME_PROCESS} fermé — nouvelle tentative d'attach…"));
                debug::emit(String::new());
                continue; // le jeu est relancé → on se rattache
            }
            overlay::ExitCode::UserQuit => break,
            overlay::ExitCode::Fatal(e) => {
                debug::emit(format!("[!] initialisation impossible: {e}"));
                std::process::exit(1);
            }
        }
    }
    debug::emit("[*] arrêt propre.".into());
}

fn wait_for_process(name: &str) -> u32 {
    loop {
        if let Some(pid) = mem::find_pid(name) {
            debug::emit(format!("[+] {name} trouvé — pid {pid}"));
            return pid;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        println!("[ !! ] PANIC: {info}");
        default(info);
    }));
}
