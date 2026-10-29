// mem — attach cs2.exe, bases de modules, ReadProcessMemory tracé.
// lecture seule (PROCESS_VM_READ): rien ne s'écrit dans le process ici.
use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, Process32FirstW, Process32NextW,
    MODULEENTRY32W, PROCESSENTRY32W, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};

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
            let proc = OpenProcess(PROCESS_VM_READ | PROCESS_QUERY_INFORMATION, false, pid)?;
            dlog!("[*] OpenProcess(pid={pid}) → handle {:#x}", proc.0 as usize);

            let client = find_module(pid, "client.dll")
                .ok_or_else(|| windows::core::Error::from_win32())?;
            let engine = find_module(pid, "engine2.dll")
                .ok_or_else(|| windows::core::Error::from_win32())?;

            debug::emit(format!("[ mem ] client.dll  base={:#018x} size={:#x}", client.base, client.size));
            debug::emit(format!("[ mem ] engine2.dll base={:#018x} size={:#x}", engine.base, engine.size));
            debug::emit("[ mem ] TOUTES les adresses de signature = client.dll base + offset".into());

            Ok(Mem { proc, pid, client, engine })
        }
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
}

impl Drop for Mem {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.proc); }
    }
}
