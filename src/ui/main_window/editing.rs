use std::path::PathBuf;
use std::rc::Rc;

use async_channel::Sender;
use gtk4::ApplicationWindow;
use gtk4::Label;

use crate::library::album::ArtKey;
use crate::library::db::Db;
use crate::library::filter::LibraryFilter;
use crate::library::metadata_edit;
use crate::library::metadata_edit::TrackEdit;
use crate::library::track::AlbumArtData;
use crate::library::track::Track;
use crate::library::window_state::WindowMessage;
use crate::ui::album_grid::AlbumGrid;
use crate::ui::edit_dialog;
use crate::ui::library_view::LibraryView;

/// Everything opening an edit dialog needs, shared by every enqueue/edit
/// wiring site — replaces five near-identical clone-and-wire closures that
/// each cloned `db_path`/`window`/`status`/`tx` by hand. Art is looked up by
/// the caller, not here: the album cover menu wants the album's
/// representative cover (`Db::art_for`), while a row menu wants the clicked
/// track's own embedded art (`Db::art_for_track`) — a difference this struct
/// has no way to know from the tracks alone.
pub(super) struct EditRequester {
    db_path: PathBuf,
    window: ApplicationWindow,
    status: Label,
    tx: Sender<WindowMessage>,
}

impl EditRequester {
    pub(super) fn new(
        db_path: PathBuf,
        window: ApplicationWindow,
        status: Label,
        tx: Sender<WindowMessage>,
    ) -> Self {
        Self {
            db_path,
            window,
            status,
            tx,
        }
    }

    /// Opens a row-level request: the single-track dialog for exactly one
    /// track (an unselected row was right-clicked), the batch dialog
    /// otherwise (a multi-selection was) — mirrors which case produced
    /// `tracks` in `context_menu::track_actions`.
    fn edit_row(&self, mut tracks: Vec<Track>, art: Option<AlbumArtData>) {
        if tracks.len() == 1 {
            self.edit_track(tracks.remove(0), art);
        } else {
            self.edit_tracks(tracks, art);
        }
    }

    fn edit_track(&self, track: Track, art: Option<AlbumArtData>) {
        let db_path = self.db_path.clone();
        let status = self.status.clone();
        let tx = self.tx.clone();
        let track_for_save = track.clone();
        edit_dialog::open_track_editor(&self.window, track, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                vec![track_for_save.clone()],
                edit,
                art,
                tx.clone(),
            );
        });
    }

    /// Opens the batch dialog unconditionally — for an album-level request,
    /// which stays batch-shaped even for a single-track album.
    fn edit_tracks(&self, tracks: Vec<Track>, art: Option<AlbumArtData>) {
        let db_path = self.db_path.clone();
        let status = self.status.clone();
        let tx = self.tx.clone();
        let tracks_for_save = tracks.clone();
        edit_dialog::open_album_editor(&self.window, tracks, art, move |edit, art| {
            status.set_text("Saving…");
            spawn_edit_save(
                db_path.clone(),
                tracks_for_save.clone(),
                edit,
                art,
                tx.clone(),
            );
        });
    }
}

/// Spawns the background file+DB save, computes whether the edit can change
/// which cover an album shows (before the edit itself is dropped — see
/// `WindowMessage::EditSaved`'s doc comment), and bridges the single result
/// into the `WindowMessage` stream. Mirrors `start_scan`'s forwarding loop,
/// but for one value instead of a stream.
fn spawn_edit_save(
    db_path: PathBuf,
    tracks: Vec<Track>,
    edit: TrackEdit,
    art: Option<AlbumArtData>,
    tx: Sender<WindowMessage>,
) {
    let affects_art = art.is_some() || edit.affects_art_grouping();
    let rx = metadata_edit::spawn_save_edits(db_path, tracks, edit, art);
    glib::spawn_future_local(async move {
        if let Ok(outcome) = rx.recv().await {
            let _ = tx
                .send(WindowMessage::EditSaved {
                    affects_art,
                    outcome,
                })
                .await;
        }
    });
}

