// mem — attach cs2.exe, bases de modules, RPM tracé + WPM (v0.8: DUAL-HANDLE).
// v0.8 — VOIE B GATED ("RAGE"): le projet redevient LECTURE SEULE au boot.
//   · attach() ouvre VM_READ | VM_QUERY uniquement — profil process lecteur,
//     indistinguable d'un outil d'analyse tant que le mode RAGE est OFF.
//   · open_write() ouvre un SECOND handle VM_WRITE | VM_OPERATION quand le
//     mode RAGE est activé au menu; close_write() le ferme immédiatement au
//     retour en LEGIT — jamais de handle RW qui dort entre deux sessions.
//   · write() est un no-op propre (false) sans handle RW — le spinbot voit
//     ses écritures échouer silencieusement, sans panic ni spam de log.
// Tant que RAGE est OFF: ESP/aimbot/triggerbot/RCS/flick marchent (lecture +
// SendInput), le scan de handles ne voit qu'un lecteur sur cs2.exe.
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
};

use crate::debug;
use crate::{dlog, dwarn};

#[derive(Clone, Copy, Debug)]
pub struct ModuleInfo {
    pub base: u64,
    pub size: u32,
}

// pid/engine: utiles au re-attach et au debug — conservés même si non lus
#[allow(dead_code)]
pub struct Mem {
    pub proc: HANDLE,
    pub pid: u32,
    pub client: ModuleInfo, // signatures: entity list / view matrix / local pawn
    pub engine: ModuleInfo, // info seulement (debug / usage futur)
    /// v0.8 — handle ÉCRITURE optionnel (mode RAGE). None = LEGIT read-only.
    wproc: Option<HANDLE>,
    /// trace "handle RW refusé" une seule fois par session (pas de spam)
    warned_no_w: std::cell::Cell<bool>,
}

fn wide_to_string(w: &[u16]) -> String {
    let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..end])
}

