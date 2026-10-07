mod batch;
mod building_commands;
mod callback_binding;
mod callback_startup;
mod game_session;
mod game_session_diagnostics;
mod event_capture;
mod memory;
mod resolve_sender;
mod runtime;
mod visuals;
mod gui_capture;
mod alliance;
mod alliance_ui_probe;
mod alliance_dialog;
mod alliance_requests;
mod alliance_output_context;
mod alliance_output_diagnostics;
mod minimap_alliance_button;
mod alliance_diagnostics;
mod buffer_rewrite;

use std::ptr::null_mut;
use winapi::shared::minwindef::{BOOL, DWORD, FALSE, HINSTANCE, LPVOID, TRUE};
use winapi::um::handleapi::CloseHandle;
use winapi::um::libloaderapi::DisableThreadLibraryCalls;
use winapi::um::processthreadsapi::CreateThread;
use winapi::um::winnt::DLL_PROCESS_ATTACH;

/// Initialization is deferred until after the loader releases its lock.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(instance: HINSTANCE, reason: DWORD, _: LPVOID) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        unsafe {
            DisableThreadLibraryCalls(instance);
        }
        let thread = unsafe {
            CreateThread(
                null_mut(),
                0,
                Some(runtime::worker),
                instance.cast(),
                0,
                null_mut(),
            )
        };
        if thread.is_null() {
            return FALSE;
        }
        unsafe {
            CloseHandle(thread);
        }
    }
    TRUE
}