pub(super) fn wire_library_view(
    library_view: &LibraryView,
    db: &Rc<Db>,
    edit_requester: &Rc<EditRequester>,
    tx: &Sender<WindowMessage>,
) {
    let tx_activated = tx.clone();
    library_view.connect_track_activated(move |tracks, index| {
        if let Some(track) = tracks.get(index) {
            let _ = tx_activated.send_blocking(WindowMessage::Enqueue(vec![track.clone()], 0));
        }
    });

    let tx_enqueue = tx.clone();
    library_view.connect_track_enqueue(move |tracks| {
        let _ = tx_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let tx_resized = tx.clone();
    library_view.connect_column_resized(move |field, width| {
        let _ = tx_resized.send_blocking(WindowMessage::ColumnWidthChanged(field, width));
    });

    let db_edit = Rc::clone(db);
    let edit_requester = Rc::clone(edit_requester);
    library_view.connect_track_edit_requested(move |tracks| {
        let art = tracks.first().and_then(|t| {
            db_edit.art_for_track(t.id).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        edit_requester.edit_row(tracks, art);
    });
}

/// The album grid's track and cover-art providers (synchronous DB reads, not
/// messages — the grid needs the answer inline to render a drawer or a cover)
/// plus its activation/enqueue/edit callbacks (which do go through `WindowMessage`).
pub(super) fn wire_album_grid(
    album_grid: &AlbumGrid,
    db: &Rc<Db>,
    edit_requester: &Rc<EditRequester>,
    tx: &Sender<WindowMessage>,
) {
    let db_tracks = Rc::clone(db);
    album_grid.set_track_provider(move |summary| {
        db_tracks
            .tracks_for(&LibraryFilter::ByAlbum(summary.album.clone()))
            .unwrap_or_else(|e| {
                tracing::error!("Album query failed: {e}");
                Vec::new()
            })
    });

    let db_art = Rc::clone(db);
    album_grid.set_art_provider(move |key| {
        db_art.art_for(key).unwrap_or_else(|e| {
            tracing::error!("Art lookup failed: {e}");
            None
        })
    });

    let tx_activated = tx.clone();
    album_grid.connect_album_activated(move |tracks| {
        let _ = tx_activated.send_blocking(WindowMessage::Enqueue(tracks, 0));
    });

    let tx_track_activated = tx.clone();
    album_grid.connect_track_activated(move |tracks, index| {
        if let Some(track) = tracks.get(index) {
            let _ =
                tx_track_activated.send_blocking(WindowMessage::Enqueue(vec![track.clone()], 0));
        }
    });

    let tx_album_enqueue = tx.clone();
    album_grid.connect_album_enqueue(move |tracks| {
        let _ = tx_album_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let tx_track_enqueue = tx.clone();
    album_grid.connect_track_enqueue(move |tracks| {
        let _ = tx_track_enqueue.send_blocking(WindowMessage::AppendToQueue(tracks));
    });

    let db_album_edit = Rc::clone(db);
    let edit_requester_album = Rc::clone(edit_requester);
    album_grid.connect_album_edit_requested(move |tracks| {
        let art = tracks.first().and_then(ArtKey::for_track).and_then(|key| {
            db_album_edit.art_for(&key).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        edit_requester_album.edit_tracks(tracks, art);
    });

    let db_track_edit = Rc::clone(db);
    let edit_requester_track = Rc::clone(edit_requester);
    album_grid.connect_track_edit_requested(move |tracks| {
        let art = tracks.first().and_then(|t| {
            db_track_edit.art_for_track(t.id).unwrap_or_else(|e| {
                tracing::error!("Art lookup failed: {e}");
                None
            })
        });
        edit_requester_track.edit_row(tracks, art);
    });
}
