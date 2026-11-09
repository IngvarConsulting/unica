//! `unica lsp`: дверь редактора в тот же демон, что обслуживает MCP.
//!
//! Тонкий frontend: читает LSP со stdio и передаёт его демону по маршруту
//! LSP протокола v5. Сессию открывает и держит одно соединение, по нему же
//! по порядку идут уведомления о документах. Каждый запрос исполняется
//! своим соединением на полосе LSP демона; отмена уходит отдельным
//! соединением. Возможности объявляет служба демона по своей готовности.

mod codec;
mod door;

use std::io;
use std::sync::Arc;
use unica_daemon_protocol::client_v5::V5DaemonClient;

/// Обслужить редактор по stdio через демон, к которому подключает `connect`.
/// Возвращает код выхода процесса.
pub fn run_stdio(connect: fn() -> Result<V5DaemonClient, String>) -> i32 {
    let client = match connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let stdin = io::stdin();
    // The main loop waits on the editor's input. A session the daemon no
    // longer has ends the process from the poller, so the editor restarts
    // the server instead of getting refusals forever.
    let session_lost: door::SessionLost = Arc::new(|| {
        eprintln!("unica lsp lost its daemon session");
        std::process::exit(1);
    });
    door::serve(stdin.lock(), io::stdout(), Arc::new(client), session_lost)
}

/// Внутренности двери для тестов `unica-daemon` с живым демоном.
#[cfg(feature = "test-support")]
pub mod test_support {
    pub use crate::door::*;

    /// Кадры LSP: чтение и запись сообщения.
    pub mod codec {
        pub use crate::codec::*;
    }
}