pub fn find_pid(name: &str) -> Option<u32> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        if Process32FirstW(snap, &mut pe).is_ok() {
            loop {
                let pn = wide_to_string(&pe.szExeFile);
                if pn.eq_ignore_ascii_case(name) {
                    found = Some(pe.th32ProcessID);
                    break;
                }
                if Process32NextW(snap, &mut pe).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

pub fn find_module(pid: u32, want: &str) -> Option<ModuleInfo> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid).ok()?;
        let mut me: MODULEENTRY32W = std::mem::zeroed();
        me.dwSize = size_of::<MODULEENTRY32W>() as u32;
        let mut found = None;
        if Module32FirstW(snap, &mut me).is_ok() {
            loop {
                let mn = wide_to_string(&me.szModule);
                if mn.eq_ignore_ascii_case(want) {
                    found = Some(ModuleInfo {
                        base: me.modBaseAddr as u64,
                        size: me.modBaseSize,
                    });
                    break;
                }
                if Module32NextW(snap, &mut me).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        found
    }
}

impl Mem {
    pub fn attach(pid: u32) -> windows::core::Result<Self> {
        unsafe {
            // v0.8 — LECTURE SEULE au boot: le handle RW n'existe QUE si le mode
            // RAGE est explicitement activé (open_write ci-dessous).
            let proc = OpenProcess(
                PROCESS_VM_READ | PROCESS_QUERY_INFORMATION,
                false,
                pid,
            )?;
            dlog!("[*] OpenProcess(pid={pid}) → handle {:#x} (LECTURE SEULE — mode LEGIT)", proc.0 as usize);

            let client = find_module(pid, "client.dll")
                .ok_or_else(|| windows::core::Error::from_win32())?;
            let engine = find_module(pid, "engine2.dll")
                .ok_or_else(|| windows::core::Error::from_win32())?;

            debug::emit(format!("[ mem ] client.dll  base={:#018x} size={:#x}", client.base, client.size));
            debug::emit(format!("[ mem ] engine2.dll base={:#018x} size={:#x}", engine.base, engine.size));
            debug::emit("[ mem ] TOUTES les adresses de signature = client.dll base + offset".into());

            Ok(Mem { proc, pid, client, engine, wproc: None, warned_no_w: std::cell::Cell::new(false) })
        }
    }

    // ═══ v0.8 — VOIE B GATED: cycle de vie du handle ÉCRITURE ═════════════
    /// mode RAGE ON → ouvre le handle RW. Idempotent. Retourne false si le
    /// jeu refuse l'ouverture (droits/anticheat) — le mode reste dégradé
    /// lecture seule, Rien ne casse.
    pub fn open_write(&mut self) -> bool {
        if self.wproc.is_some() {
            return true;
        }
        unsafe {
            match OpenProcess(PROCESS_VM_WRITE | PROCESS_VM_OPERATION, false, self.pid) {
                Ok(h) => {
                    self.wproc = Some(h);
                    dlog!("[*] RAGE: handle ÉCRITURE ouvert ({:#x}) — WPM autorisé", h.0 as usize);
                    true
                }
                Err(e) => {
                    dwarn!("RAGE: OpenProcess WRITE refusé ({e}) — écriture mémoire indisponible");
                    false
                }
            }
        }
    }

    /// retour en LEGIT → ferme le handle RW immédiatement (jamais de grâce).
    pub fn close_write(&mut self) {
        if let Some(h) = self.wproc.take() {
            unsafe { let _ = CloseHandle(h); }
            dlog!("[*] LEGIT: handle ÉCRITURE fermé — retour lecture seule");
        }
    }

    /// état du handle RW (bandeau menu: LEGIT vert / RAGE rouge)
    pub fn write_open(&self) -> bool {
        self.wproc.is_some()
    }

    // lecture typée — échec tracé + compté (détection process mort en amont)
    pub fn read<T: Copy>(&self, addr: u64) -> Option<T> {
        let mut out = std::mem::MaybeUninit::<T>::zeroed();
        let ok = unsafe {
            ReadProcessMemory(
                self.proc,
                addr as *const _,
                out.as_mut_ptr() as *const _ as *mut _,
                size_of::<T>(),
                None,
            )
            .is_ok()
        };
        if ok {
            Some(unsafe { out.assume_init() })
        } else {
            if crate::debug::rpm_fail_tick() {
                dwarn!("RPM échoué @ {:#018x} ({} octets) — process fermé ou page protégée ?", addr, size_of::<T>());
            }
            None
        }
    }

    pub fn read_arr<T: Copy>(&self, addr: u64, buf: &mut [T]) -> bool {
        let ok = unsafe {
            ReadProcessMemory(
                self.proc,
                addr as *const _,
                buf.as_mut_ptr() as *const _ as *mut _,
                size_of::<T>() * buf.len(),
                None,
            )
            .is_ok()
        };
        if !ok && crate::debug::rpm_fail_tick() {
            dwarn!("RPM échoué @ {:#018x} (arr {} × {}o)", addr, buf.len(), size_of::<T>());
        }
        ok
    }

    // chaîne ASCII inline (noms de joueurs selon les dumps: char* OU inline)
    pub fn read_ascii(&self, addr: u64, max: usize) -> Option<String> {
        let mut b = vec![0u8; max];
        if !self.read_arr(addr, &mut b) {
            return None;
        }
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        b.truncate(end);
        Some(String::from_utf8_lossy(&b).trim().to_string())
    }

    // ═══ v0.7 — ÉCRITURE typée (WPM) · v0.8 GATED SUR LE HANDLE RAGE ═══════
    // No-op propre (false) tant que open_write() n'a pas été appelé: le
    // spinbot en mode LEGIT échoue silencieusement, sans panic. Une écriture
    // ratée = frame de spin perdue, rien de plus.
    pub fn write<T: Copy>(&self, addr: u64, val: &T) -> bool {
        let Some(h) = self.wproc else {
            if !self.warned_no_w.get() {
                // une ligne par session — assez pour diagnostiquer un rage OFF
                self.warned_no_w.set(true);
                dwarn!("WPM ignoré: mode LEGIT (handle écriture fermé) — active le mode RAGE au menu");
            }
            return false;
        };
        let ok = unsafe {
            WriteProcessMemory(
                h,
                addr as *const _,
                val as *const _ as *const _,
                size_of::<T>(),
                None,
            )
            .is_ok()
        };
        if !ok && crate::debug::rpm_fail_tick() {
            dwarn!("WPM échoué @ {:#018x} ({} octets) — page protégée / handle?", addr, size_of::<T>());
        }
        ok
    }

    // ═══ v0.9 — OCTETS BRUTS (patch moteur: troisième personne) ═══════════
    /// lecture d'un bloc d'octets (sauvegarde des originaux avant patch)
    pub fn read_bytes(&self, addr: u64, buf: &mut [u8]) -> bool {
        let ok = unsafe {
            ReadProcessMemory(
                self.proc,
                addr as *const _,
                buf.as_mut_ptr() as *const _ as *mut _,
                buf.len(),
                None,
            )
            .is_ok()
        };
        if !ok && crate::debug::rpm_fail_tick() {
            dwarn!("RPM échoué @ {:#018x} ({} octets)", addr, buf.len());
        }
        ok
    }

    /// écriture d'un bloc d'octets — GATED handle RAGE comme write<T>().
    /// Usage: patch du jump gate troisième personne (voir thirdperson.rs).
    /// TOUJOURS read_bytes() avant pour garder les originaux (restauration).
    pub fn write_bytes(&self, addr: u64, bytes: &[u8]) -> bool {
        let Some(h) = self.wproc else {
            if !self.warned_no_w.get() {
                self.warned_no_w.set(true);
                dwarn!("WPM ignoré: mode LEGIT (handle écriture fermé) — active le mode RAGE au menu");
            }
            return false;
        };
        let ok = unsafe {
            WriteProcessMemory(
                h,
                addr as *const _,
                bytes.as_ptr() as *const _,
                bytes.len(),
                None,
            )
            .is_ok()
        };
        if !ok && crate::debug::rpm_fail_tick() {
            dwarn!("WPM échoué @ {:#018x} ({} octets) — page protégée / handle?", addr, bytes.len());
        }
        ok
    }
}

impl Drop for Mem {
    fn drop(&mut self) {
        unsafe {
            if let Some(h) = self.wproc.take() {
                let _ = CloseHandle(h);
            }
            let _ = CloseHandle(self.proc);
        }
    }
}
