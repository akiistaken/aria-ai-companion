//! COM apartment setup. WASAPI and the WinRT media APIs need COM initialised on the calling
//! thread; every worker joins the multithreaded apartment once.

use std::cell::Cell;

thread_local! {
    static READY: Cell<bool> = const { Cell::new(false) };
}

/// Joins the MTA on this thread, once. Harmless if the thread already has an apartment.
pub fn ensure_mta() {
    READY.with(|ready| {
        if !ready.get() {
            // An `RPC_E_CHANGED_MODE` error means the thread already has a (UI, STA) apartment,
            // which works for these APIs too; nothing to do about it either way.
            let _ = wasapi::initialize_mta();
            ready.set(true);
        }
    });
}
