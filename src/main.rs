fn main() {
    #[cfg(feature = "ui")]
    run_ui();
}

#[cfg(feature = "ui")]
fn run_ui() {
    use std::rc::Rc;

    use musicplayer_rs::library::db::Db;
    use musicplayer_rs::library::track::Track;
    use musicplayer_rs::player::PlaybackState;
    use musicplayer_rs::player::PlayerHandle;
    use musicplayer_rs::player::rodio::RodioAudioBackend;
    use musicplayer_rs::ui;

    // Reads RUST_LOG; defaults to no output when unset so users see clean startup.
    tracing_subscriber::fmt::init();

    let db_path = data_dir().join("library.db");
    let db = match Db::open(&db_path) {
        Ok(db) => Rc::new(db),
        Err(e) => {
            tracing::error!("Failed to open database: {e}");
            return;
        }
    };

    let (state_tx, state_rx) = async_channel::unbounded::<PlaybackState>();
    let (queue_tx, queue_rx) = async_channel::unbounded::<Vec<Track>>();
    let player = PlayerHandle::launch(
        RodioAudioBackend::new,
        move |s| {
            let _ = state_tx.try_send(s);
        },
        move |tracks| {
            let _ = queue_tx.try_send(tracks);
        },
    );

    ui::run(db, db_path, player, state_rx, queue_rx);
}

#[cfg(feature = "ui")]
fn data_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share")
        });
    let dir = base.join("musicplayer-rs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}
