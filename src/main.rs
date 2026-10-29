// cs2-esp — ESP externe Counter-Strike 2 · Rust · windows-rs 0.58
//
// architecture: ReadProcessMemory (usermode, LECTURE SEULE) → walk de la
// liste d'entités → world→screen via dwViewMatrix → overlay layeré à ALPHA
// PAR PIXEL (UpdateLayeredWindow + Direct2D prémultiplié + DirectWrite),
// boîtes ESP et menu GLASS in-game (INSERT), debug complet en console.
//
// build (Windows x64):
//   cargo build --release
//
// lancement:
//   cs2-esp.exe                  ESP + menu glass (INSERT) + console debug
//   cs2-esp.exe --dump           dump des adresses + entités en console, sans overlay
//   cs2-esp.exe --teammates      affiche aussi les coéquipiers
//   cs2-esp.exe --panel          panneau diagnostic Win32 dès le boot
//   cs2-esp.exe --verbose        debug verbeux dès le départ (sinon F1 en live)
//   cs2-esp.exe --quiet          console quasi muette
//   cs2-esp.exe --interval-ms 2  cadence de boucle (défaut 1 ms, aussi réglable au menu)
//
// touches: INSERT menu · END quitter · F1 debug verbeux · F2 coéquipiers · F4 dump ponctuel
//
// OFFSETS: CS2 patch toutes les ~2 semaines et les offsets bougent — la
// validation debug nomme le champ fautif; re-pointe depuis un dumper à jour
// (offsets.rs documente la re-dérivation manuelle au x64dbg si tu préfères).

mod debug;
mod entity;
mod gui;
mod math;
mod mem;
mod menu;
mod offsets;
mod overlay;

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
                println!("cs2-esp [--dump] [--verbose] [--quiet] [--teammates] [--panel] [--interval-ms N]");
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

    debug::emit("cs2-esp v0.3.0 — ESP + squelette + radar + triggerbot · Space Grotesk".into());
    debug::emit("INSERT ouvre le menu (verre translucide, tout est cliquable) · END quitter · F1/F2/F4 aussi au menu".into());
    debug::emit(String::new());

    loop {
        debug::emit(format!("[*] recherche de {GAME_PROCESS}…"));
        let pid = wait_for_process(GAME_PROCESS);

        let mem = match mem::Mem::attach(pid) {
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
            &mem,
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
